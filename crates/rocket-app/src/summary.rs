//! The one-shot overview behind `rocket status` (Go: `summary.go`).

use crate::app::App;
use crate::error::Result;
use crate::status::StatusRequest;
use rocket_domain::api::{Conflict, ProjectInfo, Summary, conflict_kind};
use rocket_domain::{JobStatus, PortHolder, Project};

impl App {
    /// Services, running jobs and port conflicts of one project, or of every
    /// project when `project` is empty (conflicts need a project).
    pub fn summary(&self, project: &str) -> Result<Summary> {
        let st = self.status(&StatusRequest {
            project: project.to_string(),
            all_projects: false,
        })?;
        let mut sum = Summary {
            project: None,
            services: st.services,
            jobs: Vec::new(),
            conflicts: Vec::new(),
        };
        let mut job_project = String::new();
        if !project.is_empty() {
            let p = self.resolve_project(project)?;
            sum.project = Some(ProjectInfo {
                name: p.name.clone(),
                root: p.root.clone(),
                default_env: p.default_env.clone(),
                envs: Some(p.envs.keys().cloned().collect()),
                pipelines: Some(p.pipelines.keys().cloned().collect()),
                deploy_envs: Some(
                    p.envs
                        .iter()
                        .filter(|(_, env)| env.deploy.is_some())
                        .map(|(name, _)| name.clone())
                        .collect(),
                ),
            });
            sum.conflicts = self.conflicts(&p);
            job_project = p.name;
        }
        let now = self.now();
        for mut j in self.d().store.list_jobs(&job_project, 0)? {
            if j.status == JobStatus::Running {
                j.duration_ms =
                    i64::try_from((now - j.started_at).whole_milliseconds()).unwrap_or(i64::MAX);
                sum.jobs.push(j);
            }
        }
        Ok(sum)
    }

    fn conflicts(&self, p: &Project) -> Vec<Conflict> {
        let mut out = Vec::new();
        for name in p.service_names() {
            let svc = &p.services[&name];
            let run = self.d().store.get_run(&p.name, &name).ok().flatten();
            let active = run.as_ref().filter(|r| r.state.active());
            for spec in &svc.ports {
                let mut c = Conflict {
                    kind: String::new(),
                    project: p.name.clone(),
                    service: name.clone(),
                    port_name: spec.name.clone(),
                    port: 0,
                    default: spec.default,
                    env: spec.env.clone(),
                    remappable: !spec.env.is_empty(),
                    holder: None,
                    detail: String::new(),
                };
                if let Some(run) = active {
                    let actual = run.ports.get(&spec.name).copied().unwrap_or(0);
                    if actual == 0 || actual == spec.default {
                        continue;
                    }
                    c.kind = conflict_kind::PORT_REMAPPED.into();
                    c.port = actual;
                    c.detail = format!(
                        "running on {actual} instead of {} (injected via {})",
                        spec.default, spec.env
                    );
                    out.push(c);
                    continue;
                }
                let Some((reason, holder)) = self.port_taken(spec.default, &p.name, &name) else {
                    continue;
                };
                c.kind = conflict_kind::PORT_BUSY.into();
                c.port = spec.default;
                c.holder = holder;
                c.detail = if c.remappable {
                    format!(
                        "{reason}; `rocket up {name}` will remap it via {}",
                        spec.env
                    )
                } else {
                    format!("{reason}; `rocket up {name}` will fail (no env var to remap)")
                };
                out.push(c);
            }
        }
        out
    }

    /// A read-only variant of the port-busy check (no stale lease cleanup):
    /// the reason `port` is taken and its holder, or `None` when free.
    fn port_taken(
        &self,
        port: u16,
        project: &str,
        service: &str,
    ) -> Option<(String, Option<PortHolder>)> {
        for l in self.d().store.list_leases().unwrap_or_default() {
            if l.port != port || (l.project == project && l.service == service) {
                continue;
            }
            if let Ok(Some(run)) = self.d().store.get_run(&l.project, &l.service)
                && run.state.active()
            {
                return Some((format!("leased by {}/{}", l.project, l.service), None));
            }
        }
        if self.d().probe.free(port) {
            return None;
        }
        Some(match self.d().probe.holder(port).ok().flatten() {
            Some(holder) => (
                format!("in use by {} (pid {})", holder.command, holder.pid),
                Some(holder),
            ),
            None => ("in use by an unknown process".to_string(), None),
        })
    }
}
