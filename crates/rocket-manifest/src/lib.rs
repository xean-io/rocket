//! `rocket.yaml` discovery, strict parsing, validation, dotenv loading and
//! the JSON Schema: a faithful port of Go's `manifest`.
//!
//! # YAML library choice
//!
//! The Go loader leans on yaml.v3 behavior that no serde-based YAML crate
//! reproduces: `KnownFields(true)` rejection with `line N: field X not found
//! in type manifest.T` messages, yaml.v3's aggregated `TypeError`, anchors
//! and aliases (a retained `probe: *disabled` must stay an alias), explicit
//! `null` versus omission, merge keys, and custom `UnmarshalYAML` hooks that
//! inspect raw nodes. This crate therefore decodes from a node tree, not
//! through serde.
//!
//! Candidates checked on crates.io (2026-10-06):
//!
//! * `serde_yaml`: archived and deprecated (`0.9.34+deprecated`).
//! * `serde_yaml_ng` (last release 2024-05) and `serde_norway` (2024-12):
//!   forks of that archived code base, stale, and serde-shaped.
//! * `serde-saphyr` 1.3.0 (2026-09): maintained and serde-based, but its
//!   typed deserializer hides the node tree this port needs.
//! * `yaml-rust2` 0.13.0 (2026-09): maintained, but resolves aliases while
//!   loading.
//! * **`saphyr-parser` 0.1.0 (2026-09, chosen)**: the event-level YAML 1.2
//!   parser that the saphyr/yaml-rust2 line shares. It reports line numbers,
//!   anchors, tags and scalar styles per event, which is exactly what is
//!   needed to rebuild a `yaml.Node` tree with yaml.v3's plain-scalar
//!   resolution rules ([`node`]) and decoder semantics ([`decode`]).
//!
//! Known limitation: syntax errors come from saphyr, so their wording
//! (`yaml: line N: <saphyr message>`) differs from libyaml's; every
//! structural, type and validation error matches Go byte for byte.
//!
//! # Determinism
//!
//! Go iterates maps in random order when building the aggregated validation
//! error; this port iterates in sorted order so the list is stable.

mod convert;
mod decode;
mod dotenv;
mod gostd;
mod node;
mod schema;
pub mod yaml;

pub use dotenv::{EnvFiles, parse_dotenv};
pub use schema::{schema, schema_value};

use rocket_domain::Project;
use rocket_domain::ports::{self, ManifestLoader};
use std::path::{Path, PathBuf};

/// The manifest file looked up in project roots.
pub const FILE_NAME: &str = "rocket.yaml";

const ALT_FILE_NAMES: [&str; 2] = [FILE_NAME, "rocket.yml"];

/// Every problem found in a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub problems: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid rocket.yaml:\n  - {}",
            self.problems.join("\n  - ")
        )
    }
}

impl std::error::Error for ValidationError {}

/// Errors from discovery, loading and parsing. `Display` matches Go's text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Go's `ErrNoManifest`: discovery found no manifest.
    #[error("no rocket.yaml found in {start} or any parent directory")]
    NoManifest { start: String },
    /// `Load` found neither `rocket.yaml` nor `rocket.yml` in the directory.
    #[error("no rocket.yaml in {dir}")]
    NotFound { dir: String },
    /// The YAML could not be decoded; the text is everything after
    /// `parse rocket.yaml: `.
    #[error("parse rocket.yaml: {0}")]
    Parse(String),
    /// The manifest decoded but is invalid.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// A failure parsing the file at `path` (`<path>: <source>`).
    #[error("{path}: {source}")]
    File { path: String, source: Box<Error> },
    /// A filesystem failure, already phrased like Go's `*PathError`.
    #[error("{0}")]
    Io(String),
}

impl Error {
    /// Whether this is Go's `errors.Is(err, ErrNoManifest)`.
    pub fn is_no_manifest(&self) -> bool {
        matches!(self, Self::NoManifest { .. })
    }
}

impl From<Error> for ports::Error {
    fn from(err: Error) -> Self {
        Self::other(err)
    }
}

/// Phrases an `io::Error` the way Go's `os.ReadFile` does
/// (`open <path>: permission denied`).
fn go_path_error(path: &Path, err: &std::io::Error) -> Error {
    let text = err.to_string();
    let text = text
        .rfind(" (os error")
        .map_or(text.as_str(), |i| &text[..i])
        .to_string();
    let mut chars = text.chars();
    let text = chars
        .next()
        .map(|c| c.to_lowercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    let op = if err.kind() == std::io::ErrorKind::IsADirectory {
        "read"
    } else {
        "open"
    };
    Error::Io(format!("{op} {}: {text}", path.display()))
}

fn abs(path: &Path) -> Result<PathBuf, Error> {
    gostd::abs(path).map_err(|e| Error::Io(format!("getwd: {e}")))
}

/// Walks up from `start` looking for `rocket.yaml` (then `rocket.yml`) and
/// returns its directory.
pub fn find(start: &Path) -> Result<PathBuf, Error> {
    let mut dir = abs(start)?;
    loop {
        for name in ALT_FILE_NAMES {
            if std::fs::metadata(dir.join(name)).is_ok_and(|m| !m.is_dir()) {
                return Ok(dir);
            }
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => {
                return Err(Error::NoManifest {
                    start: start.display().to_string(),
                });
            }
        }
    }
}

/// Reads and validates the manifest located in `dir`.
pub fn load(dir: &Path) -> Result<Project, Error> {
    let abs = abs(dir)?;
    for name in ALT_FILE_NAMES {
        let path = abs.join(name);
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(go_path_error(&path, &e)),
        };
        return parse(&data, &abs.to_string_lossy()).map_err(|e| Error::File {
            path: path.display().to_string(),
            source: Box::new(e),
        });
    }
    Err(Error::NotFound {
        dir: abs.display().to_string(),
    })
}

/// Decodes and validates manifest bytes; `root` is the project directory.
pub fn parse(data: &[u8], root: &str) -> Result<Project, Error> {
    let decoded = decode::decode_file(data).map_err(Error::Parse)?;
    let (project, mut problems) = convert::convert(&decoded, root);
    problems.extend(convert::validate(&project, &decoded));
    if problems.is_empty() {
        Ok(project)
    } else {
        Err(ValidationError { problems }.into())
    }
}

/// Adapts [`load`] to [`ManifestLoader`].
#[derive(Debug, Default, Clone, Copy)]
pub struct Loader;

impl ManifestLoader for Loader {
    fn load(&self, dir: &Path) -> ports::Result<Project> {
        load(dir).map_err(Into::into)
    }
}
