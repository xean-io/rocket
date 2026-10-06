//! Projects, services and health resolution (Go: `project.go`).

use crate::serde_util::{duration_ns, null_default, zero_time};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;
use time::OffsetDateTime;

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(feature = "ts", derive(ts_rs::TS))]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $text)] $variant),+
        }

        impl $name {
            /// The exact string Go serializes.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
pub(crate) use string_enum;

string_enum! {
    /// Tells which adapter supervises a service.
    ServiceKind {
        Compose => "compose",
        Task => "task",
        Run => "run",
    }
}

impl Default for ServiceKind {
    /// Convenience for builders and tests (Go's zero value is the invalid
    /// empty string). Decoding still requires `kind` to be present.
    fn default() -> Self {
        Self::Run
    }
}

/// Used when neither `--owner` nor `ROCKET_OWNER` is set.
pub const DEFAULT_OWNER: &str = "user";

/// Bounds how long `up` waits for a service to be healthy.
pub const DEFAULT_HEALTH_TIMEOUT: Duration = Duration::from_secs(60);

fn is_false(b: &bool) -> bool {
    !*b
}

/// A named port a service listens on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSpec {
    pub name: String,
    #[serde(default)]
    pub default: u16,
    /// Scalar or first unconditional binding, for remap reporting.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub env: String,
    /// Never serialized (Go: `json:"-"`).
    #[serde(skip)]
    pub env_bindings: BTreeMap<String, PortEnvBinding>,
    /// `None` defaults to enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<bool>,
}

impl PortSpec {
    /// Whether Rocket should use this port for readiness. Disabled ports still
    /// reserve a lease and permit remapping.
    pub fn probe_enabled(&self) -> bool {
        self.probe.unwrap_or(true)
    }
}

/// Injects the resolved port into a template. Default bindings preserve
/// nonempty values from the inherited, dotenv or service environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortEnvBinding {
    pub template: String,
    pub default: bool,
}

/// Describes how to decide a service is ready.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthSpec {
    /// Path probed on the health port.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub http: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tcp: bool,
    /// Port name; defaults to the first eligible port.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub port: String,
    #[serde(
        default,
        with = "duration_ns",
        skip_serializing_if = "Duration::is_zero"
    )]
    pub timeout: Duration,
}

/// One supervised unit of a project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    #[serde(default)]
    pub name: String,
    pub kind: ServiceKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub compose: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub env: BTreeMap<String, String>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub dotenv: Vec<String>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub profiles: Vec<String>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub depends_on: Vec<String>,
    /// Sorted by name.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub ports: Vec<PortSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<HealthSpec>,
}

impl Service {
    /// The configured or default readiness timeout (non-positive means default).
    pub fn health_timeout(&self) -> Duration {
        match &self.health {
            Some(h) if !h.timeout.is_zero() => h.timeout,
            _ => DEFAULT_HEALTH_TIMEOUT,
        }
    }
}

/// One unit of a pipeline or setup action.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Step {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run: String,
}

impl Step {
    /// Renders a step for logs and CLI output.
    pub fn describe(&self) -> String {
        if self.task.is_empty() {
            format!("run {}", self.run)
        } else {
            format!("task {}", self.task)
        }
    }
}

/// An environment's deploy action (exactly one of `task` or `run`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deploy {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run: String,
    #[serde(default)]
    pub confirm: bool,
}

impl Deploy {
    /// The deploy action as a job step.
    pub fn step(&self) -> Step {
        Step {
            task: self.task.clone(),
            run: self.run.clone(),
        }
    }
}

/// Selects compose files/profiles (dev, smoke...) or a deploy target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default)]
    pub name: String,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub compose: Vec<String>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy: Option<Deploy>,
}

/// A loaded `rocket.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub root: String,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub dotenv: Vec<String>,
    #[serde(default)]
    pub default_env: String,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub setup: BTreeMap<String, Step>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub envs: BTreeMap<String, Environment>,
    #[serde(default, deserialize_with = "null_default")]
    pub services: BTreeMap<String, Service>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub groups: BTreeMap<String, Vec<String>>,
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub pipelines: BTreeMap<String, Vec<Step>>,
    /// Prerequisite service/group targets per pipeline. Never serialized.
    #[serde(skip)]
    pub pipeline_needs: BTreeMap<String, Vec<String>>,
}

/// An entry of the global project registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ProjectRef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default = "zero_time", with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub added_at: OffsetDateTime,
}

/// A resolved, single probe target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    /// `"tcp"` or `"http"`.
    pub kind: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

/// Resolves the probe for a service given its leased ports. Returns `None`
/// when the service has nothing to probe.
pub fn health_check_for(svc: &Service, ports: &BTreeMap<String, u16>) -> Option<HealthCheck> {
    let target = svc
        .health
        .as_ref()
        .map(|h| h.port.as_str())
        .unwrap_or_default();
    let spec = svc
        .ports
        .iter()
        .find(|spec| spec.probe_enabled() && (target.is_empty() || spec.name == target))?;
    let port = *ports.get(&spec.name).filter(|p| **p != 0)?;
    if let Some(h) = svc.health.as_ref().filter(|h| !h.http.is_empty()) {
        return Some(HealthCheck {
            kind: "http".into(),
            port,
            path: h.http.clone(),
        });
    }
    if svc.kind == ServiceKind::Compose && !svc.health.as_ref().is_some_and(|h| h.tcp) {
        return None; // compose health is covered by `up --wait`
    }
    Some(HealthCheck {
        kind: "tcp".into(),
        port,
        path: String::new(),
    })
}
