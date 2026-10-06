//! rocketd's composition root (Go: `daemon`): wires the adapters
//! into the application core, serves the API on the unix socket (mode 0600,
//! token-free) and on a token-protected loopback TCP listener, publishes
//! `daemon.json`, and runs until a signal or `POST /v1/shutdown`.
//!
//! ```ignore
//! let paths = rocket_client::Paths::resolve()?;
//! rocket_daemon::run(rocket_daemon::RunOptions::for_process(paths, "0.1.0")).await?;
//! ```
//!
//! Differences from Go: log timestamps are UTC unless the binary called
//! [`init_local_offset`] before starting its runtime; serving stops
//! gracefully for at most 5 seconds and open SSE streams are closed instead
//! of being left to the process exit.

#[cfg(unix)]
mod lock;
mod log;
#[cfg(unix)]
mod token;

pub use log::init_local_offset;
use rocket_client::Paths;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// How often persisted service and job deadlines are checked.
pub const TTL_INTERVAL: Duration = Duration::from_secs(5);

/// How long in-flight requests get to finish on shutdown.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Everything [`run`] needs.
pub struct RunOptions {
    /// The `$ROCKET_HOME` layout.
    pub paths: Paths,
    /// Reported by `/v1/health` and `daemon.json`.
    pub version: String,
    /// Cancelling it stops the daemon, like SIGTERM (Go: the context).
    pub stop: CancellationToken,
    /// Handle SIGINT/SIGTERM (graceful stop) and ignore SIGHUP. The
    /// standalone binary and `rocket daemon run` set it; embedders and tests
    /// drive [`RunOptions::stop`] instead.
    pub handle_signals: bool,
    /// TTL check period (default [`TTL_INTERVAL`]).
    pub ttl_interval: Duration,
}

impl RunOptions {
    /// Defaults for embedding: no signal handlers, a fresh stop token.
    pub fn new(paths: Paths, version: impl Into<String>) -> Self {
        Self {
            paths,
            version: version.into(),
            stop: CancellationToken::new(),
            handle_signals: false,
            ttl_interval: TTL_INTERVAL,
        }
    }

    /// Defaults for a daemon process: [`RunOptions::new`] plus signal handling.
    pub fn for_process(paths: Paths, version: impl Into<String>) -> Self {
        Self {
            handle_signals: true,
            ..Self::new(paths, version)
        }
    }
}

#[cfg(unix)]
mod serve;
#[cfg(unix)]
pub use serve::run;

/// The daemon needs unix sockets and `flock`.
#[cfg(not(unix))]
pub async fn run(_opts: RunOptions) -> anyhow::Result<()> {
    anyhow::bail!("rocketd is not supported on this platform yet")
}
