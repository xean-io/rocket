//! Starts and stops local process groups (Go: `adapters/process`).
//!
//! Unix uses POSIX process groups (`pgid == pid`); Windows is a stub that
//! returns [`ports::Error::Unsupported`](rocket_domain::ports::Error::Unsupported)
//! until Job Object support lands, exactly like the Go adapter.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::Runner;
#[cfg(windows)]
pub use windows::Runner;
