//! Job execution seam (Go: `internal/app/jobs.go`).
//!
//! Job execution (run/setup/deploy/cancel/TTL/job logs) is ported in task R6.
//! This module holds the pieces the rest of the application already depends
//! on, with the exact signatures R6 fills in:
//!
//! * [`JobRun`] is the per-running-job record kept in [`State::jobs`]
//!   (Go: `jobRun`); R6 adds its fields (cancel token, pgid, done signal...).
//! * [`App::cancel_jobs`], [`App::expire_jobs_ttl`], [`App::reconcile_jobs`]
//!   and [`App::prune_jobs`] are called by `down`, `reconcile` and `gc`; until
//!   R6 they are no-ops that report nothing.
//! * [`App::get_job`] is complete because `gc` and the job endpoints need it.
//!
//! [`State::jobs`]: crate::app::State

use crate::app::App;
use crate::error::{AppError, Result};
use rocket_domain::Job;
use rocket_domain::api::GcAction;

/// Go `SetupSequence`: what `rocket setup` runs when no name is given.
pub const SETUP_SEQUENCE: [&str; 3] = ["doctor", "install", "migrate"];

/// How many finished jobs per project GC keeps (Go: `KeepJobs`).
pub const KEEP_JOBS: usize = 100;

/// A job currently executing in this daemon (Go: `jobRun`). R6 fills this in.
#[derive(Debug, Default)]
pub(crate) struct JobRun {}

impl App {
    /// Returns a job, with a live duration while it runs.
    pub fn get_job(&self, id: &str) -> Result<Job> {
        let Some(mut job) = self.d().store.get_job(id)? else {
            return Err(AppError::not_found(format!("job {id:?}")));
        };
        if job.status == rocket_domain::JobStatus::Running {
            job.duration_ms = i64::try_from((self.now() - job.started_at).whole_milliseconds())
                .unwrap_or(i64::MAX);
        }
        Ok(job)
    }

    /// Cancels the running jobs of `project` (all projects when empty),
    /// restricted to `owner` when set, and returns their ids. R6.
    pub(crate) async fn cancel_jobs(&self, _project: &str, _owner: &str) -> Vec<String> {
        Vec::new()
    }

    /// Cancels running jobs whose TTL elapsed and returns them. R6.
    pub(crate) async fn expire_jobs_ttl(&self) -> Result<Vec<Job>> {
        Ok(Vec::new())
    }

    /// Marks jobs the previous daemon left running as lost (or expired) and
    /// returns `(lost, expired)` ids. R6.
    pub(crate) async fn reconcile_jobs(&self) -> Result<(Vec<String>, Vec<String>)> {
        Ok((Vec::new(), Vec::new()))
    }

    /// Prunes finished jobs beyond [`KEEP_JOBS`] per project. R6.
    pub(crate) fn prune_jobs(&self) -> Vec<GcAction> {
        Vec::new()
    }
}
