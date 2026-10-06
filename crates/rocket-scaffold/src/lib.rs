//! `rocket init`: detects a project's Taskfile, compose files and `.env` and
//! renders a starting `rocket.yaml` (Go: `scaffold`). Detection only
//! parses YAML; it never runs `task` or `docker`.
//!
//! The rendered text and the JSON shape of [`Detected`] are byte-identical to
//! the Go implementation; `tests/golden` and `tests/cases` hold the output
//! captured from it.

mod compose;
mod gopath;
mod render;
mod taskfile;

pub use compose::parse_short_port;
pub use gopath::{abs, clean};
pub use taskfile::is_long_running;

use rocket_adapters::task::Driver;
use rocket_domain::ports::TaskDriver;
use serde::Serialize;
use std::path::Path;

/// A published compose port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Port {
    pub name: String,
    pub default: i64,
    /// From `${VAR:-default}`; empty means not remappable.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub env: String,
    /// Container port.
    pub target: i64,
}

/// A guessed rocket service.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Service {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub compose: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub task: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
}

/// A guessed compose environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Env {
    pub name: String,
    pub files: Vec<String>,
}

/// Maps a setup action to a Taskfile task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupStep {
    pub name: String,
    pub task: String,
}

/// A Taskfile task (including included namespaces).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Task {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
}

/// Everything [`detect`] found (Go: `scaffold.Result`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Detected {
    pub name: String,
    pub root: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dotenv: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub taskfile: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub compose_files: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub envs: Vec<Env>,
    pub services: Vec<Service>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<SetupStep>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pipelines: Vec<String>,
    /// Left out of `pipelines` on purpose.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deploy_tasks: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<Task>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A filesystem failure, phrased like Go's `*PathError`.
    #[error("{0}")]
    Io(String),
    /// A YAML file could not be read or decoded; the text is
    /// `<file>: <reason>`.
    #[error("{0}")]
    Yaml(String),
}

/// Inspects `root`.
pub fn detect(root: &Path) -> Result<Detected, Error> {
    let abs_root = abs(root).map_err(|e| Error::Io(format!("getwd: {e}")))?;
    let mut r = Detected {
        name: project_name(&abs_root),
        root: abs_root.display().to_string(),
        services: Vec::new(),
        ..Detected::default()
    };
    if std::fs::metadata(abs_root.join(".env")).is_ok_and(|m| !m.is_dir()) {
        r.dotenv = vec![".env".into()];
    }
    compose::detect(&mut r, &abs_root)?;
    if let Some(tf) = Driver::new().taskfile(&abs_root) {
        r.taskfile = tf
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tasks =
            taskfile::read(&tf, "", 0).map_err(|e| Error::Yaml(format!("{}: {e}", r.taskfile)))?;
        taskfile::classify(&mut r, tasks);
    }
    Ok(r)
}

/// Go's `strings.ToLower` maps one code point to one code point.
fn lower_per_code_point(s: &str) -> String {
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

fn project_name(dir: &Path) -> String {
    let base = dir
        .file_name()
        .map_or_else(|| "/".to_owned(), |n| n.to_string_lossy().into_owned());
    let mut out = String::new();
    let mut in_run = false;
    for c in lower_per_code_point(&base).chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    let n = out.trim_matches(['-', '.', '_']);
    if n.is_empty() {
        "project".into()
    } else {
        n.into()
    }
}
