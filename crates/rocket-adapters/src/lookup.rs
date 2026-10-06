//! `exec.LookPath` equivalent.
//!
//! Go resolves a bare program name against the *parent's* `PATH` before the
//! child's (possibly different) environment is installed. Rust's `Command`
//! would search the child's `PATH` instead, so adapters that pass an explicit
//! environment resolve the program here first to keep Go's behaviour.

use std::path::{Path, PathBuf};

/// Resolves `name` like Go's `exec.LookPath`: names containing a path
/// separator are returned unchanged; bare names are searched in `PATH`.
pub(crate) fn look_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') || (cfg!(windows) && name.contains('\\')) {
        return Some(PathBuf::from(name));
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let dir = if dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            dir
        };
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// The error Go reports when `look_path` finds nothing.
pub(crate) fn not_found(name: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("exec: \"{name}\": executable file not found in $PATH"),
    )
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
