use crate::lookup::{look_path, not_found};
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::{Pid, getpgid, getpgrp};
use rocket_domain::ports::{Error, ProcessHandle, ProcessRunner, ProcessSpec, Result, async_trait};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// Polling interval while waiting for a group to disappear (Go: 25ms).
const POLL: Duration = Duration::from_millis(25);
/// How long to wait for the group to vanish after SIGKILL.
const KILL_WAIT: Duration = Duration::from_secs(3);

/// [`ProcessRunner`] built on POSIX process groups.
#[derive(Debug, Clone, Copy, Default)]
pub struct Runner;

#[async_trait]
impl ProcessRunner for Runner {
    /// Runs `spec` in a new process group (`pgid == pid`).
    ///
    /// * An empty `env` inherits the parent environment (Go: a nil `Env`);
    ///   otherwise the child sees exactly `spec.env`.
    /// * A bare `argv[0]` is resolved against the *parent's* `PATH`, as in Go.
    /// * An empty `dir` keeps the parent's working directory.
    /// * Stdin is `/dev/null`; stdout and stderr both go to `spec.output`
    ///   (or `/dev/null`).
    ///
    /// A dedicated thread waits for the child, so it is always reaped and
    /// `done` yields the exit code (`128 + signal` when killed by a signal).
    fn start(&self, spec: ProcessSpec) -> Result<ProcessHandle> {
        let Some(program) = spec.argv.first() else {
            return Err(Error::msg("empty argv"));
        };
        let path = look_path(program).ok_or_else(|| Error::other(not_found(program)))?;
        let mut cmd = Command::new(path);
        cmd.arg0(program).args(&spec.argv[1..]);
        if !spec.dir.as_os_str().is_empty() {
            cmd.current_dir(&spec.dir);
        }
        if !spec.env.is_empty() {
            cmd.env_clear();
            for kv in &spec.env {
                if let Some((k, v)) = kv.split_once('=') {
                    cmd.env(k, v);
                }
            }
        }
        cmd.stdin(Stdio::null());
        match spec.output {
            Some(file) => {
                cmd.stdout(Stdio::from(file.try_clone()?));
                cmd.stderr(Stdio::from(file));
            }
            None => {
                cmd.stdout(Stdio::null());
                cmd.stderr(Stdio::null());
            }
        }
        cmd.process_group(0);
        let mut child = cmd.spawn()?;
        let pid = i32::try_from(child.id()).map_err(Error::other)?;
        let (tx, rx) = oneshot::channel();
        std::thread::Builder::new()
            .name(format!("rocket-wait-{pid}"))
            .spawn(move || {
                let code = child.wait().map_or(-1, exit_code);
                let _ = tx.send(code);
            })?;
        Ok(ProcessHandle {
            pid,
            pgid: pid,
            done: rx,
        })
    }

    /// SIGTERM to the whole group, wait up to `grace`, then SIGKILL.
    /// A group that no longer exists is not an error.
    async fn stop(&self, pgid: i32, grace: Duration) -> Result<()> {
        if pgid <= 1 || pgid == getpgrp().as_raw() {
            return Err(Error::msg(format!(
                "refusing to signal process group {pgid}"
            )));
        }
        let group = Pid::from_raw(-pgid);
        match kill(group, Signal::SIGTERM) {
            Err(Errno::ESRCH) => return Ok(()),
            Err(e) => return Err(Error::msg(format!("SIGTERM group {pgid}: {e}"))),
            Ok(()) => {}
        }
        if wait_gone(group, grace).await {
            return Ok(());
        }
        match kill(group, Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => {}
            Err(e) => return Err(Error::msg(format!("SIGKILL group {pgid}: {e}"))),
        }
        if !wait_gone(group, KILL_WAIT).await {
            return Err(Error::msg(format!(
                "process group {pgid} still alive after SIGKILL"
            )));
        }
        Ok(())
    }

    /// Whether `pid` exists (EPERM counts as alive) and, when `pgid > 0`,
    /// still belongs to that group.
    fn alive(&self, pid: i32, pgid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        match kill(Pid::from_raw(pid), None) {
            Ok(()) | Err(Errno::EPERM) => {}
            Err(_) => return false,
        }
        if pgid > 0 {
            return getpgid(Some(Pid::from_raw(pid))).is_ok_and(|got| got.as_raw() == pgid);
        }
        true
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    match status.signal() {
        Some(sig) => 128 + sig,
        None => status.code().unwrap_or(-1),
    }
}

/// True once no process remains in `group` (`kill(-pgid, 0)` gives ESRCH).
async fn wait_gone(group: Pid, d: Duration) -> bool {
    let deadline = Instant::now() + d;
    loop {
        if kill(group, None) == Err(Errno::ESRCH) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}
