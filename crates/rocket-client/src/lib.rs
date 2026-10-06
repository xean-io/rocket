//! Typed async client for the rocketd HTTP/SSE API (`crates/rocket-api/README.md`).
//!
//! * [`Paths`] / [`DaemonInfo`]: the `$ROCKET_HOME` layout and `daemon.json`.
//! * [`Client`]: one typed method per endpoint over the unix socket (no
//!   token) or the token-protected TCP listener (token re-read from
//!   `daemon.json` once on a 401).
//! * [`SseParser`] / [`DaemonEvent`] / [`EventStream`]: live events, log
//!   follow and job-log follow.
//! * [`ensure_daemon`]: health check, then spawn `<rocket> daemon run`
//!   detached and poll until it answers.
//!
//! Wire DTOs live in [`rocket_domain::api`]; this crate re-exports nothing
//! from there so the server and client share a single definition.

mod bootstrap;
mod client;
mod daemon_info;
mod error;
mod events;
mod paths;
mod sse;
mod transport;

pub use bootstrap::{
    EnsureOptions, candidate_rocket_bins, ensure_daemon, find_rocket_bin, find_rocket_bin_in,
};
pub use client::{Client, EventFilter, JobsQuery, TransportKind};
pub use daemon_info::DaemonInfo;
pub use error::{ClientError, ErrorCode};
pub use events::{DaemonEvent, EventStream};
pub use paths::{MAX_SOCKET_PATH, Paths, PathsError};
pub use sse::{SseFrame, SseMessage, SseParser};
