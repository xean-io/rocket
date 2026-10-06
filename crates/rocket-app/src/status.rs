//! `ps`, `logs` and the port map (Go: `status.go`).

use crate::app::{App, key};
use crate::error::{AppError, Result};
use rocket_domain::api::{LogsResult, PortInfo, PortsResult, StatusResult};
use rocket_domain::{Health, Run, RunState, ServiceKind, merge_profiles};
use serde::{Deserialize, Serialize};

/// Selects the services shown by `rocket ps`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_projects: bool,
}

/// Asks for the tail of a service log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogsRequest {
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub tail: i64,
}

impl App {
    /// Current service states. Declared-but-never-started services appear as
    /// `stopped` when a single project is requested.
    pub fn status(&self, req: &StatusRequest) -> Result<StatusResult> {
        self.refresh_liveness();
        let runs = self.d().store.list_runs()?;
        let mut out: Vec<Run> = Vec::new();
        if req.all_projects || req.project.is_empty() {
            out = runs;
        } else {
            let p = self.resolve_project(&req.project)?;
            let mut have = std::collections::HashSet::new();
            for r in runs.into_iter().filter(|r| r.project == p.name) {
                have.insert(r.service.clone());
                out.push(r);
            }
            for name in p.service_names() {
                if have.contains(&name) {
                    continue;
                }
                out.push(Run {
                    project: p.name.clone(),
                    service: name.clone(),
                    env: p.default_env.clone(),
                    kind: p.services[&name].kind,
                    state: RunState::Stopped,
                    health: Health::Unknown,
                    pid: 0,
                    pgid: 0,
                    compose_project: String::new(),
                    profiles: Vec::new(),
                    container_id: String::new(),
                    owner: String::new(),
                    expires_at: None,
                    started_at: None,
                    stopped_at: None,
                    exit_code: None,
                    ports: Default::default(),
                    log_path: String::new(),
                    error: String::new(),
                });
            }
        }
        out.sort_by(|a, b| (&a.project, &a.service).cmp(&(&b.project, &b.service)));
        Ok(StatusResult { services: out })
    }

    /// Marks unsupervised local processes that vanished as dead.
    fn refresh_liveness(&self) {
        let st = self.lock();
        for r in self.d().store.list_runs().unwrap_or_default() {
            if !r.state.active() || r.kind == ServiceKind::Compose || r.state == RunState::Starting
            {
                continue;
            }
            if st.watches.contains_key(&key(&r.project, &r.service)) {
                continue;
            }
            if r.pid > 0 && self.d().runner.alive(r.pid, r.pgid) {
                continue;
            }
            self.mark_dead_locked(r);
        }
    }

    /// Records a vanished run as dead and releases its leases. The caller
    /// holds the state lock.
    pub(crate) fn mark_dead_locked(&self, mut r: Run) -> Run {
        r.state = RunState::Dead;
        r.health = Health::Unknown;
        r.stopped_at = Some(self.now());
        let _ = self.save_run(&r);
        self.release_leases(&r.project, &r.service);
        r
    }

    /// The last lines of a service's output (oldest first).
    pub async fn logs(&self, req: LogsRequest) -> Result<LogsResult> {
        let p = self.resolve_project(&req.project)?;
        let Some(svc) = p.services.get(&req.service) else {
            return Err(AppError::not_found(format!(
                "unknown service {:?} in project {}",
                req.service, p.name
            )));
        };
        let tail = if req.tail <= 0 {
            100
        } else {
            usize::try_from(req.tail).unwrap_or(usize::MAX)
        };
        let mut res = LogsResult {
            project: p.name.clone(),
            service: svc.name.clone(),
            lines: Vec::new(),
        };
        if svc.kind == ServiceKind::Compose {
            let mut env = p.default_env.clone();
            let mut run_profiles = Vec::new();
            if let Ok(Some(r)) = self.d().store.get_run(&p.name, &svc.name)
                && !r.env.is_empty()
            {
                env = r.env;
                run_profiles = r.profiles;
            }
            let mut target = self.compose_target(&p, &env, svc, self.base_env(), &[]);
            if !run_profiles.is_empty() {
                target.profiles = merge_profiles(&[run_profiles]);
            }
            if let Ok(lines) = self.d().compose.logs(&target, &svc.compose, tail).await {
                res.lines.extend(lines);
                return Ok(res);
            }
        }
        // No log yet is not an error.
        if let Ok(lines) = self.d().logs.tail(&p.name, &svc.name, tail) {
            res.lines.extend(lines);
        }
        Ok(res)
    }

    /// Every leased port with its owning run, by port number.
    pub fn ports(&self) -> Result<PortsResult> {
        let mut out = Vec::new();
        for lease in self.d().store.list_leases()? {
            let mut info = PortInfo {
                lease,
                owner: String::new(),
                state: None,
                pid: 0,
                env: String::new(),
            };
            if let Ok(Some(r)) = self
                .d()
                .store
                .get_run(&info.lease.project, &info.lease.service)
            {
                (info.owner, info.state, info.pid, info.env) =
                    (r.owner, Some(r.state), r.pid, r.env);
            }
            out.push(info);
        }
        out.sort_by_key(|i| i.lease.port);
        Ok(PortsResult { ports: out })
    }
}
