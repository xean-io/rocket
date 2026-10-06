//! One-shot jobs: pipelines, setup actions and deploys (Go: `job.go`).

use crate::project::{Step, string_enum};
use crate::run::is_zero_i32;
use crate::serde_util::{null_default, zero_time};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

string_enum! {
    /// Why a one-shot job runs.
    JobKind {
        Setup => "setup",
        Pipeline => "pipeline",
        Deploy => "deploy",
    }
}

string_enum! {
    /// Lifecycle state of a job.
    JobStatus {
        Running => "running",
        Succeeded => "succeeded",
        Failed => "failed",
        Canceled => "canceled",
        /// Its process vanished while no daemon watched it.
        Lost => "lost",
    }
}

impl JobStatus {
    /// Whether the job has finished one way or another.
    pub fn terminal(self) -> bool {
        self != Self::Running
    }
}

/// One execution of a pipeline, setup action or deploy, supervised by the
/// daemon. Steps run sequentially and stop at the first failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub project: String,
    /// Pipeline/task name, setup name or deploy env.
    pub name: String,
    pub kind: JobKind,
    /// Selected environment; deploy target for deploy jobs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    /// Effective env/request startup selection for prerequisites.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub owner: String,
    #[serde(default, deserialize_with = "null_default")]
    pub steps: Vec<Step>,
    /// Appended to the last step.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub args: Vec<String>,
    pub status: JobStatus,
    /// 1-based index of the current/last step.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub step: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pid: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pgid: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default = "zero_time", with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub finished_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

impl Job {
    /// Whether the persisted deadline has elapsed at `now`.
    pub fn expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at.is_some_and(|t| now >= t)
    }
}

/// Whether `owner` identifies an AI agent (`agent:<id>`).
pub fn is_agent_owner(owner: &str) -> bool {
    owner.starts_with("agent:")
}
