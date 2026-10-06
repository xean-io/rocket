//! `$ROCKET_HOME` layout (Go: `internal/paths`). Pure path computation; only
//! [`Paths::ensure`] touches the filesystem.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Conservative `sun_path` limit (macOS: 104 bytes including the NUL).
pub const MAX_SOCKET_PATH: usize = 103;

/// rocket's on-disk layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub socket: PathBuf,
    pub db: PathBuf,
    pub logs: PathBuf,
    pub pid_file: PathBuf,
    pub lock_file: PathBuf,
    pub daemon_log: PathBuf,
    /// Tells GUI clients the TCP address and bearer token.
    pub daemon_json: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("cannot determine the home directory; set ROCKET_HOME")]
    NoHome,
    #[error("cannot make {0} absolute: {1}")]
    Absolute(PathBuf, std::io::Error),
    #[error(
        "socket path {0} is too long for a unix socket; set ROCKET_HOME to a shorter directory"
    )]
    SocketTooLong(PathBuf),
}

impl Paths {
    /// `$ROCKET_HOME` when set and non-empty, else `~/.rocket`.
    pub fn resolve() -> Result<Self, PathsError> {
        Self::resolve_from(
            std::env::var_os("ROCKET_HOME").as_deref(),
            std::env::home_dir().as_deref(),
        )
    }

    /// Pure form of [`Paths::resolve`] for callers (and tests) that supply
    /// the environment explicitly.
    pub fn resolve_from(
        rocket_home: Option<&OsStr>,
        user_home: Option<&Path>,
    ) -> Result<Self, PathsError> {
        match rocket_home.filter(|h| !h.is_empty()) {
            Some(h) => Self::from_home(h),
            None => Self::from_home(user_home.ok_or(PathsError::NoHome)?.join(".rocket")),
        }
    }

    /// The layout rooted at `home` (made absolute against the cwd).
    pub fn from_home(home: impl AsRef<Path>) -> Result<Self, PathsError> {
        let home = home.as_ref();
        let home =
            std::path::absolute(home).map_err(|e| PathsError::Absolute(home.to_path_buf(), e))?;
        let p = Self {
            socket: home.join("rocketd.sock"),
            db: home.join("state.db"),
            logs: home.join("logs"),
            pid_file: home.join("rocketd.pid"),
            lock_file: home.join("rocketd.lock"),
            daemon_log: home.join("rocketd.log"),
            daemon_json: home.join("daemon.json"),
            home,
        };
        if p.socket.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(PathsError::SocketTooLong(p.socket));
        }
        Ok(p)
    }

    /// Creates the home and logs directories with private permissions.
    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [&self.home, &self.logs] {
            create_private_dir(d)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}
