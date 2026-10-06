//! Startup reconciliation, TTL expiry and garbage collection (Go:
//! `maintenance.go`).

use crate::app::{App, key};
use crate::error::Result;
use rocket_domain::api::{GcAction, GcResult, gc_action};
use rocket_domain::{Lease, Run, RunState, ServiceKind, event_type};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// What the daemon found on start.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReconcileResult {
    pub adopted: Vec<Run>,
    pub dead: Vec<Run>,
    pub released_leases: Vec<Lease>,
    pub lost_jobs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expired_jobs: Vec<String>,
}

impl App {
    /// Compares persisted runs with reality: live processes/containers are
    /// adopted, vanished ones are marked dead and their leases released.
    pub async fn reconcile(&self) -> Result<ReconcileResult> {
        let mut res = ReconcileResult::default();
        for job in self.expire_jobs_ttl().await? {
            res.expired_jobs.push(job.id);
        }
        for r in self.d().store.list_runs()? {
            if !r.state.active() {
                continue;
            }
            let supervised = self
                .lock()
                .watches
                .contains_key(&key(&r.project, &r.service));
            if supervised {
                continue;
            }
            let live = self.is_live(&r).await;
            let mut st = self.lock();
            if live {
                let mut r = r;
                if r.state != RunState::Running {
                    r.state = RunState::Running;
                    let _ = self.save_run(&r);
                }
                if r.kind != ServiceKind::Compose {
                    self.adopt(&mut st, &r);
                }
                res.adopted.push(r);
            } else {
                res.dead.push(self.mark_dead_locked(r));
            }
        }
        let (lost, expired) = self.reconcile_jobs().await?;
        res.lost_jobs.extend(lost);
        res.expired_jobs.extend(expired);
        res.released_leases.extend(self.release_stale_leases()?);
        Ok(res)
    }

    /// Releases leases whose service has no active run.
    fn release_stale_leases(&self) -> Result<Vec<Lease>> {
        let _g = self.lock();
        let mut out = Vec::new();
        let mut done = HashSet::new();
        for l in self.d().store.list_leases()? {
            let k = key(&l.project, &l.service);
            if done.contains(&k) {
                continue;
            }
            if let Ok(Some(r)) = self.d().store.get_run(&l.project, &l.service)
                && r.state.active()
            {
                continue;
            }
            done.insert(k);
            let released = self
                .d()
                .store
                .release_leases(&l.project, &l.service)
                .unwrap_or_default();
            for rl in &released {
                self.publish_lease(event_type::PORT_RELEASED, rl);
            }
            out.extend(released);
        }
        Ok(out)
    }

    /// Stops every active run whose TTL elapsed.
    pub async fn expire_ttl(&self) -> Result<Vec<Run>> {
        let now = self.now();
        let mut by_project: BTreeMap<String, Vec<Run>> = BTreeMap::new();
        for r in self.d().store.list_runs()? {
            if r.state.active() && r.state != RunState::Starting && r.expired(now) {
                by_project.entry(r.project.clone()).or_default().push(r);
            }
        }
        let mut out = Vec::new();
        for (name, runs) in by_project {
            let p = self.resolve_project(&name).ok();
            let (stopped, _, _) = self.stop_project_runs(p.as_ref(), runs, false).await;
            out.extend(stopped);
        }
        Ok(out)
    }

    /// Reconciles state, expires TTLs, stops runs whose project is no longer
    /// registered, releases stale leases and prunes inactive run records. It
    /// only ever signals processes rocket itself started.
    pub async fn gc(&self) -> Result<GcResult> {
        let mut actions: Vec<GcAction> = Vec::new();
        let action =
            |kind: &str, project: &str, service: &str, port: u16, detail: String| GcAction {
                action: kind.into(),
                project: project.into(),
                service: service.into(),
                port,
                detail,
            };
        let rec = self.reconcile().await?;
        for r in &rec.dead {
            actions.push(action(
                gc_action::MARKED_DEAD,
                &r.project,
                &r.service,
                0,
                String::new(),
            ));
        }
        for id in &rec.lost_jobs {
            actions.push(action(gc_action::LOST_JOB, "", "", 0, id.clone()));
        }
        for id in &rec.expired_jobs {
            let job = self.get_job(id).ok();
            let (project, name) = job.map(|j| (j.project, j.name)).unwrap_or_default();
            actions.push(action(
                gc_action::EXPIRED_JOB,
                &project,
                &name,
                0,
                id.clone(),
            ));
        }
        for l in &rec.released_leases {
            actions.push(action(
                gc_action::RELEASED_LEASE,
                &l.project,
                &l.service,
                l.port,
                String::new(),
            ));
        }
        for r in self.expire_ttl().await? {
            actions.push(action(
                gc_action::EXPIRED,
                &r.project,
                &r.service,
                0,
                format!("owner {}", r.owner),
            ));
        }

        let mut orphans: BTreeMap<String, Vec<Run>> = BTreeMap::new();
        for r in self.d().store.list_runs()? {
            if r.state.active() && !matches!(self.d().store.get_project(&r.project), Ok(Some(_))) {
                orphans.entry(r.project.clone()).or_default().push(r);
            }
        }
        for (name, runs) in orphans {
            let (stopped, _, _) = self.stop_project_runs(None, runs, false).await;
            for r in stopped {
                actions.push(action(
                    gc_action::STOPPED_ORPHAN,
                    &name,
                    &r.service,
                    0,
                    format!("project {name} is not registered"),
                ));
            }
        }

        for r in self.d().store.list_runs().unwrap_or_default() {
            if r.state.active() {
                continue;
            }
            if r.state == RunState::Failed
                && r.kind == ServiceKind::Compose
                && !r.compose_project.is_empty()
            {
                continue; // whole-project `down` still needs this container cleanup tracker
            }
            if self.d().store.delete_run(&r.project, &r.service).is_ok() {
                actions.push(action(
                    gc_action::PRUNED,
                    &r.project,
                    &r.service,
                    0,
                    r.state.to_string(),
                ));
            }
        }
        actions.extend(self.prune_jobs());
        actions.sort_by(|a, b| a.action.cmp(&b.action));
        Ok(GcResult { actions })
    }
}
