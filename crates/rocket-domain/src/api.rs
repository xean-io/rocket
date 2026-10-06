//! Wire DTOs of the daemon HTTP API (v1): request and response bodies that are
//! not already domain types. Serde only, no I/O, so both the client and the
//! future axum server share one definition.
//!
//! Field names and order follow the Go struct tags (`internal/app`,
//! `internal/adapters/api`); `omitempty` maps to `skip_serializing_if` with
//! Go's zero-value rules. Response types ignore unknown fields (forward
//! compatible clients); request types reject them, like the Go daemon's
//! `DisallowUnknownFields` decoder. Query-string endpoints (`ps`, `logs`,
//! `jobs`, ...) have no body type; the client builds their queries.

use crate::Job;
use crate::job::{JobKind, JobStatus};
use crate::project::ProjectRef;
use crate::run::{Health, Lease, PortHolder, PortRemap, Run, RunState};
use crate::serde_util::{null_default, zero_time};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero_u16(v: &u16) -> bool {
    *v == 0
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

/// `ServiceResult.action` values.
pub mod service_action {
    pub const STARTED: &str = "started";
    pub const ALREADY_RUNNING: &str = "already_running";
    pub const FAILED: &str = "failed";
    /// A dependency failed.
    pub const SKIPPED: &str = "skipped";
}

/// `GcAction.action` values.
pub mod gc_action {
    pub const MARKED_DEAD: &str = "marked_dead";
    pub const RELEASED_LEASE: &str = "released_lease";
    pub const EXPIRED: &str = "expired";
    pub const EXPIRED_JOB: &str = "expired_job";
    pub const STOPPED_ORPHAN: &str = "stopped_orphan";
    pub const PRUNED: &str = "pruned";
    pub const LOST_JOB: &str = "lost_job";
    pub const PRUNED_JOB: &str = "pruned_job";
}

/// `Conflict.kind` values.
pub mod conflict_kind {
    pub const PORT_REMAPPED: &str = "port_remapped";
    pub const PORT_BUSY: &str = "port_busy";
}

/// `GET /v1/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct HealthInfo {
    pub ok: bool,
    pub api: String,
    pub version: String,
    pub pid: i32,
    #[serde(default = "zero_time", with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub started_at: OffsetDateTime,
    pub home: String,
    pub socket: String,
    /// The token-protected TCP listener; omitted when it could not bind.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub http: String,
}

/// Body of every non-2xx response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ErrorBody {
    #[serde(default)]
    pub error: String,
    /// `invalid` | `unauthorized` | `not_found` | `conflict` |
    /// `confirmation_required` | `internal`.
    #[serde(default)]
    pub code: String,
}

/// `POST /v1/projects`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AddProjectRequest {
    #[serde(default)]
    pub path: String,
}

/// `GET /v1/projects`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectsResult {
    #[serde(default, deserialize_with = "null_default")]
    pub projects: Vec<ProjectRef>,
}

/// `DELETE /v1/projects/{name}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RemovedProject {
    #[serde(default)]
    pub removed: String,
}

/// `POST /v1/shutdown`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ShutdownResult {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub pid: i32,
}

/// `POST /v1/up` and `POST /v1/restart`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct UpRequest {
    /// Absolute path or registered name.
    #[serde(default)]
    pub project: String,
    /// Services or groups; empty means everything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    /// Defaults to the project's default env.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    /// Extra profiles enabled for wildcard selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// Defaults to `user`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    /// Go duration, e.g. `30m`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ttl: String,
}

/// Outcome of starting one service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ServiceResult {
    pub service: String,
    /// See [`service_action`].
    pub action: String,
    pub state: RunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pid: i32,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub ports: BTreeMap<String, u16>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub remaps: Vec<PortRemap>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// `POST /v1/up` and `POST /v1/restart` response; services in start order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct UpResult {
    pub project: String,
    pub env: String,
    #[serde(default, deserialize_with = "null_default")]
    pub services: Vec<ServiceResult>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub hints: Vec<String>,
}

/// `POST /v1/down`.
impl UpResult {
    /// Whether any service failed or was skipped (Go: `UpResult.Failed`).
    pub fn failed(&self) -> bool {
        self.services
            .iter()
            .any(|s| s.action == service_action::FAILED || s.action == service_action::SKIPPED)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DownRequest {
    /// Path or name; ignored with `everywhere`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    /// Services or groups; empty means the whole project.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    /// Only runs started by this owner.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    /// All projects.
    #[serde(default, skip_serializing_if = "is_false")]
    pub everywhere: bool,
}

/// `POST /v1/down` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DownResult {
    #[serde(default, deserialize_with = "null_default")]
    pub stopped: Vec<Run>,
    /// Compose projects taken down.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub compose_down: Vec<String>,
    /// Running jobs canceled by a whole-project/owner/everywhere `down`.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub canceled_jobs: Vec<String>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub errors: Vec<String>,
}

/// `GET /v1/ps`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct StatusResult {
    #[serde(default, deserialize_with = "null_default")]
    pub services: Vec<Run>,
}

/// `GET /v1/logs` (tail).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogsResult {
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub service: String,
    #[serde(default, deserialize_with = "null_default")]
    pub lines: Vec<String>,
}

/// One row of the global port map: a lease plus its owning run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PortInfo {
    #[serde(flatten)]
    pub lease: Lease,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<RunState>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pid: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
}

/// `GET /v1/ports`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PortsResult {
    #[serde(default, deserialize_with = "null_default")]
    pub ports: Vec<PortInfo>,
}

/// One garbage-collection step; see [`gc_action`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GcAction {
    pub action: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub service: String,
    #[serde(default, skip_serializing_if = "is_zero_u16")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// `POST /v1/gc`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GcResult {
    #[serde(default, deserialize_with = "null_default")]
    pub actions: Vec<GcAction>,
}

/// Manifest overview inside [`Summary`]. `envs`, `pipelines` and
/// `deploy_envs` are `None` when an older daemon omitted them, which differs
/// from an explicitly empty list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectInfo {
    pub name: String,
    pub root: String,
    pub default_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipelines: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy_envs: Option<Vec<String>>,
}

/// A port problem reported by `status`; see [`conflict_kind`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Conflict {
    pub kind: String,
    pub project: String,
    pub service: String,
    pub port_name: String,
    /// Remapped port, or the busy default.
    pub port: u16,
    /// Port declared in rocket.yaml.
    pub default: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    /// Whether the service declares an env var rocket can inject.
    #[serde(default)]
    pub remappable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<PortHolder>,
    #[serde(default)]
    pub detail: String,
}

/// `GET /v1/status`: one-shot overview.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Summary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectInfo>,
    #[serde(default, deserialize_with = "null_default")]
    pub services: Vec<Run>,
    /// Running jobs only.
    #[serde(default, deserialize_with = "null_default")]
    pub jobs: Vec<Job>,
    #[serde(default, deserialize_with = "null_default")]
    pub conflicts: Vec<Conflict>,
}

/// `POST /v1/jobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    #[serde(default)]
    pub project: String,
    pub kind: JobKind,
    /// Pipeline/task, setup name or deploy env.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// Positive Go duration measured from job creation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ttl: String,
    /// Appended to the last step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    /// Confirms a deploy to an env with `confirm: true`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub yes: bool,
    /// Lets an `agent:` owner deploy (CLI sets it from `ROCKET_ALLOW_DEPLOY`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_agent_deploy: bool,
}

impl Default for JobRequest {
    fn default() -> Self {
        Self {
            project: String::new(),
            kind: JobKind::Pipeline,
            name: String::new(),
            env: String::new(),
            profiles: Vec::new(),
            ttl: String::new(),
            args: Vec::new(),
            owner: String::new(),
            yes: false,
            allow_agent_deploy: false,
        }
    }
}

/// `GET /v1/jobs`, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobsResult {
    #[serde(default, deserialize_with = "null_default")]
    pub jobs: Vec<Job>,
}

/// `GET /v1/jobs/{id}/logs` (tail).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobLogsResult {
    #[serde(default)]
    pub job: String,
    #[serde(default)]
    pub project: String,
    #[serde(default, deserialize_with = "null_default")]
    pub lines: Vec<String>,
}

/// Printed by blocking `rocket run|setup|... --json` (CLI only). `exit_code`
/// is always present (`null` when unknown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct JobOutcome {
    pub job: String,
    pub project: String,
    pub kind: JobKind,
    pub name: String,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub log_path: String,
    #[serde(default, deserialize_with = "null_default")]
    pub tail: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}
