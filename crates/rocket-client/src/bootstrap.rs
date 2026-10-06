//! Locating the rocket binary and starting the daemon (Go:
//! `client.Ensure` / `client.StartDetached`, plus the Swift `DaemonLauncher`
//! search order for GUI apps that inherit a minimal PATH).

use crate::client::{Client, TransportKind};
use crate::daemon_info::DaemonInfo;
use crate::error::ClientError;
use crate::paths::Paths;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
const BIN_NAME: &str = "rocket.exe";
#[cfg(not(windows))]
const BIN_NAME: &str = "rocket";

/// Candidate rocket binaries, best first: daemon.json's `rocket_bin`,
/// `$ROCKET_BIN`, every `$PATH` entry, then common install locations (GUI
/// apps get a minimal PATH). Pure: nothing is checked on disk.
pub fn candidate_rocket_bins(
    info_bin: Option<&Path>,
    rocket_bin_env: Option<&OsStr>,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    out.extend(
        info_bin
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf),
    );
    out.extend(rocket_bin_env.filter(|p| !p.is_empty()).map(PathBuf::from));
    if let Some(path) = path_var {
        out.extend(
            std::env::split_paths(path)
                .filter(|d| !d.as_os_str().is_empty())
                .map(|d| d.join(BIN_NAME)),
        );
    }
    let mut extra: Vec<PathBuf> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];
    if let Some(h) = home {
        extra.extend(["go/bin", ".local/bin", "bin"].map(|d| h.join(d)));
    }
    out.extend(extra.into_iter().map(|d| d.join(BIN_NAME)));
    out
}

/// The first candidate that is an executable file.
pub fn find_rocket_bin_in(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| is_executable(p)).cloned()
}

/// [`candidate_rocket_bins`] over the process environment, then the first
/// executable one. `info` is the current daemon.json, when readable.
pub fn find_rocket_bin(info: Option<&DaemonInfo>) -> Option<PathBuf> {
    let info_bin = info.map(|i| PathBuf::from(&i.rocket_bin));
    let candidates = candidate_rocket_bins(
        info_bin.as_deref(),
        std::env::var_os("ROCKET_BIN").as_deref(),
        std::env::var_os("PATH").as_deref(),
        std::env::home_dir().as_deref(),
    );
    find_rocket_bin_in(&candidates)
}

fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
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

/// Options of [`ensure_daemon`].
#[derive(Debug, Clone)]
pub struct EnsureOptions {
    pub paths: Paths,
    pub transport: TransportKind,
    /// Binary to launch; `None` runs [`find_rocket_bin`]. Tests inject a
    /// script here.
    pub rocket_bin: Option<PathBuf>,
    /// How long to wait for the daemon to answer (Go: 10s).
    pub timeout: Duration,
    /// Arguments of the launch command (Go: `daemon run`).
    pub launch_args: Vec<OsString>,
}

impl EnsureOptions {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            transport: TransportKind::default(),
            rocket_bin: None,
            timeout: Duration::from_secs(10),
            launch_args: vec!["daemon".into(), "run".into()],
        }
    }
}

async fn running_client(opts: &EnsureOptions) -> Option<Client> {
    let client = Client::from_paths(&opts.paths, opts.transport).ok()?;
    client.is_running().await.map(|_| client)
}

/// Returns a client to a running daemon, starting one detached when none
/// answers: `<rocket> daemon run` in its own session with `ROCKET_HOME` set
/// and stdout/stderr appended to `rocketd.log`, then polling health with
/// backoff until [`EnsureOptions::timeout`].
pub async fn ensure_daemon(opts: &EnsureOptions) -> Result<Client, ClientError> {
    if let Some(c) = running_client(opts).await {
        return Ok(c);
    }
    let bin = match &opts.rocket_bin {
        Some(b) => b.clone(),
        None => find_rocket_bin(DaemonInfo::load(&opts.paths.daemon_json).ok().as_ref())
            .ok_or_else(|| {
                ClientError::Launch(
                    "the rocket binary was not found; install it, put it on PATH or set ROCKET_BIN"
                        .into(),
                )
            })?,
    };
    let mut child = spawn_detached(opts, &bin)?;

    let started = Instant::now();
    let mut delay = Duration::from_millis(50);
    loop {
        if let Ok(Some(status)) = child.try_wait()
            && !status.success()
        {
            return Err(ClientError::Launch(format!(
                "{} {} exited early ({status}); see {}",
                bin.display(),
                join_args(&opts.launch_args),
                opts.paths.daemon_log.display()
            )));
        }
        if let Some(c) = running_client(opts).await {
            reap(child);
            return Ok(c);
        }
        if started.elapsed() >= opts.timeout {
            reap(child);
            return Err(ClientError::DaemonStartTimeout {
                waited: started.elapsed(),
                log: opts.paths.daemon_log.clone(),
            });
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 3 / 2).min(Duration::from_millis(250));
    }
}

fn join_args(args: &[OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Waits for a launched child on a helper thread so it never lingers as a
/// zombie if it exits while this process keeps running.
fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

fn spawn_detached(opts: &EnsureOptions, bin: &Path) -> Result<std::process::Child, ClientError> {
    opts.paths.ensure()?;
    let log = open_log(&opts.paths.daemon_log)?;
    let mut cmd = Command::new(bin);
    cmd.args(&opts.launch_args)
        .env("ROCKET_HOME", &opts.paths.home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    detach(&mut cmd);
    cmd.spawn()
        .map_err(|e| ClientError::Launch(format!("start rocketd ({}): {e}", bin.display())))
}

#[cfg(unix)]
fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// Starts the daemon in a new session so it survives the caller's terminal
/// and is not part of its process group (Go: `Setsid`).
#[cfg(unix)]
#[allow(unsafe_code)] // SAFETY: only async-signal-safe `setsid(2)` runs between fork and exec.
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure calls nothing but setsid(2), which is
    // async-signal-safe, and touches no memory shared with the parent.
    unsafe {
        cmd.pre_exec(|| {
            nix::unistd::setsid()
                .map(drop)
                .map_err(std::io::Error::from)
        });
    }
}

/// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, as the Go client does.
#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0000_0008 | 0x0000_0200);
}
