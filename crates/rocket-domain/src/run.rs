//! Runs, leases, events and process environment helpers (Go: `run.go`).

use crate::job::{Job, JobStatus};
use crate::project::{ServiceKind, string_enum};
use crate::serde_util::{null_default, zero_time};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

string_enum! {
    /// Lifecycle state of a service instance.
    RunState {
        Starting => "starting",
        Running => "running",
        Stopping => "stopping",
        Stopped => "stopped",
        Exited => "exited",
        Failed => "failed",
        /// Found gone during reconcile.
        Dead => "dead",
    }
}

impl RunState {
    /// Whether the run may still own processes or ports.
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}

string_enum! {
    /// Last known readiness of a run.
    Health {
        Unknown => "unknown",
        Healthy => "healthy",
        Unhealthy => "unhealthy",
    }
}

/// The current (or last) instance of a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub project: String,
    pub service: String,
    pub env: String,
    pub kind: ServiceKind,
    pub state: RunState,
    pub health: Health,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pid: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pgid: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub compose_project: String,
    /// Effective Compose launch profiles; retained for stop.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub container_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
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
    pub started_at: Option<OffsetDateTime>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub stopped_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub ports: BTreeMap<String, u16>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

pub(crate) fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

impl Run {
    /// Whether the run's TTL has elapsed at `now`.
    pub fn expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at.is_some_and(|t| now >= t)
    }
}

/// Reserves a host port for one service port across all projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub port: u16,
    pub project: String,
    pub service: String,
    pub port_name: String,
    #[serde(default = "zero_time", with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// A foreign process listening on a port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortHolder {
    pub pid: i32,
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
}

/// An automatic port reassignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRemap {
    pub name: String,
    pub from: u16,
    pub to: u16,
    pub env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<PortHolder>,
    pub reason: String,
}

/// Event types published on the daemon event stream.
pub mod event_type {
    pub const SERVICE_STATE: &str = "service.state";
    pub const LOG_LINE: &str = "log.line";
    pub const PORT_LEASED: &str = "port.leased";
    pub const PORT_RELEASED: &str = "port.released";
    pub const JOB_STATE: &str = "job.state";
    pub const JOB_LOG: &str = "job.log";
}

/// A daemon notification (SSE payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub r#type: String,
    #[serde(default = "zero_time", with = "time::serde::rfc3339")]
    pub time: OffsetDateTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub service: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<RunState>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Box<Run>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<Lease>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub job_id: String,
    /// `job.state` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<JobStatus>,
    /// `job.state` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<Box<Job>>,
}

/// Ports to try when the desired one is busy: first a +100 jump
/// (3000 -> 3100), then +1 steps from there, wrapping past 65535 into 1024+.
pub fn remap_candidates(port: u16) -> Vec<u16> {
    const ATTEMPTS: u32 = 200;
    (0..ATTEMPTS)
        .map(|i| {
            let mut c = u32::from(port) + 100 + i;
            if c > 65535 {
                c = 1024 + (c - 65535);
            }
            c as u16
        })
        .collect()
}

/// Forces a unique compose project per rocket project/env: lowercased,
/// every run of characters outside `[a-z0-9_-]` becomes one `-`, and leading
/// and trailing `-` are trimmed.
pub fn compose_project_name(project: &str, env: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for c in lower_per_code_point(&format!("rocket-{project}-{env}")) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Go's `strings.ToLower` maps one code point to one code point; Rust's
/// `str::to_lowercase` applies full case mapping (e.g. U+0130 becomes two
/// code points, final sigma is contextual). Mirror Go.
fn lower_per_code_point(s: &str) -> Vec<char> {
    s.chars()
        .flat_map(|c| {
            if c == '\u{130}' {
                vec!['i']
            } else {
                c.to_lowercase().collect()
            }
        })
        .collect()
}

/// Merges env layers for a child process. Later layers win. `ROCKET_*`
/// variables from the base environment never leak into children. The result
/// is `KEY=value` pairs sorted by key.
pub fn build_env(base: &[String], layers: &[&BTreeMap<String, String>]) -> Vec<String> {
    let mut m: BTreeMap<&str, &str> = BTreeMap::new();
    for kv in base {
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        if k.starts_with("ROCKET_") {
            continue;
        }
        m.insert(k, v);
    }
    for layer in layers {
        for (k, v) in layer.iter() {
            m.insert(k, v);
        }
    }
    m.into_iter().map(|(k, v)| format!("{k}={v}")).collect()
}
