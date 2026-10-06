//! `up` and `restart`: dependency-ordered startup with port leasing,
//! `{service.port}` expansion, health probing and cancellation (Go: `up.go`).

use crate::app::App;
use crate::error::{AppError, Result};
use crate::go_duration::{format_duration, parse_duration};
use crate::paths;
use crate::volume_hints::compose_volume_hints;
use crate::watch::Watch;
use rocket_domain::api::{DownRequest, ServiceResult, UpRequest, UpResult, service_action};
use rocket_domain::ports::{ComposeTarget, ProcessSpec};
use rocket_domain::{
    DEFAULT_OWNER, Health, Lease, PortHolder, PortRemap, Project, Run, RunState, Service,
    ServiceKind, build_env, compose_project_name, event_type, health_check_for, merge_profiles,
    port_references, remap_candidates,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::time::Duration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio_util::sync::CancellationToken;

/// Go's `context.Canceled` message, used where errors travel as text.
const CANCELED: &str = "context canceled";

/// Everything `start_service` needs about the request.
struct StartCtx<'a> {
    project: &'a Project,
    env_name: &'a str,
    profiles: &'a [String],
    owner: &'a str,
    expires: Option<OffsetDateTime>,
    cancel: &'a CancellationToken,
}

/// Why a port is unavailable.
struct Busy {
    reason: String,
    holder: Option<PortHolder>,
}

/// Outcome of leasing every port of a service.
struct Leased {
    ports: BTreeMap<String, u16>,
    remaps: Vec<PortRemap>,
    error: Option<String>,
}

/// Runs `fut` unless `cancel` fires first (Go: a call with a canceled ctx).
pub(crate) async fn cancelable<T>(
    cancel: &CancellationToken,
    fut: impl Future<Output = T>,
) -> std::result::Result<T, AppError> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(AppError::Canceled),
        v = fut => Ok(v),
    }
}

impl App {
    /// Starts the requested services in dependency order. Idempotent:
    /// services that are already running are reported, not restarted.
    pub async fn up(&self, req: UpRequest) -> Result<UpResult> {
        self.up_with(req, &CancellationToken::new(), None).await
    }

    /// [`up`](Self::up) with cancellation and an absolute TTL `deadline` for
    /// pipeline prerequisites, so time spent waiting for earlier services
    /// does not extend later services' TTLs. Canceling stops (and fails) only
    /// the service being started; services already started stay up.
    pub async fn up_with(
        &self,
        req: UpRequest,
        cancel: &CancellationToken,
        deadline: Option<OffsetDateTime>,
    ) -> Result<UpResult> {
        if cancel.is_cancelled() {
            return Err(AppError::Canceled);
        }
        let p = self.resolve_project(&req.project)?;
        let (env_name, targets) = startup_targets(&p, &req)?;
        let owner = if req.owner.is_empty() {
            DEFAULT_OWNER.to_string()
        } else {
            req.owner.clone()
        };
        let mut expires = deadline;
        if let Some(ttl) = parse_service_ttl(&req.ttl)? {
            expires = Some(self.now() + ttl);
        }
        let order = p
            .start_order(&targets)
            .map_err(|e| AppError::invalid(e.to_string()))?;

        let _gate = self.lock_project_cancellable(&p.name, cancel).await?;
        // A project has one live run per service. Check the complete
        // dependency closure before starting anything so no stale
        // environment is reused.
        for name in &order {
            let svc = p.services.get(name).cloned().unwrap_or_default();
            if let Some(run) = self.d().store.get_run(&p.name, name).ok().flatten()
                && run.env != env_name
                && self.is_service_live(&run, &svc).await
            {
                return Err(AppError::invalid(format!(
                    "service {name:?} is already running in env {:?}, requested {env_name:?}; stop it or restart with --env {env_name}",
                    run.env
                )));
            }
        }

        let mut res = UpResult {
            project: p.name.clone(),
            env: env_name.clone(),
            services: Vec::new(),
            hints: Vec::new(),
        };
        let mut created_volumes = BTreeSet::new();
        let mut failed: HashSet<String> = HashSet::new();
        let cx = StartCtx {
            project: &p,
            env_name: &env_name,
            profiles: &req.profiles,
            owner: &owner,
            expires,
            cancel,
        };
        for name in &order {
            if cancel.is_cancelled() {
                return Err(AppError::Canceled);
            }
            let svc = p.services.get(name).cloned().unwrap_or_default();
            if let Some(dep) = svc.depends_on.iter().find(|d| failed.contains(*d)) {
                failed.insert(name.clone());
                res.services.push(ServiceResult {
                    error: format!("dependency {dep:?} failed"),
                    ..blank_result(name, service_action::SKIPPED, RunState::Stopped)
                });
                continue;
            }
            if let Some(run) = self.d().store.get_run(&p.name, name).ok().flatten()
                && self.is_service_live(&run, &svc).await
            {
                res.services.push(result_from_run(
                    &run,
                    service_action::ALREADY_RUNNING,
                    Vec::new(),
                ));
                continue;
            }
            let sr = self.start_service(&cx, svc, &mut created_volumes).await;
            res.hints = compose_volume_hints(&p, &env_name, &created_volumes);
            if sr.action == service_action::FAILED {
                failed.insert(name.clone());
            }
            res.services.push(sr);
        }
        Ok(res)
    }

    /// Stops then starts the requested services.
    pub async fn restart(&self, mut req: UpRequest) -> Result<UpResult> {
        let p = self.resolve_project(&req.project)?;
        let (env_name, targets) = startup_targets(&p, &req)?;
        parse_service_ttl(&req.ttl)?;
        req.env = env_name;
        if targets.is_empty() {
            return self.up(req).await;
        }
        req.services.clone_from(&targets);
        self.down(DownRequest {
            project: req.project.clone(),
            services: targets,
            ..DownRequest::default()
        })
        .await?;
        self.up(req).await
    }

    // --- liveness -----------------------------------------------------------

    /// Whether an active run still has its process/container.
    pub(crate) async fn is_live(&self, r: &Run) -> bool {
        if !r.state.active() {
            return false;
        }
        if r.kind == ServiceKind::Compose {
            let mut name = r.service.clone();
            if let Ok(p) = self.resolve_project(&r.project)
                && let Some(svc) = p.services.get(&r.service)
                && !svc.compose.is_empty()
            {
                name.clone_from(&svc.compose);
            }
            return matches!(
                self.d().compose.running(&r.compose_project, &name).await,
                Ok(true)
            );
        }
        r.pid > 0 && self.d().runner.alive(r.pid, r.pgid)
    }

    /// [`is_live`](Self::is_live) using the loaded Compose service name,
    /// which may differ from rocket's service name. The run identity itself
    /// is left unchanged.
    pub(crate) async fn is_service_live(&self, r: &Run, svc: &Service) -> bool {
        if r.kind == ServiceKind::Compose && !svc.compose.is_empty() {
            if !r.state.active() {
                return false;
            }
            return matches!(
                self.d()
                    .compose
                    .running(&r.compose_project, &svc.compose)
                    .await,
                Ok(true)
            );
        }
        self.is_live(r).await
    }

    // --- starting one service -------------------------------------------------

    async fn start_service(
        &self,
        cx: &StartCtx<'_>,
        mut svc: Service,
        created_volumes: &mut BTreeSet<String>,
    ) -> ServiceResult {
        let (p, cancel) = (cx.project, cx.cancel);
        let now = self.now();
        let mut run = Run {
            project: p.name.clone(),
            service: svc.name.clone(),
            env: cx.env_name.to_string(),
            kind: svc.kind,
            state: RunState::Starting,
            health: Health::Unknown,
            pid: 0,
            pgid: 0,
            compose_project: String::new(),
            profiles: Vec::new(),
            container_id: String::new(),
            owner: cx.owner.to_string(),
            expires_at: cx.expires,
            started_at: Some(now),
            stopped_at: None,
            exit_code: None,
            ports: BTreeMap::new(),
            log_path: String::new(),
            error: String::new(),
        };
        if cancel.is_cancelled() {
            return self.fail_run(run, Vec::new(), CANCELED.into());
        }

        let Leased {
            ports,
            remaps,
            error,
        } = self.lease_ports(&p.name, &svc);
        run.ports.clone_from(&ports);
        if let Some(err) = error {
            return self.fail_run(run, remaps, err);
        }
        svc.env = match self.resolve_port_references(p, cx.env_name, &svc.env).await {
            Ok(env) => env,
            Err(err) => return self.fail_run(run, remaps, err),
        };
        let env = match self.service_env(p, cx.env_name, &svc, &ports) {
            Ok(env) => env,
            Err(err) => return self.fail_run(run, remaps, err),
        };
        let (logf, log_path) = match self.d().logs.open(&p.name, &svc.name) {
            Ok(opened) => opened,
            Err(err) => return self.fail_run(run, remaps, format!("open log: {err}")),
        };
        run.log_path = log_path;
        let started = now.replace_nanosecond(0).unwrap_or(now);
        let _ = writeln!(
            &logf,
            "=== rocket: starting {} ({}, env {}, owner {}) at {} ===",
            svc.name,
            svc.kind,
            cx.env_name,
            cx.owner,
            started.format(&Rfc3339).unwrap_or_default()
        );

        let mut watch: Option<Watch> = None;
        let mut attempted_compose: Option<ComposeTarget> = None;
        if cancel.is_cancelled() {
            return self.fail_run(run, remaps, CANCELED.into());
        }
        match svc.kind {
            ServiceKind::Compose => {
                let target = self.compose_target(p, cx.env_name, &svc, env, cx.profiles);
                run.compose_project.clone_from(&target.project_name);
                run.profiles.clone_from(&target.profiles);
                {
                    let _g = self.lock();
                    let _ = self.save_run(&run);
                }
                let missing = self
                    .missing_compose_volumes(&target, &svc.compose, cancel)
                    .await;
                if cancel.is_cancelled() {
                    return self.fail_run(run, remaps, CANCELED.into());
                }
                attempted_compose = Some(target.clone());
                let mut out = &logf;
                let up =
                    cancelable(cancel, self.d().compose.up(&target, &svc.compose, &mut out)).await;
                self.confirm_compose_volumes(&target, &missing, created_volumes, cancel)
                    .await;
                let up = match up {
                    Ok(Ok(cid)) => Ok(cid),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(err) => Err(err.to_string()),
                };
                let cid = match up {
                    Ok(cid) => cid,
                    Err(err) => {
                        let msg = format!("docker compose up {}: {err}", svc.compose);
                        let msg = self
                            .stop_canceled_compose(
                                attempted_compose.as_ref(),
                                cancel,
                                &svc.compose,
                                &logf,
                                msg,
                            )
                            .await;
                        return self.fail_run(run, remaps, msg);
                    }
                };
                run.container_id = cid;
            }
            ServiceKind::Run | ServiceKind::Task => {
                let argv = if svc.kind == ServiceKind::Task {
                    self.d().tasks.argv(&svc.task, &[])
                } else {
                    vec!["/bin/sh".into(), "-c".into(), svc.run.clone()]
                };
                let output = match logf.try_clone() {
                    Ok(f) => f,
                    Err(err) => {
                        return self.fail_run(run, remaps, format!("start {}: {err}", svc.name));
                    }
                };
                let spec = ProcessSpec {
                    argv,
                    dir: paths::join(&p.root, &svc.cwd),
                    env,
                    output: Some(output),
                };
                let h = match self.d().runner.start(spec) {
                    Ok(h) => h,
                    Err(err) => {
                        return self.fail_run(run, remaps, format!("start {}: {err}", svc.name));
                    }
                };
                (run.pid, run.pgid) = (h.pid, h.pgid);
                let mut st = self.lock();
                let _ = self.save_run(&run);
                watch = Some(self.watch_process(&mut st, &p.name, &svc.name, h));
            }
        }

        if let Err(msg) = self
            .wait_healthy(&svc, &ports, watch.as_ref(), cancel)
            .await
        {
            if watch.is_some() {
                let _ = self.d().runner.stop(run.pgid, self.cfg().stop_grace).await;
            }
            let msg = self
                .stop_canceled_compose(attempted_compose.as_ref(), cancel, &svc.compose, &logf, msg)
                .await;
            return self.fail_run(run, remaps, msg);
        }
        if cancel.is_cancelled() {
            if watch.is_some() {
                let _ = self.d().runner.stop(run.pgid, self.cfg().stop_grace).await;
            }
            let msg = self
                .stop_canceled_compose(
                    attempted_compose.as_ref(),
                    cancel,
                    &svc.compose,
                    &logf,
                    CANCELED.into(),
                )
                .await;
            return self.fail_run(run, remaps, msg);
        }

        let _g = self.lock();
        if let Some(code) = watch.as_ref().and_then(Watch::code) {
            run.exit_code = Some(code);
            self.mark_failed(
                &mut run,
                format!("process exited with code {code} right after start"),
            );
            let _ = self.save_run(&run);
            self.release_leases(&p.name, &svc.name);
            return result_from_run(&run, service_action::FAILED, remaps);
        }
        run.state = RunState::Running;
        run.health = Health::Healthy;
        let _ = self.save_run(&run);
        result_from_run(&run, service_action::STARTED, remaps)
    }

    fn mark_failed(&self, run: &mut Run, message: String) {
        run.state = RunState::Failed;
        run.health = Health::Unhealthy;
        run.error = message;
        run.stopped_at = Some(self.now());
    }

    /// Records a failed start and releases the service's leases.
    fn fail_run(&self, mut run: Run, remaps: Vec<PortRemap>, message: String) -> ServiceResult {
        self.mark_failed(&mut run, message);
        {
            let _g = self.lock();
            let _ = self.save_run(&run);
        }
        self.release_leases(&run.project, &run.service);
        result_from_run(&run, service_action::FAILED, remaps)
    }

    /// When startup was canceled after Compose was invoked, stops that one
    /// service (with a detached deadline) and folds a stop failure into the
    /// message.
    async fn stop_canceled_compose(
        &self,
        attempted: Option<&ComposeTarget>,
        cancel: &CancellationToken,
        compose_service: &str,
        logf: &File,
        err: String,
    ) -> String {
        let Some(target) = attempted.filter(|_| cancel.is_cancelled()) else {
            return err;
        };
        let mut out = logf;
        let grace = self.cfg().stop_grace + Duration::from_secs(5);
        match tokio::time::timeout(
            grace,
            self.d().compose.stop(target, compose_service, &mut out),
        )
        .await
        {
            Ok(Ok(())) => err,
            Ok(Err(stop_err)) => format!("{err}; stop canceled compose service: {stop_err}"),
            Err(_) => format!("{err}; stop canceled compose service: context deadline exceeded"),
        }
    }

    // --- environment ------------------------------------------------------------

    /// Expands `{service.port}` tokens against live providers of the same
    /// environment. The manifest is not modified.
    async fn resolve_port_references(
        &self,
        p: &Project,
        env_name: &str,
        source: &BTreeMap<String, String>,
    ) -> std::result::Result<BTreeMap<String, String>, String> {
        let mut resolved = BTreeMap::new();
        for (variable, value) in source {
            let mut value = value.clone();
            for r in port_references(&value) {
                let fail =
                    |why: &str| format!("resolve env {variable} reference {}: {why}", r.token);
                let run = self
                    .d()
                    .store
                    .get_run(&p.name, &r.service)
                    .map_err(|e| fail(&e.to_string()))?;
                let provider = p.services.get(&r.service).cloned().unwrap_or_default();
                let run = match run {
                    Some(run)
                        if run.state == RunState::Running
                            && self.is_service_live(&run, &provider).await =>
                    {
                        run
                    }
                    _ => return Err(fail("provider is not running")),
                };
                if run.env != env_name {
                    return Err(fail(&format!(
                        "provider is running in env {:?}, requested {env_name:?}",
                        run.env
                    )));
                }
                let port = run.ports.get(&r.port).copied().unwrap_or(0);
                if port == 0 {
                    return Err(fail("provider has no resolved port"));
                }
                value = r.replace(&value, &port.to_string());
            }
            resolved.insert(variable.clone(), value);
        }
        Ok(resolved)
    }

    /// The child environment: base < dotenv < service env, then the port
    /// variables, then the forced Compose project identity.
    pub(crate) fn service_env(
        &self,
        p: &Project,
        env_name: &str,
        svc: &Service,
        ports: &BTreeMap<String, u16>,
    ) -> std::result::Result<Vec<String>, String> {
        let files: Vec<String> = p.dotenv.iter().chain(&svc.dotenv).cloned().collect();
        let dotenv = self
            .d()
            .env
            .dotenv(std::path::Path::new(&p.root), &files)
            .map_err(|e| format!("read dotenv: {e}"))?;
        let merged = build_env(&self.base_env(), &[&dotenv, &svc.env]);
        let values: BTreeMap<String, String> = merged
            .iter()
            .map(|item| {
                let (k, v) = item.split_once('=').unwrap_or((item, ""));
                (k.to_string(), v.to_string())
            })
            .collect();
        let mut port_vars = BTreeMap::new();
        for spec in &svc.ports {
            let port = ports.get(&spec.name).copied().unwrap_or(0).to_string();
            if !spec.env.is_empty() && spec.env_bindings.is_empty() {
                port_vars.insert(spec.env.clone(), port.clone());
            }
            for (variable, binding) in &spec.env_bindings {
                if !binding.default || values.get(variable).is_none_or(String::is_empty) {
                    port_vars.insert(variable.clone(), binding.template.replace("{port}", &port));
                }
            }
        }
        let identity = BTreeMap::from([(
            "COMPOSE_PROJECT_NAME".to_string(),
            compose_project_name(&p.name, env_name),
        )]);
        Ok(build_env(&[], &[&values, &port_vars, &identity]))
    }

    pub(crate) fn compose_target(
        &self,
        p: &Project,
        env_name: &str,
        svc: &Service,
        env: Vec<String>,
        profiles: &[String],
    ) -> ComposeTarget {
        let e = p.envs.get(env_name).cloned().unwrap_or_default();
        let files = e
            .compose
            .iter()
            .map(|f| {
                if std::path::Path::new(f).is_absolute() {
                    f.clone()
                } else {
                    paths::join(&p.root, f).to_string_lossy().into_owned()
                }
            })
            .collect();
        ComposeTarget {
            project_name: compose_project_name(&p.name, env_name),
            dir: p.root.clone().into(),
            files,
            profiles: merge_profiles(&[e.profiles, profiles.to_vec(), svc.profiles.clone()]),
            env,
        }
    }

    // --- ports ----------------------------------------------------------------------

    /// Reserves every port of `svc`, remapping busy ones when the port
    /// declares an env var.
    fn lease_ports(&self, project: &str, svc: &Service) -> Leased {
        let mut out = Leased {
            ports: BTreeMap::new(),
            remaps: Vec::new(),
            error: None,
        };
        for spec in &svc.ports {
            let busy = self.port_busy(spec.default, project, &svc.name);
            let (reason, holder) = match busy {
                None => {
                    if self
                        .lease(spec.default, project, &svc.name, &spec.name)
                        .is_ok()
                    {
                        out.ports.insert(spec.name.clone(), spec.default);
                        continue;
                    }
                    ("leased concurrently".to_string(), None)
                }
                Some(Busy { reason, holder }) => (reason, holder),
            };
            if spec.env.is_empty() {
                out.error = Some(format!(
                    "port {} ({}) for {project}/{} is busy: {reason}; it declares no env var so rocket cannot remap it \u{2014} free the port or add `env:` to the port in rocket.yaml",
                    spec.default, spec.name, svc.name
                ));
                return out;
            }
            let chosen = remap_candidates(spec.default).into_iter().find(|&c| {
                self.port_busy(c, project, &svc.name).is_none()
                    && self.lease(c, project, &svc.name, &spec.name).is_ok()
            });
            let Some(chosen) = chosen else {
                out.error = Some(format!(
                    "port {} ({}) for {project}/{} is busy ({reason}) and no free port was found to remap",
                    spec.default, spec.name, svc.name
                ));
                return out;
            };
            out.ports.insert(spec.name.clone(), chosen);
            out.remaps.push(PortRemap {
                name: spec.name.clone(),
                from: spec.default,
                to: chosen,
                env: spec.env.clone(),
                holder,
                reason,
            });
        }
        out
    }

    fn lease(
        &self,
        port: u16,
        project: &str,
        service: &str,
        name: &str,
    ) -> rocket_domain::ports::Result<()> {
        let l = Lease {
            port,
            project: project.into(),
            service: service.into(),
            port_name: name.into(),
            created_at: self.now(),
        };
        self.d().store.acquire_lease(&l)?;
        self.publish_lease(event_type::PORT_LEASED, &l);
        Ok(())
    }

    /// Why `port` is unavailable to `project/service`, or `None` when free.
    /// Leases left by runs that are no longer active are released.
    fn port_busy(&self, port: u16, project: &str, service: &str) -> Option<Busy> {
        for l in self.d().store.list_leases().unwrap_or_default() {
            if l.port != port || (l.project == project && l.service == service) {
                continue;
            }
            if let Ok(Some(run)) = self.d().store.get_run(&l.project, &l.service)
                && run.state.active()
            {
                return Some(Busy {
                    reason: format!("leased by {}/{}", l.project, l.service),
                    holder: None,
                });
            }
            // stale lease left by a run that is no longer active
            self.release_leases(&l.project, &l.service);
        }
        if self.d().probe.free(port) {
            return None;
        }
        Some(match self.d().probe.holder(port).ok().flatten() {
            Some(holder) => {
                let mut reason = format!("in use by {} (pid {})", holder.command, holder.pid);
                if !holder.cwd.is_empty() {
                    reason.push_str(" in ");
                    reason.push_str(&holder.cwd);
                }
                Busy {
                    reason,
                    holder: Some(holder),
                }
            }
            None => Busy {
                reason: "in use by an unknown process".into(),
                holder: None,
            },
        })
    }

    // --- readiness --------------------------------------------------------------------

    async fn wait_healthy(
        &self,
        svc: &Service,
        ports: &BTreeMap<String, u16>,
        watch: Option<&Watch>,
        cancel: &CancellationToken,
    ) -> std::result::Result<(), String> {
        let Some(check) = health_check_for(svc, ports) else {
            let Some(w) = watch else { return Ok(()) };
            return tokio::select! {
                () = w.wait_exit() => Err(format!(
                    "process exited with code {} right after start",
                    w.code().unwrap_or(-1)
                )),
                () = tokio::time::sleep(self.cfg().start_grace) => Ok(()),
                () = cancel.cancelled() => Err(CANCELED.into()),
            };
        };
        let exit_wait = async {
            match watch {
                Some(w) => w.wait_exit().await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(exit_wait);
        let timeout = svc.health_timeout();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(w) = watch.filter(|w| w.is_exited()) {
                return Err(format!(
                    "process exited with code {} before becoming healthy",
                    w.code().unwrap_or(-1)
                ));
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let probe_timeout = remaining.min(Duration::from_secs(2));
            let last_err = tokio::select! {
                biased;
                () = cancel.cancelled() => CANCELED.to_string(),
                r = tokio::time::timeout(probe_timeout, self.d().health.check(&check)) => match r {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => "context deadline exceeded".to_string(),
                },
            };
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(CANCELED.into()),
                () = tokio::time::sleep_until(deadline) => {
                    return Err(format!(
                        "not healthy after {} ({} probe on port {}): {last_err}",
                        format_duration(timeout), check.kind, check.port
                    ));
                }
                () = &mut exit_wait => {}
                () = tokio::time::sleep(self.cfg().health_interval) => {}
            }
        }
    }
}

/// `ServiceResult` with only identity and outcome set.
pub(crate) fn blank_result(service: &str, action: &str, state: RunState) -> ServiceResult {
    ServiceResult {
        service: service.into(),
        action: action.into(),
        state,
        health: None,
        pid: 0,
        ports: BTreeMap::new(),
        remaps: Vec::new(),
        owner: String::new(),
        expires_at: None,
        log_path: String::new(),
        error: String::new(),
    }
}

pub(crate) fn result_from_run(r: &Run, action: &str, remaps: Vec<PortRemap>) -> ServiceResult {
    ServiceResult {
        service: r.service.clone(),
        action: action.into(),
        state: r.state,
        health: Some(r.health),
        pid: r.pid,
        ports: r.ports.clone(),
        remaps,
        owner: r.owner.clone(),
        expires_at: r.expires_at,
        log_path: r.log_path.clone(),
        error: r.error.clone(),
    }
}

/// Parses a Go duration TTL; `None` when empty.
pub(crate) fn parse_service_ttl(value: &str) -> Result<Option<Duration>> {
    if value.is_empty() {
        return Ok(None);
    }
    match parse_duration(value).and_then(|ns| u64::try_from(ns).ok()) {
        Some(ns) if ns > 0 => Ok(Some(Duration::from_nanos(ns))),
        _ => Err(AppError::invalid(format!(
            "invalid ttl {value:?} (use a Go duration like 30m)"
        ))),
    }
}

/// The selected env name and the expanded startup targets of a request.
pub(crate) fn startup_targets(p: &Project, req: &UpRequest) -> Result<(String, Vec<String>)> {
    let env_name = if req.env.is_empty() {
        p.default_env.clone()
    } else {
        req.env.clone()
    };
    let Some(env) = p.envs.get(&env_name) else {
        return Err(AppError::invalid(format!(
            "unknown env {env_name:?} in project {}",
            p.name
        )));
    };
    let profiles = merge_profiles(&[env.profiles.clone(), req.profiles.clone()]);
    let targets = p
        .expand_startup_targets(&req.services, &profiles)
        .map_err(|e| AppError::invalid(e.to_string()))?;
    Ok((env_name, targets))
}
