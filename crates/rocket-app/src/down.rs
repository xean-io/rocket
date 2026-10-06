//! `down`: stopping services in reverse dependency order (Go: `down.go`).

use crate::app::{App, key};
use crate::error::{AppError, Result};
use rocket_domain::api::{DownRequest, DownResult};
use rocket_domain::ports::ComposeTarget;
use rocket_domain::{Health, Project, Run, RunState, ServiceKind, merge_profiles};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

impl App {
    /// Stops services in reverse dependency order. Whole-project and
    /// everywhere requests also cancel running jobs (R6) and take Compose
    /// projects down.
    pub async fn down(&self, req: DownRequest) -> Result<DownResult> {
        let mut res = DownResult::default();
        let mut projects: HashMap<String, Project> = HashMap::new();
        let mut targets: Option<HashSet<String>> = None;
        let mut job_project = String::new();
        if req.everywhere {
            if !req.services.is_empty() {
                return Err(AppError::invalid(
                    "services cannot be combined with --everywhere",
                ));
            }
        } else {
            let p = self.resolve_project(&req.project)?;
            job_project.clone_from(&p.name);
            if !req.services.is_empty() {
                let names = p
                    .expand_targets(&req.services)
                    .map_err(|e| AppError::invalid(e.to_string()))?;
                targets = Some(names.into_iter().collect());
            }
            projects.insert(p.name.clone(), p);
        }
        // Cancel prerequisite startups before taking the service snapshot or
        // waiting for their project gate. Their partial startups must unwind
        // first.
        if req.services.is_empty() {
            res.canceled_jobs = self.cancel_jobs(&job_project, &req.owner).await;
        }
        let whole_project = req.services.is_empty() && req.owner.is_empty();

        let mut by_project: BTreeMap<String, Vec<Run>> = BTreeMap::new();
        for r in self.d().store.list_runs()? {
            let leftover_compose = whole_project
                && r.state == RunState::Failed
                && r.kind == ServiceKind::Compose
                && !r.compose_project.is_empty();
            if !r.state.active() && !leftover_compose {
                continue;
            }
            if !req.everywhere && !projects.contains_key(&r.project) {
                continue;
            }
            if targets.as_ref().is_some_and(|t| !t.contains(&r.service)) {
                continue;
            }
            if !req.owner.is_empty() && r.owner != req.owner {
                continue;
            }
            by_project.entry(r.project.clone()).or_default().push(r);
        }

        for (name, runs) in by_project {
            let p = projects
                .get(&name)
                .cloned()
                .or_else(|| self.resolve_project(&name).ok());
            let (stopped, compose_down, errs) = self
                .stop_project_runs(p.as_ref(), runs, whole_project)
                .await;
            res.stopped.extend(stopped);
            res.compose_down.extend(compose_down);
            res.errors.extend(errs);
        }
        Ok(res)
    }

    /// Stops `runs` of one project under its gate. Returns the stopped runs,
    /// the Compose projects taken down and the error messages. `p` is `None`
    /// for projects that are no longer registered.
    pub(crate) async fn stop_project_runs(
        &self,
        p: Option<&Project>,
        runs: Vec<Run>,
        compose_down: bool,
    ) -> (Vec<Run>, Vec<String>, Vec<String>) {
        let Some(first) = runs.first() else {
            return (Vec::new(), Vec::new(), Vec::new());
        };
        let _gate = self.lock_project(&first.project).await;

        let mut by_service: HashMap<String, Run> = HashMap::new();
        let mut services = Vec::new();
        for r in runs {
            services.push(r.service.clone());
            by_service.insert(r.service.clone(), r);
        }
        let services = match p {
            Some(p) => p.stop_order(&services),
            None => {
                services.sort();
                services
            }
        };
        let mut stopped = Vec::new();
        let mut errs = Vec::new();
        let mut compose_targets: BTreeMap<String, ComposeTarget> = BTreeMap::new();
        for s in services {
            let Some(r) = by_service.remove(&s) else {
                continue;
            };
            let kind = r.kind;
            let (project, service) = (r.project.clone(), r.service.clone());
            let (out, mut target, err) = self.stop_run(p, r).await;
            if let Some(err) = err {
                errs.push(format!("{project}/{service}: {err}"));
            }
            if kind == ServiceKind::Compose {
                if let Some(existing) = compose_targets.get(&target.project_name) {
                    target.profiles = merge_profiles(&[
                        existing.profiles.clone(),
                        std::mem::take(&mut target.profiles),
                    ]);
                }
                compose_targets.insert(target.project_name.clone(), target);
            }
            stopped.push(out);
        }
        let mut downed = Vec::new();
        if compose_down {
            for (name, target) in &compose_targets {
                match self.d().compose.down(target, &mut std::io::sink()).await {
                    Ok(()) => downed.push(name.clone()),
                    Err(err) => errs.push(format!("compose down {name}: {err}")),
                }
            }
        }
        (stopped, downed, errs)
    }

    /// Stops one run and releases its leases. The caller holds the project
    /// gate. Returns the stopped run, its Compose target (default for local
    /// processes) and the stop error, if any.
    async fn stop_run(
        &self,
        p: Option<&Project>,
        mut r: Run,
    ) -> (Run, ComposeTarget, Option<String>) {
        {
            let _g = self.lock();
            r.state = RunState::Stopping;
            let _ = self.save_run(&r);
        }

        let mut target = ComposeTarget::default();
        let stop_err = match r.kind {
            ServiceKind::Compose => {
                target = self.compose_target_for_run(p, &r);
                let svc_name = p
                    .and_then(|p| p.services.get(&r.service))
                    .map_or_else(|| r.service.clone(), |svc| svc.compose.clone());
                let log = self
                    .d()
                    .logs
                    .open(&r.project, &r.service)
                    .ok()
                    .map(|(f, _)| f);
                let mut sink = std::io::sink();
                let mut file_out;
                let out: &mut (dyn Write + Send) = match &log {
                    Some(f) => {
                        file_out = f;
                        &mut file_out
                    }
                    None => &mut sink,
                };
                self.d().compose.stop(&target, &svc_name, out).await.err()
            }
            ServiceKind::Run | ServiceKind::Task => {
                if r.pgid > 0 {
                    self.d()
                        .runner
                        .stop(r.pgid, self.cfg().stop_grace)
                        .await
                        .err()
                } else {
                    None
                }
            }
        };

        let mut st = self.lock();
        if let Ok(Some(cur)) = self.d().store.get_run(&r.project, &r.service)
            && cur.pid == r.pid
        {
            r.exit_code = cur.exit_code;
        }
        r.state = RunState::Stopped;
        r.health = Health::Unknown;
        r.stopped_at = Some(self.now());
        let stop_err = stop_err.map(|e| e.to_string());
        if let Some(e) = &stop_err {
            r.error = format!("stop: {e}");
        }
        let _ = self.save_run(&r);
        st.watches.remove(&key(&r.project, &r.service));
        self.release_leases(&r.project, &r.service);
        (r, target, stop_err)
    }

    /// The Compose target to stop a run with: the launch-time profile
    /// selection stored on the run, or (for runs predating it) the one
    /// reconstructed from the current manifest.
    pub(crate) fn compose_target_for_run(&self, p: Option<&Project>, r: &Run) -> ComposeTarget {
        if let Some(p) = p
            && let Some(svc) = p.services.get(&r.service)
            && p.envs.contains_key(&r.env)
        {
            let env = self
                .service_env(p, &r.env, svc, &r.ports)
                .unwrap_or_else(|_| self.base_env());
            let mut target = self.compose_target(p, &r.env, svc, env, &[]);
            if !r.profiles.is_empty() {
                target.profiles = merge_profiles(std::slice::from_ref(&r.profiles));
            }
            return target;
        }
        ComposeTarget {
            project_name: r.compose_project.clone(),
            env: self.base_env(),
            profiles: merge_profiles(std::slice::from_ref(&r.profiles)),
            ..ComposeTarget::default()
        }
    }
}
