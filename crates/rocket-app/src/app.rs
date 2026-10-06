//! The application core: dependency wiring, shared state, event helpers,
//! per-project gates and the project registry (Go: `internal/app/app.go`).

use crate::error::{AppError, Result};
use crate::jobs::JobRun;
use crate::watch::Watch;
use rocket_domain::ports::{
    ComposeDriver, EnvSource, EventBus, HealthProbe, LogSink, ManifestLoader, PortProbe,
    ProcessRunner, Store, TaskDriver,
};
use rocket_domain::{Event, Lease, Project, ProjectRef, Run, event_type};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use time::{OffsetDateTime, UtcOffset};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// Injectable clock (Go: `Deps.Now`).
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;
/// Source of the daemon's base environment (Go: `Deps.BaseEnv`, `os.Environ`).
pub type BaseEnv = Arc<dyn Fn() -> Vec<String> + Send + Sync>;
/// Job id generator (Go: `Deps.NewID`).
pub type IdGen = Arc<dyn Fn() -> String + Send + Sync>;

/// Default SIGTERM -> SIGKILL grace.
pub const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(10);
/// Default delay between readiness probes.
pub const DEFAULT_HEALTH_INTERVAL: Duration = Duration::from_millis(250);
/// Default liveness polling interval of adopted processes.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Default wait for an early exit of probe-less processes.
pub const DEFAULT_START_GRACE: Duration = Duration::from_millis(500);

/// Tunables and test seams. A zero duration or a `None` closure selects the
/// default, exactly like Go's zero values in `Deps`.
#[derive(Clone, Default)]
pub struct Options {
    /// Wall clock; default is the system clock.
    pub now: Option<Clock>,
    /// Daemon environment; default is the process environment.
    pub base_env: Option<BaseEnv>,
    /// Job id generator; default is `j` + 10 random hex digits.
    pub new_id: Option<IdGen>,
    /// SIGTERM -> SIGKILL grace (default 10s).
    pub stop_grace: Duration,
    /// Delay between readiness probes (default 250ms).
    pub health_interval: Duration,
    /// Liveness polling of adopted processes (default 2s).
    pub poll_interval: Duration,
    /// Wait for an early exit of probe-less processes (default 500ms).
    pub start_grace: Duration,
}

/// Wires adapters into the application. The ports are injected as trait
/// objects; tests inject in-memory fakes the same way.
#[derive(Clone)]
pub struct Deps {
    pub store: Arc<dyn Store>,
    pub manifests: Arc<dyn ManifestLoader>,
    pub env: Arc<dyn EnvSource>,
    pub runner: Arc<dyn ProcessRunner>,
    pub compose: Arc<dyn ComposeDriver>,
    pub tasks: Arc<dyn TaskDriver>,
    pub probe: Arc<dyn PortProbe>,
    pub health: Arc<dyn HealthProbe>,
    /// Optional: without a bus nothing is published.
    pub bus: Option<Arc<dyn EventBus>>,
    pub logs: Arc<dyn LogSink>,
    pub options: Options,
}

/// [`Options`] with defaults applied.
pub(crate) struct Config {
    pub now: Clock,
    pub base_env: BaseEnv,
    /// Used by job execution (R6).
    #[allow(dead_code)]
    pub new_id: IdGen,
    pub stop_grace: Duration,
    pub health_interval: Duration,
    pub poll_interval: Duration,
    pub start_grace: Duration,
}

impl Config {
    fn resolve(o: &Options) -> Self {
        fn or(value: Duration, default: Duration) -> Duration {
            if value.is_zero() { default } else { value }
        }
        Self {
            now: o
                .now
                .clone()
                .unwrap_or_else(|| Arc::new(OffsetDateTime::now_utc)),
            base_env: o.base_env.clone().unwrap_or_else(|| {
                Arc::new(|| {
                    std::env::vars_os()
                        .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
                        .collect()
                })
            }),
            new_id: o.new_id.clone().unwrap_or_else(|| Arc::new(new_job_id)),
            stop_grace: or(o.stop_grace, DEFAULT_STOP_GRACE),
            health_interval: or(o.health_interval, DEFAULT_HEALTH_INTERVAL),
            poll_interval: or(o.poll_interval, DEFAULT_POLL_INTERVAL),
            start_grace: or(o.start_grace, DEFAULT_START_GRACE),
        }
    }
}

/// Go `newJobID`: `j` followed by 5 random bytes in hex.
pub(crate) fn new_job_id() -> String {
    let bytes = uuid::Uuid::new_v4();
    let mut id = String::from("j");
    for b in &bytes.as_bytes()[..5] {
        id.push_str(&format!("{b:02x}"));
    }
    id
}

/// Mutable state guarded by [`App::lock`] (Go: `App.mu`). The same mutex also
/// serializes store read-modify-write sequences; it is never held across an
/// `.await`.
#[derive(Default)]
pub(crate) struct State {
    /// Processes this daemon supervises, by `project/service`.
    pub watches: HashMap<String, Watch>,
    /// Running jobs by id (R6).
    #[allow(dead_code)]
    pub jobs: HashMap<String, JobRun>,
}

pub(crate) struct Inner {
    pub d: Deps,
    pub cfg: Config,
    state: Mutex<State>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Cancelled by [`App::close`]; stops background watchers (Go: `App.ctx`).
    pub shutdown: CancellationToken,
    pub tasks: TaskTracker,
}

/// The rocket application core, hosted by the daemon. Cloning is cheap: all
/// clones share one state.
#[derive(Clone)]
pub struct App {
    pub(crate) inner: Arc<Inner>,
}

/// Holds a project's operation gate until dropped.
pub(crate) struct ProjectGuard(#[allow(dead_code)] tokio::sync::OwnedMutexGuard<()>);

pub(crate) fn key(project: &str, service: &str) -> String {
    format!("{project}/{service}")
}

impl App {
    /// Builds the application with defaults for unset options.
    pub fn new(deps: Deps) -> Self {
        let cfg = Config::resolve(&deps.options);
        Self {
            inner: Arc::new(Inner {
                d: deps,
                cfg,
                state: Mutex::new(State::default()),
                locks: Mutex::new(HashMap::new()),
                shutdown: CancellationToken::new(),
                tasks: TaskTracker::new(),
            }),
        }
    }

    /// Stops background watchers. Supervised processes keep running and are
    /// adopted by the next daemon through [`App::reconcile`].
    pub async fn close(&self) {
        self.inner.shutdown.cancel();
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
    }

    pub(crate) fn d(&self) -> &Deps {
        &self.inner.d
    }

    pub(crate) fn cfg(&self) -> &Config {
        &self.inner.cfg
    }

    /// Locks the shared state. Poisoning is ignored: the guarded data stays
    /// consistent because every critical section is a short, panic-free
    /// store/map update.
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The current time in UTC.
    pub(crate) fn now(&self) -> OffsetDateTime {
        (self.cfg().now)().to_offset(UtcOffset::UTC)
    }

    pub(crate) fn base_env(&self) -> Vec<String> {
        (self.cfg().base_env)()
    }

    /// Acquires `name`'s gate, waiting behind other project operations.
    pub(crate) async fn lock_project(&self, name: &str) -> ProjectGuard {
        ProjectGuard(self.gate(name).lock_owned().await)
    }

    /// [`lock_project`](Self::lock_project) that gives up when `cancel` fires,
    /// so a queued startup can be canceled without waiting for another
    /// operation to release the project (Go: `projectGate.LockContext`).
    pub(crate) async fn lock_project_cancellable(
        &self,
        name: &str,
        cancel: &CancellationToken,
    ) -> Result<ProjectGuard> {
        tokio::select! {
            () = cancel.cancelled() => Err(AppError::Canceled),
            guard = self.gate(name).lock_owned() => {
                if cancel.is_cancelled() {
                    Err(AppError::Canceled)
                } else {
                    Ok(ProjectGuard(guard))
                }
            }
        }
    }

    fn gate(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .inner
            .locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        locks.entry(name.to_string()).or_default().clone()
    }

    // --- events and persistence helpers ---------------------------------

    pub(crate) fn publish(&self, event: Event) {
        if let Some(bus) = &self.d().bus {
            bus.publish(event);
        }
    }

    fn publish_run(&self, r: &Run) {
        if self.d().bus.is_none() {
            return;
        }
        let mut event = blank_event(event_type::SERVICE_STATE, self.now());
        event.project.clone_from(&r.project);
        event.service.clone_from(&r.service);
        event.state = Some(r.state);
        event.run = Some(Box::new(r.clone()));
        self.publish(event);
    }

    pub(crate) fn publish_lease(&self, kind: &str, l: &Lease) {
        if self.d().bus.is_none() {
            return;
        }
        let mut event = blank_event(kind, self.now());
        event.project.clone_from(&l.project);
        event.service.clone_from(&l.service);
        event.lease = Some(l.clone());
        self.publish(event);
    }

    /// Persists a run and publishes `service.state`.
    pub(crate) fn save_run(&self, r: &Run) -> rocket_domain::ports::Result<()> {
        self.d().store.save_run(r)?;
        self.publish_run(r);
        Ok(())
    }

    /// Releases every lease of a service and publishes `port.released`.
    pub(crate) fn release_leases(&self, project: &str, service: &str) {
        let released = self
            .d()
            .store
            .release_leases(project, service)
            .unwrap_or_default();
        for l in &released {
            self.publish_lease(event_type::PORT_RELEASED, l);
        }
    }

    // --- project registry ------------------------------------------------

    /// Loads a project by absolute path (registering it) or by registered
    /// name.
    pub fn resolve_project(&self, reference: &str) -> Result<Project> {
        if reference.is_empty() {
            return Err(AppError::invalid(
                "project is required (run inside a directory with rocket.yaml or pass -p)",
            ));
        }
        if Path::new(reference).is_absolute() {
            let p = self
                .d()
                .manifests
                .load(Path::new(reference))
                .map_err(|e| AppError::invalid(e.to_string()))?;
            self.register(&p)?;
            return Ok(p);
        }
        let Some(registered) = self.d().store.get_project(reference)? else {
            return Err(AppError::not_found(format!(
                "project {reference:?} is not registered (rocket projects add <path>)"
            )));
        };
        self.d()
            .manifests
            .load(Path::new(&registered.path))
            .map_err(|e| AppError::invalid(e.to_string()))
    }

    fn register(&self, p: &Project) -> Result<()> {
        let existing = self.d().store.get_project(&p.name)?;
        if let Some(existing) = &existing {
            if existing.path == p.root {
                return Ok(());
            }
            if self.d().manifests.load(Path::new(&existing.path)).is_ok() {
                return Err(AppError::conflict(format!(
                    "project name {:?} is already registered at {} (rename it in rocket.yaml or `rocket projects rm {}`)",
                    p.name, existing.path, p.name
                )));
            }
        }
        self.d().store.upsert_project(&ProjectRef {
            name: p.name.clone(),
            path: p.root.clone(),
            added_at: self.now(),
        })?;
        Ok(())
    }

    /// Registers the project at `path`.
    pub fn add_project(&self, path: &str) -> Result<ProjectRef> {
        if !Path::new(path).is_absolute() {
            return Err(AppError::invalid("project path must be absolute"));
        }
        let p = self.resolve_project(path)?;
        self.d()
            .store
            .get_project(&p.name)?
            .ok_or_else(|| AppError::internal(format!("project {:?} was not registered", p.name)))
    }

    /// Returns the registry.
    pub fn list_projects(&self) -> Result<Vec<ProjectRef>> {
        Ok(self.d().store.list_projects()?)
    }

    /// Unregisters a project that has no active runs.
    pub fn remove_project(&self, name: &str) -> Result<()> {
        if self.d().store.get_project(name)?.is_none() {
            return Err(AppError::not_found(format!(
                "project {name:?} is not registered"
            )));
        }
        let runs = self.d().store.list_runs()?;
        if runs.iter().any(|r| r.project == name && r.state.active()) {
            return Err(AppError::conflict(format!(
                "project {name:?} has running services; run `rocket down -p {name}` first"
            )));
        }
        for r in runs.iter().filter(|r| r.project == name) {
            let _ = self.d().store.delete_run(&r.project, &r.service);
        }
        Ok(self.d().store.delete_project(name)?)
    }
}

/// An [`Event`] with only its type and time set.
pub(crate) fn blank_event(kind: &str, time: OffsetDateTime) -> Event {
    Event {
        r#type: kind.to_string(),
        time,
        project: String::new(),
        service: String::new(),
        state: None,
        line: String::new(),
        run: None,
        lease: None,
        job_id: String::new(),
        status: None,
        job: None,
    }
}
