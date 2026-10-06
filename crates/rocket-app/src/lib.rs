//! rocket's use cases on top of the port traits (Go: `app`).
//!
//! [`App`] is built from [`Deps`] (every port injected as `Arc<dyn Trait>`)
//! and hosts: `up`/`restart`/`down`, `status`/`logs`/`ports`/`summary`,
//! project registry operations, `reconcile`/`expire_ttl`/`gc`. Job execution
//! lives in the (R6) [`jobs`] module; [`App`] already carries the state and
//! seams it needs.
//!
//! # Differences from the Go implementation
//!
//! * `context.Context` becomes a [`CancellationToken`]: the plain methods
//!   (`up`, `down`, ...) run to completion (like the daemon, which detaches
//!   operations from the client connection), while [`App::up_with`] takes a
//!   token whose cancellation fails and stops only the service being started.
//!   Dropping any returned future abandons the call.
//! * Errors are [`AppError`]; [`AppError::code`] gives the API `code`.
//! * The store, probes and process ports are synchronous, so the pure
//!   read-only use cases (`status`, `ports`, `summary`, registry) are plain
//!   functions; anything that talks to Docker or waits is `async`.
//!
//! [`CancellationToken`]: tokio_util::sync::CancellationToken

mod app;
mod down;
mod error;
mod go_duration;
pub mod jobs;
mod maintenance;
mod paths;
mod status;
mod summary;
mod up;
mod volume_hints;
mod watch;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use app::{
    App, BaseEnv, Clock, DEFAULT_HEALTH_INTERVAL, DEFAULT_POLL_INTERVAL, DEFAULT_START_GRACE,
    DEFAULT_STOP_GRACE, Deps, IdGen, Options,
};
pub use error::{AppError, Result};
pub use maintenance::ReconcileResult;
pub use status::{LogsRequest, StatusRequest};
pub use tokio_util::sync::CancellationToken;
