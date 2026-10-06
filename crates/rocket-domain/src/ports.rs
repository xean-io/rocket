//! Interfaces the application core depends on (Go: `internal/ports`).
//!
//! Adapters implement these traits; the composition root injects them as
//! `Arc<dyn Trait>`. See the crate docs for why the async ones use
//! `async-trait`. Cancellation: dropping a returned future cancels the call
//! (Go's `context.Context`).
//!
//! The traits mention `std::fs::File`, `std::io::Write` and `tokio::sync`
//! channel types only as parameter and return types; this crate itself never
//! performs I/O.

use crate::job::Job;
use crate::project::{HealthCheck, Project, ProjectRef};
use crate::run::{Event, Lease, PortHolder, Run};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub use async_trait::async_trait;

/// Errors returned by port implementations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Go's `ErrUnsupported`: the adapter is not implemented on this platform.
    #[error("not supported on this platform")]
    Unsupported,
    /// Go's `ErrLeaseTaken`: a port is leased by another service. The payload
    /// is the human-readable detail Go wraps around the sentinel
    /// (`port 3000 held by project/service`).
    #[error("port already leased: {0}")]
    LeaseTaken(String),
    /// Any other failure, with its message preserved.
    #[error("{0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// An [`Error::Other`] carrying only a message.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Other(message.into().into())
    }

    /// An [`Error::LeaseTaken`] naming the current holder.
    pub fn lease_taken(port: u16, project: &str, service: &str) -> Self {
        Self::LeaseTaken(format!("port {port} held by {project}/{service}"))
    }

    /// Wraps any error as [`Error::Other`].
    pub fn other(err: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Other(Box::new(err))
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::other(err)
    }
}

impl From<crate::Error> for Error {
    fn from(err: crate::Error) -> Self {
        Self::other(err)
    }
}

/// Result alias for port operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Loads a project from its root directory.
pub trait ManifestLoader: Send + Sync {
    fn load(&self, dir: &Path) -> Result<Project>;
}

/// Reads dotenv files.
pub trait EnvSource: Send + Sync {
    fn dotenv(&self, root: &Path, files: &[String]) -> Result<BTreeMap<String, String>>;
}

/// Persists the project registry, runs, port leases and jobs.
pub trait Store: Send + Sync {
    fn upsert_project(&self, project: &ProjectRef) -> Result<()>;
    fn get_project(&self, name: &str) -> Result<Option<ProjectRef>>;
    fn list_projects(&self) -> Result<Vec<ProjectRef>>;
    fn delete_project(&self, name: &str) -> Result<()>;

    fn save_run(&self, run: &Run) -> Result<()>;
    fn get_run(&self, project: &str, service: &str) -> Result<Option<Run>>;
    fn list_runs(&self) -> Result<Vec<Run>>;
    fn delete_run(&self, project: &str, service: &str) -> Result<()>;

    /// Stores a lease; returns [`Error::LeaseTaken`] when the port is leased
    /// by a different project/service.
    fn acquire_lease(&self, lease: &Lease) -> Result<()>;
    fn release_leases(&self, project: &str, service: &str) -> Result<Vec<Lease>>;
    fn list_leases(&self) -> Result<Vec<Lease>>;

    fn save_job(&self, job: &Job) -> Result<()>;
    fn get_job(&self, id: &str) -> Result<Option<Job>>;
    /// Newest first; an empty `project` means every project and `limit <= 0`
    /// means no limit.
    fn list_jobs(&self, project: &str, limit: i64) -> Result<Vec<Job>>;
    fn delete_job(&self, id: &str) -> Result<()>;
}

/// A local process to start in its own process group.
#[derive(Debug)]
pub struct ProcessSpec {
    pub argv: Vec<String>,
    pub dir: PathBuf,
    /// `KEY=value` entries.
    pub env: Vec<String>,
    /// Receives stdout and stderr.
    pub output: Option<File>,
}

/// Identifies a started process. `done` yields the exit code once.
#[derive(Debug)]
pub struct ProcessHandle {
    pub pid: i32,
    pub pgid: i32,
    pub done: oneshot::Receiver<i32>,
}

/// Starts and stops process groups.
#[async_trait]
pub trait ProcessRunner: Send + Sync {
    fn start(&self, spec: ProcessSpec) -> Result<ProcessHandle>;
    /// Sends SIGTERM to the group, waits up to `grace`, then SIGKILL.
    async fn stop(&self, pgid: i32, grace: Duration) -> Result<()>;
    /// Whether `pid` is running and still leads/belongs to `pgid`.
    fn alive(&self, pid: i32, pgid: i32) -> bool;
}

/// Addresses one compose project (rocket project + env).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposeTarget {
    pub project_name: String,
    pub dir: PathBuf,
    pub files: Vec<String>,
    pub profiles: Vec<String>,
    pub env: Vec<String>,
}

/// Wraps `docker compose`.
#[async_trait]
pub trait ComposeDriver: Send + Sync {
    /// Nonexternal named volumes mounted by one service, after Compose has
    /// normalized names and interpolated the target environment.
    async fn named_volumes(&self, t: &ComposeTarget, service: &str) -> Result<Vec<String>>;
    /// Uses a successful Docker inventory to establish exact-name presence.
    /// Errors provide no evidence that a volume is absent.
    async fn volume_exists(&self, t: &ComposeTarget, name: &str) -> Result<bool>;
    /// Starts one service and returns its container id.
    async fn up(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> Result<String>;
    async fn stop(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> Result<()>;
    async fn down(&self, t: &ComposeTarget, out: &mut (dyn Write + Send)) -> Result<()>;
    async fn running(&self, compose_project: &str, service: &str) -> Result<bool>;
    async fn logs(&self, t: &ComposeTarget, service: &str, tail: usize) -> Result<Vec<String>>;
}

/// Builds argv for go-task invocations.
pub trait TaskDriver: Send + Sync {
    fn argv(&self, task: &str, args: &[String]) -> Vec<String>;
    /// The Taskfile path in `dir`, or `None` when there is none.
    fn taskfile(&self, dir: &Path) -> Option<PathBuf>;
}

/// Inspects the host's TCP ports.
pub trait PortProbe: Send + Sync {
    fn free(&self, port: u16) -> bool;
    fn holder(&self, port: u16) -> Result<Option<PortHolder>>;
}

/// Performs a single readiness probe.
#[async_trait]
pub trait HealthProbe: Send + Sync {
    async fn check(&self, c: &HealthCheck) -> Result<()>;
}

/// A live event subscription. Dropping it unsubscribes (Go's cancel func).
pub struct Subscription {
    rx: mpsc::Receiver<Event>,
    cancel: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl Subscription {
    /// `cancel` runs exactly once, when the subscription is dropped.
    pub fn new(rx: mpsc::Receiver<Event>, cancel: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self {
            rx,
            cancel: Some(Box::new(cancel)),
        }
    }

    /// Next event, or `None` once the bus is gone.
    pub async fn recv(&mut self) -> Option<Event> {
        self.rx.recv().await
    }

    /// Non-blocking variant of [`recv`](Self::recv).
    pub fn try_recv(&mut self) -> std::result::Result<Event, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

/// Fans out daemon events to subscribers. Publishing never blocks: slow
/// subscribers may miss events.
pub trait EventBus: Send + Sync {
    fn publish(&self, event: Event);
    fn subscribe(&self, buffer: usize) -> Subscription;
}

/// Owns per-service log files and their in-memory tail.
pub trait LogSink: Send + Sync {
    /// An append-mode file for the child's stdout/stderr, plus its path; also
    /// starts following it for tail and event publishing.
    fn open(&self, project: &str, service: &str) -> Result<(File, String)>;
    fn tail(&self, project: &str, service: &str, n: usize) -> Result<Vec<String>>;

    /// [`open`](Self::open) for a job log; new lines are published as `job.log`.
    fn open_job(&self, project: &str, job_id: &str) -> Result<(File, String)>;
    fn tail_job(&self, project: &str, job_id: &str, n: usize) -> Result<Vec<String>>;
    /// Publishes the remaining lines and stops following the log.
    fn close_job(&self, project: &str, job_id: &str);
    fn remove_job(&self, project: &str, job_id: &str) -> Result<()>;
}
