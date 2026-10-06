//! Payloads that exist only on the desktop bridge (not daemon wire types).

use rocket_domain::Event;
use rocket_domain::api::HealthInfo;
use serde::Serialize;
use ts_rs::TS;

/// Link between the app and rocketd, as emitted on `rocket://connection`
/// and returned by `connection_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Locating, starting or reconnecting to the daemon.
    Connecting,
    /// The event stream is open.
    Online { version: String, pid: i32 },
    /// The last attempt failed; the supervisor retries with backoff.
    Offline { reason: String },
}

/// One message of a log-follow channel.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FollowMessage {
    /// A `log.line`, `job.log` or `job.state` event.
    Event { event: Box<Event> },
    /// The stream ended (job finished or the daemon closed it).
    End,
    /// The stream broke; no further messages follow.
    Error { message: String },
}

/// What Settings shows about the daemon. Never carries the TCP token.
#[derive(Debug, Clone, Serialize, TS)]
pub struct DaemonDetails {
    pub running: bool,
    pub home: String,
    pub socket: String,
    pub db: String,
    pub logs: String,
    pub daemon_log: String,
    pub daemon_json: String,
    /// The rocket binary the app would launch, when one is found.
    pub rocket_bin: Option<String>,
    pub health: Option<HealthInfo>,
}
