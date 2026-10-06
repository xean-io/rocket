//! `$ROCKET_HOME/daemon.json` (Go: `daemon.Info`).

use crate::error::ClientError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::OffsetDateTime;

fn zero_time() -> OffsetDateTime {
    time::macros::datetime!(0001-01-01 00:00:00 UTC)
}

/// How GUI clients reach the daemon's token-protected TCP listener. The file
/// exists only while the daemon runs. Parsing is lenient (every field has a
/// default) like the Swift loader; callers needing TCP check `http`/`token`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub pid: i32,
    #[serde(default)]
    pub socket: String,
    /// e.g. `http://127.0.0.1:53124`
    #[serde(default)]
    pub http: String,
    /// 64 hex chars, new on every daemon start.
    #[serde(default)]
    pub token: String,
    /// Absolute path of the running rocket binary.
    #[serde(default)]
    pub rocket_bin: String,
    #[serde(default = "zero_time", with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

impl DaemonInfo {
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    /// Reads and parses `daemon.json`.
    pub fn load(path: &Path) -> Result<Self, ClientError> {
        let err = |reason: String| ClientError::DaemonInfo {
            path: path.to_path_buf(),
            reason,
        };
        let data = std::fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => err("not found (is the daemon running?)".into()),
            _ => err(e.to_string()),
        })?;
        Self::parse(&data).map_err(|e| err(format!("malformed: {e}")))
    }

    /// Atomically writes the file with mode 0600 (temp file + rename), like
    /// the Go daemon. Two-space indent and a trailing newline.
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let mut data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        data.push(b'\n');
        let dir = path.parent().unwrap_or(Path::new("."));
        let tmp = dir.join(format!(
            ".daemon-{}-{}.json",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let result = (|| {
            let mut f = open_private(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}
