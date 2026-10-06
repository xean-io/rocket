//! Jobs: pipelines, setup actions and deploys (Go: `internal/app/jobs.go`).
//!
//! [`App::start_job`] validates a request, persists the job and returns while
//! a background task runs the prerequisite startup and the steps. The task
//! checks the persisted deadline at every transition, so delayed scheduling
//! can never start another step or record a late success.
//!
//! # Differences from the Go implementation
//!
//! * `context.Context` becomes a [`CancellationToken`]: [`App::wait_job`]
//!   takes one; the other methods run to completion and can be abandoned by
//!   dropping their future (cancellation itself is [`App::cancel_job`]).
//! * [`JobRun`] keeps its cancel and done signals as tokens. The job's own
//!   token is a child of the application's shutdown token, so closing the
//!   app interrupts waits without finishing the job (the next daemon adopts
//!   or marks it lost).

use crate::app::{App, State, blank_event};
use crate::error::{AppError, Result};
use crate::go_duration::parse_duration;
use rocket_domain::api::{
    GcAction, JobLogsResult, JobOutcome, JobRequest, JobsResult, UpRequest, gc_action,
};
use rocket_domain::{
    DEFAULT_OWNER, Job, JobKind, JobStatus, Project, Step, build_env, compose_project_name,
    event_type, is_agent_owner, merge_profiles,
};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

/// Go `SetupSequence`: what `rocket setup` runs when no name is given.
pub const SETUP_SEQUENCE: [&str; 3] = ["doctor", "install", "migrate"];

/// How many finished jobs per project GC keeps (Go: `KeepJobs`).
pub const KEEP_JOBS: usize = 100;

/// The failure recorded for a job whose deadline elapsed.
const TTL_EXPIRED: &str = "ttl expired";

/// Lists jobs of one project (or all) (Go: `JobsRequest`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobsRequest {
    /// Registered name or absolute path; empty lists every project.
    pub project: String,
    /// Ignores `project`.
    pub all_projects: bool,
    /// At most this many jobs; `0` selects the default of 50.
    pub limit: i64,
}

/// Builds the blocking-command summary of a job (Go: `Outcome`).
pub fn outcome(j: &Job, tail: Vec<String>) -> JobOutcome {
    JobOutcome {
        job: j.id.clone(),
        project: j.project.clone(),
        kind: j.kind,
        name: j.name.clone(),
        status: j.status,
        exit_code: j.exit_code,
        duration_ms: j.duration_ms,
        log_path: j.log_path.clone(),
        tail,
        error: j.error.clone(),
    }
}

/// The daemon-side handle of a supervised job (Go: `jobRun`), guarded by the
/// application lock.
#[derive(Debug, Clone)]
pub(crate) struct JobRun {
    pub canceled: bool,
    pub reason: String,
    /// Cancels the job's prerequisite startup and the wait for its step.
    pub cancel: CancellationToken,
    /// Process group of the step in flight (`0` when none).
    pub pgid: i32,
    /// Cancelled once the job's supervision ended.
    pub done: CancellationToken,
    /// Signals the process group at most once (Go: `stopOnce`).
    pub stop: Arc<OnceCell<Option<String>>>,
}

impl JobRun {
    fn new(cancel: CancellationToken, pgid: i32) -> Self {
        Self {
            canceled: false,
            reason: String::new(),
            cancel,
            pgid,
            done: CancellationToken::new(),
            stop: Arc::new(OnceCell::new()),
        }
    }
}

/// Everything the background task of a job owns.
struct ExecJob {
    project: Project,
    job: Job,
    needs: Vec<String>,
    env: Vec<String>,
    logf: File,
    token: CancellationToken,
    done: CancellationToken,
    stop: Arc<OnceCell<Option<String>>>,
}

impl App {
    /// Validates the request, persists the job and runs its steps in the
    /// background. Returns as soon as the job is running.
    pub fn start_job(&self, req: JobRequest) -> Result<Job> {
        let p = self.resolve_project(&req.project)?;
        let owner = if req.owner.is_empty() {
            DEFAULT_OWNER.to_string()
        } else {
            req.owner.clone()
        };
        let (name, env, steps) = self.resolve_job(&p, &req, &owner)?;
        let profiles = if req.kind == JobKind::Deploy {
            Vec::new()
        } else {
            let selected = p
                .envs
                .get(&env)
                .map(|e| e.profiles.clone())
                .unwrap_or_default();
            merge_profiles(&[selected, req.profiles.clone()])
        };
        let mut needs = Vec::new();
        if req.kind == JobKind::Pipeline {
            needs = p.pipeline_needs.get(&name).cloned().unwrap_or_default();
            if !needs.is_empty() {
                p.expand_targets(&needs).map_err(|e| {
                    AppError::invalid(format!("pipeline {name:?} prerequisites: {e}"))
                })?;
            }
        }
        let started_at = self.now();
        let mut expires_at = None;
        if !req.ttl.is_empty() {
            let invalid = || {
                AppError::invalid(format!(
                    "invalid ttl {:?} (use a positive Go duration like 30m)",
                    req.ttl
                ))
            };
            let ns = parse_duration(&req.ttl)
                .filter(|ns| *ns > 0)
                .ok_or_else(invalid)?;
            expires_at = Some(
                started_at
                    .checked_add(time::Duration::nanoseconds_i128(ns))
                    .ok_or_else(invalid)?,
            );
        }
        let dotenv = self
            .d()
            .env
            .dotenv(Path::new(&p.root), &p.dotenv)
            .map_err(|e| AppError::internal(format!("read dotenv: {e}")))?;
        let identity = BTreeMap::from([(
            "COMPOSE_PROJECT_NAME".to_string(),
            compose_project_name(&p.name, &env),
        )]);
        let child_env = build_env(&self.base_env(), &[&dotenv, &identity]);

        let id = (self.cfg().new_id)();
        let (logf, log_path) = self
            .d()
            .logs
            .open_job(&p.name, &id)
            .map_err(|e| AppError::internal(format!("open job log: {e}")))?;
        let job = Job {
            id: id.clone(),
            project: p.name.clone(),
            name,
            kind: req.kind,
            env,
            profiles,
            owner: owner.clone(),
            steps,
            args: req.args.clone(),
            status: JobStatus::Running,
            step: 0,
            pid: 0,
            pgid: 0,
            exit_code: None,
            started_at,
            expires_at,
            finished_at: None,
            duration_ms: 0,
            log_path,
            error: String::new(),
        };
        let _ = writeln!(
            &logf,
            "=== rocket: job {id} ({} {}, owner {owner}) started at {} ===",
            job.kind,
            job.name,
            started_at
                .replace_nanosecond(0)
                .unwrap_or(started_at)
                .format(&Rfc3339)
                .unwrap_or_default()
        );
        let token = self.inner.shutdown.child_token();
        let jr = JobRun::new(token.clone(), 0);
        let (done, stop) = (jr.done.clone(), jr.stop.clone());
        {
            let mut st = self.lock();
            if let Err(e) = self.save_job(&job) {
                drop(st);
                token.cancel();
                self.d().logs.close_job(&p.name, &id);
                return Err(e.into());
            }
            st.jobs.insert(id, jr);
        }

        let exec = ExecJob {
            project: p,
            job: job.clone(),
            needs,
            env: child_env,
            logf,
            token,
            done: done.clone(),
            stop,
        };
        let app = self.clone();
        self.inner
            .tasks
            .spawn(async move { app.exec_job(exec).await });
        self.watch_job_ttl(&job, done);
        Ok(job)
    }

    /// Resolves the job's name, environment and steps, applying the deploy
    /// confirmation gate (Go: `resolveJob`).
    fn resolve_job(
        &self,
        p: &Project,
        req: &JobRequest,
        owner: &str,
    ) -> Result<(String, String, Vec<Step>)> {
        let mut env = req.env.clone();
        if req.kind == JobKind::Deploy {
            if !req.profiles.is_empty() {
                return Err(AppError::invalid(
                    "profiles select setup/pipeline prerequisites and are not supported for deploy jobs",
                ));
            }
            if !env.is_empty() && env != req.name {
                return Err(AppError::invalid(format!(
                    "deploy env {env:?} must match target {:?}",
                    req.name
                )));
            }
            env.clone_from(&req.name);
        } else if env.is_empty() {
            env.clone_from(&p.default_env);
        }
        if !p.envs.contains_key(&env) {
            return Err(AppError::invalid(format!(
                "unknown env {env:?} in project {}",
                p.name
            )));
        }
        match req.kind {
            JobKind::Setup => {
                if req.name.is_empty() {
                    let steps: Vec<Step> = SETUP_SEQUENCE
                        .iter()
                        .filter_map(|n| p.setup.get(*n).cloned())
                        .collect();
                    if steps.is_empty() {
                        return Err(AppError::invalid(format!(
                            "project {} defines none of setup.{} in rocket.yaml",
                            p.name,
                            SETUP_SEQUENCE.join("/")
                        )));
                    }
                    return Ok(("setup".into(), env, steps));
                }
                let Some(step) = p.setup.get(&req.name) else {
                    return Err(AppError::invalid(format!(
                        "project {} has no setup.{} (defined: {})",
                        p.name,
                        req.name,
                        keys(&p.setup)
                    )));
                };
                Ok((req.name.clone(), env, vec![step.clone()]))
            }
            JobKind::Pipeline => {
                if req.name.is_empty() {
                    return Err(AppError::invalid("pipeline name is required"));
                }
                if let Some(steps) = p.pipelines.get(&req.name) {
                    return Ok((req.name.clone(), env, steps.clone()));
                }
                if self.d().tasks.taskfile(Path::new(&p.root)).is_none() {
                    return Err(AppError::invalid(format!(
                        "project {} has no pipeline {:?} and no Taskfile to fall back to (pipelines: {})",
                        p.name,
                        req.name,
                        keys(&p.pipelines)
                    )));
                }
                let step = Step {
                    task: req.name.clone(),
                    ..Step::default()
                };
                Ok((req.name.clone(), env, vec![step]))
            }
            JobKind::Deploy => {
                let Some(e) = p.envs.get(&req.name) else {
                    return Err(AppError::invalid(format!(
                        "unknown env {:?} in project {}",
                        req.name, p.name
                    )));
                };
                let Some(deploy) = &e.deploy else {
                    return Err(AppError::invalid(format!(
                        "env {:?} of project {} has no deploy",
                        req.name, p.name
                    )));
                };
                if deploy.confirm && !req.yes {
                    return Err(AppError::confirmation_required(format!(
                        "deploying {} to {} needs --yes",
                        p.name, req.name
                    )));
                }
                if is_agent_owner(owner) && !(req.yes && req.allow_agent_deploy) {
                    return Err(AppError::confirmation_required(format!(
                        "owner {owner} is an agent; agents may deploy only with --yes and ROCKET_ALLOW_DEPLOY=1 set by a human"
                    )));
                }
                Ok((req.name.clone(), req.name.clone(), vec![deploy.step()]))
            }
        }
    }

    /// The argv of a step; `args` are appended without shell quoting.
    fn step_argv(&self, s: &Step, args: &[String]) -> Vec<String> {
        if !s.task.is_empty() {
            return self.d().tasks.argv(&s.task, args);
        }
        if args.is_empty() {
            return vec!["/bin/sh".into(), "-c".into(), s.run.clone()];
        }
        // "$@" keeps every argument intact.
        let mut argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("{} \"$@\"", s.run),
            "rocket".to_string(),
        ];
        argv.extend(args.iter().cloned());
        argv
    }

    /// Runs the job in the background and releases its supervision handles.
    async fn exec_job(&self, x: ExecJob) {
        let (token, done) = (x.token.clone(), x.done.clone());
        self.run_job(x).await;
        done.cancel();
        token.cancel();
    }

    async fn run_job(&self, x: ExecJob) {
        let ExecJob {
            project: p,
            mut job,
            needs,
            env,
            logf,
            token,
            done: _,
            stop,
        } = x;
        let shutdown = self.inner.shutdown.clone();
        let mut status = JobStatus::Succeeded;
        let mut code: Option<i32> = None;
        let mut failure = String::new();

        if !needs.is_empty() {
            let _ = writeln!(&logf, "=== rocket: prerequisites: {} ===", needs.join(", "));
            let result = self
                .up_with(
                    UpRequest {
                        project: p.root.clone(),
                        services: needs,
                        env: job.env.clone(),
                        profiles: job.profiles.clone(),
                        owner: job.owner.clone(),
                        ttl: String::new(),
                    },
                    &token,
                    job.expires_at,
                )
                .await;
            if shutdown.is_cancelled() {
                return;
            }
            match result {
                Err(e) => {
                    status = JobStatus::Failed;
                    failure = format!("prerequisite startup: {e}");
                }
                Ok(r) if r.failed() => {
                    let problems: Vec<String> = r
                        .services
                        .iter()
                        .filter(|s| {
                            s.action == rocket_domain::api::service_action::FAILED
                                || s.action == rocket_domain::api::service_action::SKIPPED
                        })
                        .map(|s| format!("{}: {}", s.service, s.error))
                        .collect();
                    status = JobStatus::Failed;
                    failure = format!("prerequisite startup failed: {}", problems.join("; "));
                }
                Ok(_) => {}
            }
            let mut st = self.lock();
            self.expire_job_run_locked(&mut st, &job);
            if let Some(reason) = canceled_reason(&st, &job.id) {
                status = JobStatus::Canceled;
                failure = reason;
            }
        }

        let steps = job.steps.clone();
        for (i, step) in steps.iter().enumerate() {
            if status != JobStatus::Succeeded {
                break;
            }
            let args = if i == steps.len() - 1 {
                job.args.clone()
            } else {
                Vec::new()
            };
            let _ = writeln!(
                &logf,
                "=== rocket: step {}/{}: {} ===",
                i + 1,
                steps.len(),
                step.describe()
            );

            // Start under the lock so a concurrent cancel either sees this
            // step's pgid or prevents it from starting.
            let mut handle = {
                let mut st = self.lock();
                self.expire_job_run_locked(&mut st, &job);
                if let Some(reason) = canceled_reason(&st, &job.id) {
                    failure = reason;
                    status = JobStatus::Canceled;
                    break;
                }
                let started = logf
                    .try_clone()
                    .map_err(|e| e.to_string())
                    .and_then(|output| {
                        self.d()
                            .runner
                            .start(rocket_domain::ports::ProcessSpec {
                                argv: self.step_argv(step, &args),
                                dir: PathBuf::from(&p.root),
                                env: env.clone(),
                                output: Some(output),
                            })
                            .map_err(|e| e.to_string())
                    });
                match started {
                    Err(e) => {
                        drop(st);
                        status = JobStatus::Failed;
                        failure = format!("step {} ({}): {e}", i + 1, step.describe());
                        let _ = writeln!(&logf, "rocket: {failure}");
                        break;
                    }
                    Ok(h) => {
                        if let Some(jr) = st.jobs.get_mut(&job.id) {
                            jr.pgid = h.pgid;
                        }
                        job.step = i32::try_from(i + 1).unwrap_or(i32::MAX);
                        (job.pid, job.pgid) = (h.pid, h.pgid);
                        let _ = self.save_job(&job);
                        h
                    }
                }
            };

            let exit = tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    // Daemon shutdown: the step keeps running and the next
                    // daemon marks the job lost once its process group is gone.
                    return;
                }
                c = &mut handle.done => c.unwrap_or(-1),
                () = token.cancelled() => {
                    if shutdown.is_cancelled() {
                        return;
                    }
                    let _ = self.stop_job_process(&stop, handle.pgid).await;
                    tokio::select! {
                        biased;
                        () = shutdown.cancelled() => return,
                        c = &mut handle.done => c.unwrap_or(-1),
                    }
                }
            };
            code = Some(exit);
            let canceled = {
                let mut st = self.lock();
                if let Some(jr) = st.jobs.get_mut(&job.id) {
                    jr.pgid = 0;
                }
                self.expire_job_run_locked(&mut st, &job);
                canceled_reason(&st, &job.id)
            };
            if let Some(reason) = canceled {
                status = JobStatus::Canceled;
                failure = reason;
                break;
            }
            if exit != 0 {
                status = JobStatus::Failed;
                failure = format!(
                    "step {} ({}) exited with code {exit}",
                    i + 1,
                    step.describe()
                );
                break;
            }
        }

        {
            let mut st = self.lock();
            self.expire_job_run_locked(&mut st, &job);
            if let Some(reason) = canceled_reason(&st, &job.id) {
                status = JobStatus::Canceled;
                failure = reason;
            }
        }
        let _ = writeln!(&logf, "\n=== rocket: job {} {status} ===", job.id);
        drop(logf);
        // Flush remaining job.log events before the terminal job.state.
        self.d().logs.close_job(&job.project, &job.id);

        let mut st = self.lock();
        self.expire_job_run_locked(&mut st, &job);
        if let Some(jr) = st.jobs.remove(&job.id)
            && jr.canceled
        {
            status = JobStatus::Canceled;
            failure = jr.reason;
        }
        self.finish_job_locked(&mut st, &job, status, code, failure);
    }

    /// Checks the persisted deadline at transitions as well as on the timer
    /// (Go: `expireJobRunLocked`).
    fn expire_job_run_locked(&self, st: &mut State, job: &Job) {
        if let Some(jr) = st.jobs.get_mut(&job.id)
            && !jr.canceled
            && job.expired(self.now())
        {
            jr.canceled = true;
            jr.reason = TTL_EXPIRED.into();
            jr.cancel.cancel();
        }
    }

    /// Records the end of a job unless a concurrent cancellation or
    /// reconciliation already settled it. A deadline that elapsed turns any
    /// non-canceled outcome into `canceled: ttl expired`.
    fn finish_job_locked(
        &self,
        _st: &mut State,
        job: &Job,
        mut status: JobStatus,
        code: Option<i32>,
        mut failure: String,
    ) -> Job {
        if let Ok(Some(current)) = self.d().store.get_job(&job.id)
            && current.status.terminal()
        {
            return current;
        }
        let t = self.now();
        if status != JobStatus::Canceled && job.expired(t) {
            status = JobStatus::Canceled;
            failure = TTL_EXPIRED.into();
        }
        let mut job = job.clone();
        job.status = status;
        job.exit_code = code;
        job.error = failure;
        job.finished_at = Some(t);
        job.duration_ms = millis(t - job.started_at);
        let _ = self.save_job(&job);
        job
    }

    /// Persists a job and publishes `job.state`.
    fn save_job(&self, j: &Job) -> rocket_domain::ports::Result<()> {
        self.d().store.save_job(j)?;
        if self.d().bus.is_some() {
            let mut event = blank_event(event_type::JOB_STATE, self.now());
            event.project.clone_from(&j.project);
            event.job_id.clone_from(&j.id);
            event.status = Some(j.status);
            event.job = Some(Box::new(j.clone()));
            self.publish(event);
        }
        Ok(())
    }

    /// Returns one job; a running job reports its elapsed duration.
    pub fn get_job(&self, id: &str) -> Result<Job> {
        let Some(mut job) = self.d().store.get_job(id)? else {
            return Err(AppError::not_found(format!("job {id:?}")));
        };
        if job.status == JobStatus::Running {
            job.duration_ms = millis(self.now() - job.started_at);
        }
        Ok(job)
    }

    /// Blocks until the job is terminal or `cancel` fires.
    pub async fn wait_job(&self, id: &str, cancel: &CancellationToken) -> Result<Job> {
        loop {
            let job = self.get_job(id)?;
            if job.status.terminal() {
                return Ok(job);
            }
            let done = self.lock().jobs.get(id).map(|jr| jr.done.clone());
            let finished = async {
                match done.filter(|d| !d.is_cancelled()) {
                    Some(d) => d.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = finished => {}
                () = tokio::time::sleep(Duration::from_millis(250)) => {}
                () = cancel.cancelled() => return Err(AppError::Canceled),
            }
        }
    }

    /// Returns jobs newest first.
    pub fn list_jobs(&self, req: &JobsRequest) -> Result<JobsResult> {
        let project = if !req.all_projects && !req.project.is_empty() {
            self.resolve_project(&req.project)?.name
        } else {
            String::new()
        };
        let limit = if req.limit <= 0 { 50 } else { req.limit };
        let mut jobs = self.d().store.list_jobs(&project, limit)?;
        let now = self.now();
        for j in &mut jobs {
            if j.status == JobStatus::Running {
                j.duration_ms = millis(now - j.started_at);
            }
        }
        Ok(JobsResult { jobs })
    }

    /// The last `n` lines of a job log (`0` selects 100).
    pub fn job_logs(&self, id: &str, n: usize) -> Result<JobLogsResult> {
        let j = self.get_job(id)?;
        let n = if n == 0 { 100 } else { n };
        let lines = self
            .d()
            .logs
            .tail_job(&j.project, &j.id, n)
            .unwrap_or_default();
        Ok(JobLogsResult {
            job: j.id,
            project: j.project,
            lines,
        })
    }

    /// Stops the job's current process group and waits for the job to
    /// settle. Cancelling a finished job is a no-op.
    pub async fn cancel_job(&self, id: &str) -> Result<Job> {
        self.cancel_job_reason(id, "canceled").await
    }

    async fn cancel_job_reason(&self, id: &str, reason: &str) -> Result<Job> {
        enum Plan {
            Orphan(Job, JobRun),
            Supervised(Job, JobRun),
        }
        let j = self.get_job(id)?;
        if j.status.terminal() {
            return Ok(j);
        }
        let plan = {
            let mut st = self.lock();
            let mut j = j;
            if let Ok(Some(current)) = self.d().store.get_job(id) {
                j = current;
                if j.status.terminal() {
                    return Ok(j);
                }
            }
            let reason = if j.expired(self.now()) {
                TTL_EXPIRED
            } else {
                reason
            };
            match st.jobs.get_mut(id) {
                None => {
                    let mut jr = JobRun::new(CancellationToken::new(), j.pgid);
                    jr.canceled = true;
                    jr.reason = reason.to_string();
                    st.jobs.insert(id.to_string(), jr.clone());
                    Plan::Orphan(j, jr)
                }
                Some(jr) => {
                    if !jr.canceled {
                        jr.canceled = true;
                        jr.reason = reason.to_string();
                    }
                    jr.cancel.cancel();
                    Plan::Supervised(j, jr.clone())
                }
            }
        };
        match plan {
            Plan::Orphan(j, jr) => {
                // A job adopted from a previous daemon that this one does not
                // supervise: stop its process group and settle it here.
                if j.pgid > 0
                    && self.d().runner.alive(j.pid, j.pgid)
                    && let Err(e) = self.stop_job_process(&jr.stop, j.pgid).await
                {
                    self.lock().jobs.remove(id);
                    jr.done.cancel();
                    return Err(e);
                }
                self.d().logs.close_job(&j.project, &j.id);
                let final_job = {
                    let mut st = self.lock();
                    st.jobs.remove(id);
                    self.finish_job_locked(
                        &mut st,
                        &j,
                        JobStatus::Canceled,
                        None,
                        jr.reason.clone(),
                    )
                };
                jr.done.cancel();
                Ok(final_job)
            }
            Plan::Supervised(j, jr) => {
                let _ = j;
                if jr.pgid > 0 {
                    self.stop_job_process(&jr.stop, jr.pgid).await?;
                }
                let wait =
                    self.cfg().stop_grace + self.cfg().poll_interval + Duration::from_secs(5);
                tokio::select! {
                    () = jr.done.cancelled() => {}
                    () = tokio::time::sleep(wait) => {
                        return Err(AppError::internal("timed out waiting for job cancellation"));
                    }
                }
                self.get_job(id)
            }
        }
    }

    /// Signals a job's process group once, however many callers race here;
    /// every caller sees the same outcome. The cleanup deadline is independent
    /// of the caller (Go: `stopJobProcess`).
    async fn stop_job_process(&self, stop: &OnceCell<Option<String>>, pgid: i32) -> Result<()> {
        let outcome = stop
            .get_or_init(|| async {
                let grace = self.cfg().stop_grace;
                let stopping = self.d().runner.stop(pgid, grace);
                match tokio::time::timeout(grace + Duration::from_secs(5), stopping).await {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(_) => Some("context deadline exceeded".to_string()),
                }
            })
            .await;
        match outcome {
            None => Ok(()),
            Some(e) => Err(AppError::internal(e.clone())),
        }
    }

    /// Cancels the job when its deadline elapses, independent of any
    /// maintenance run.
    fn watch_job_ttl(&self, job: &Job, done: CancellationToken) {
        let Some(deadline) = job.expires_at else {
            return;
        };
        let remaining = Duration::try_from(deadline - self.now()).unwrap_or(Duration::ZERO);
        let (app, id) = (self.clone(), job.id.clone());
        self.inner.tasks.spawn(async move {
            let shutdown = app.inner.shutdown.clone();
            tokio::select! {
                // Daemon shutdown retains child jobs for adoption.
                () = shutdown.cancelled() => return,
                () = done.cancelled() => return,
                () = tokio::time::sleep(remaining) => {}
            }
            if shutdown.is_cancelled() {
                return;
            }
            let bound = app.cfg().stop_grace + app.cfg().poll_interval + Duration::from_secs(5);
            let _ = tokio::time::timeout(bound, app.cancel_job_reason(&id, TTL_EXPIRED)).await;
        });
    }

    /// Cancels expired running jobs without acquiring any project gate and
    /// returns the ones that ended as `canceled: ttl expired`.
    pub(crate) async fn expire_jobs_ttl(&self) -> Result<Vec<Job>> {
        let mut expired = Vec::new();
        for job in self.d().store.list_jobs("", 0)? {
            if job.status != JobStatus::Running || !job.expired(self.now()) {
                continue;
            }
            let fin = self.cancel_job_reason(&job.id, TTL_EXPIRED).await?;
            if fin.status == JobStatus::Canceled && fin.error == TTL_EXPIRED {
                expired.push(fin);
            }
        }
        Ok(expired)
    }

    /// Cancels the running jobs of `project` (all projects when empty),
    /// restricted to `owner` when set, and returns their ids.
    pub(crate) async fn cancel_jobs(&self, project: &str, owner: &str) -> Vec<String> {
        let Ok(jobs) = self.d().store.list_jobs(project, 0) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for j in jobs {
            if j.status != JobStatus::Running || (!owner.is_empty() && j.owner != owner) {
                continue;
            }
            if self.cancel_job(&j.id).await.is_ok() {
                out.push(j.id);
            }
        }
        out
    }

    /// Adopts running jobs whose process group is still alive and marks the
    /// rest lost (or expired). Returns `(lost, expired)` ids.
    pub(crate) async fn reconcile_jobs(&self) -> Result<(Vec<String>, Vec<String>)> {
        let jobs = self.d().store.list_jobs("", 0)?;
        let (mut lost, mut expired) = (Vec::new(), Vec::new());
        for j in jobs {
            if j.status != JobStatus::Running {
                continue;
            }
            if j.expired(self.now()) {
                self.cancel_job_reason(&j.id, TTL_EXPIRED).await?;
                expired.push(j.id);
                continue;
            }
            let mut st = self.lock();
            if st.jobs.contains_key(&j.id) {
                continue;
            }
            if j.pid > 0 && self.d().runner.alive(j.pid, j.pgid) {
                self.adopt_job_locked(&mut st, &j);
            } else {
                let reason = "its process was gone when the daemon (re)started; exit code unknown";
                let fin = self.finish_job_locked(&mut st, &j, JobStatus::Lost, None, reason.into());
                if fin.status == JobStatus::Lost {
                    lost.push(j.id);
                } else if fin.status == JobStatus::Canceled && fin.error == TTL_EXPIRED {
                    expired.push(j.id);
                }
            }
        }
        Ok((lost, expired))
    }

    /// Polls a job step started by a previous daemon. Its exit code cannot be
    /// observed, so the job ends as lost (or canceled). Needs the state lock.
    fn adopt_job_locked(&self, st: &mut State, j: &Job) {
        let jr = JobRun::new(CancellationToken::new(), j.pgid);
        let done = jr.done.clone();
        st.jobs.insert(j.id.clone(), jr);
        self.watch_job_ttl(j, done.clone());
        let (app, j) = (self.clone(), j.clone());
        self.inner.tasks.spawn(async move {
            let period = app.cfg().poll_interval;
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            let shutdown = app.inner.shutdown.clone();
            loop {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => break,
                    _ = ticker.tick() => {
                        if app.d().runner.alive(j.pid, j.pgid) {
                            continue;
                        }
                        app.d().logs.close_job(&j.project, &j.id);
                        let mut st = app.lock();
                        let jr = st.jobs.remove(&j.id);
                        match jr.filter(|jr| jr.canceled) {
                            Some(jr) => {
                                app.finish_job_locked(&mut st, &j, JobStatus::Canceled, None, jr.reason);
                            }
                            None => {
                                app.finish_job_locked(
                                    &mut st,
                                    &j,
                                    JobStatus::Lost,
                                    None,
                                    "the daemon restarted while this job ran; exit code unknown and later steps did not run".into(),
                                );
                            }
                        }
                        break;
                    }
                }
            }
            done.cancel();
        });
    }

    /// Deletes finished jobs beyond the newest [`KEEP_JOBS`] per project.
    pub(crate) fn prune_jobs(&self) -> Vec<GcAction> {
        let Ok(jobs) = self.d().store.list_jobs("", 0) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for j in jobs {
            if !j.status.terminal() {
                continue;
            }
            let n = seen.entry(j.project.clone()).or_default();
            *n += 1;
            if *n <= KEEP_JOBS {
                continue;
            }
            let _ = self.d().logs.remove_job(&j.project, &j.id);
            if self.d().store.delete_job(&j.id).is_ok() {
                out.push(GcAction {
                    action: gc_action::PRUNED_JOB.into(),
                    project: j.project,
                    service: j.name,
                    port: 0,
                    detail: j.id,
                });
            }
        }
        out
    }
}

/// The cancellation reason of a supervised job that was canceled.
fn canceled_reason(st: &State, id: &str) -> Option<String> {
    st.jobs
        .get(id)
        .filter(|jr| jr.canceled)
        .map(|jr| jr.reason.clone())
}

fn millis(d: time::Duration) -> i64 {
    i64::try_from(d.whole_milliseconds()).unwrap_or(i64::MAX)
}

/// Sorted keys for error messages, `none` when empty.
fn keys<V>(m: &BTreeMap<String, V>) -> String {
    if m.is_empty() {
        "none".to_string()
    } else {
        m.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}
