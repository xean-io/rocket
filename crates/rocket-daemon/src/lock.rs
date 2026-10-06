//! One daemon per `$ROCKET_HOME`: an exclusive, non-blocking `flock` on
//! `rocketd.lock` (Go: `lockFile`).

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Holds the lock until dropped (the kernel also releases it on exit).
pub(crate) struct LockGuard {
    _lock: nix::fcntl::Flock<File>,
}

/// Takes the lock or explains why not, with Go's error texts: the libc
/// `strerror` text in lowercase (`resource temporarily unavailable`; nix's
/// own descriptions differ, e.g. `try again`).
pub(crate) fn acquire(path: &Path) -> Result<LockGuard, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| lower_first(&io_reason(&e)))?;
    nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map(|lock| LockGuard { _lock: lock })
        .map_err(|(_, errno)| lower_first(&io_reason(&std::io::Error::from(errno))))
}

/// The OS reason without Rust's ` (os error N)` suffix.
fn io_reason(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}
