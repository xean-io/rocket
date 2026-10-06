//! Shared harness for the end-to-end suite: a throwaway `ROCKET_HOME`, a copy of
//! `testdata/fixture`, and helpers that drive the built `rocket` binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Ports the fixture manifests bind (18531 is the first remap candidate).
pub const FIXTURE_PORTS: [u16; 4] = [18431, 18432, 18433, 18531];

/// Every test binds the same fixture ports, so tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

pub struct Env {
    pub home: PathBuf,
    pub project: PathBuf,
    bin: PathBuf,
    // Field order matters: the temp dir is removed before the lock is released.
    _tmp: tempfile::TempDir,
    _lock: MutexGuard<'static, ()>,
}

pub fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

/// Returns `None` (a skip) when python3 is missing or a fixture port is busy,
/// mirroring the Go suite.
pub fn setup() -> Option<Env> {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 not installed");
        return None;
    }
    let lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for p in FIXTURE_PORTS {
        if !port_free(p) {
            eprintln!("skipped: fixture port {p} is busy on this host");
            return None;
        }
    }
    // Short home: unix socket paths are limited to ~104 bytes.
    let tmp = tempfile::Builder::new()
        .prefix("rk")
        .tempdir_in("/tmp")
        .expect("create short temp home under /tmp");
    let home = tmp.path().to_path_buf();
    let project = home.join("fixture");
    std::fs::create_dir_all(&project).unwrap();
    for f in ["rocket.yaml", ".env"] {
        std::fs::copy(testdata().join("fixture").join(f), project.join(f)).unwrap();
    }
    Some(Env {
        home,
        project,
        bin: PathBuf::from(env!("CARGO_BIN_EXE_rocket")),
        _tmp: tmp,
        _lock: lock,
    })
}

impl Env {
    /// Runs the CLI in the fixture dir and returns (stdout, exit code).
    pub fn rocket(&self, extra: &[(&str, &str)], args: &[&str]) -> (String, i32) {
        self.rocket_at(&self.project, extra, args)
    }

    /// Runs the CLI in `dir`; later `extra` entries override earlier ones.
    pub fn rocket_at(&self, dir: &Path, extra: &[(&str, &str)], args: &[&str]) -> (String, i32) {
        let out = self.spawn(dir, extra, args).expect("run rocket");
        if !out.stderr.is_empty() {
            eprintln!(
                "rocket {args:?} stderr: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn spawn(
        &self,
        dir: &Path,
        extra: &[(&str, &str)],
        args: &[&str],
    ) -> std::io::Result<std::process::Output> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .current_dir(dir)
            .env_clear()
            .stdin(Stdio::null());
        // Inherited ROCKET_* / FIXTURE_* must not leak into the daemon under test.
        for (k, v) in std::env::vars_os() {
            let name = k.to_string_lossy();
            if !name.starts_with("ROCKET_") && !name.starts_with("FIXTURE_") {
                cmd.env(k, v);
            }
        }
        cmd.env("ROCKET_HOME", &self.home);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output()
    }

    /// Runs `args --json`, asserts the exit code and parses stdout.
    pub fn json(&self, extra: &[(&str, &str)], want: i32, args: &[&str]) -> Value {
        self.json_at(&self.project, extra, want, args)
    }

    pub fn json_at(&self, dir: &Path, extra: &[(&str, &str)], want: i32, args: &[&str]) -> Value {
        let mut a = args.to_vec();
        a.push("--json");
        let (out, code) = self.rocket_at(dir, extra, &a);
        assert_eq!(
            code, want,
            "rocket {args:?}: exit {code} (want {want})\n{out}"
        );
        serde_json::from_str(&out)
            .unwrap_or_else(|e| panic!("rocket {args:?}: bad json: {e}\n{out}"))
    }

    /// `rocket ps` keyed by service name.
    pub fn ps(&self) -> serde_json::Map<String, Value> {
        let res = self.json(&[], 0, &["ps"]);
        arr(&res, "services")
            .into_iter()
            .map(|r| (str_of(&r, "service").to_string(), r))
            .collect()
    }

    pub fn wait_job_pgid(&self, id: &str) -> Value {
        let mut job = Value::Null;
        wait_for(Duration::from_secs(10), "job process", || {
            job = self.json(&[], 0, &["job", id]);
            job["pgid"].as_i64().unwrap_or(0) > 0
        });
        job
    }

    pub fn wait_expired_job(&self, id: &str) -> Value {
        let mut job = Value::Null;
        wait_for(Duration::from_secs(8), "job TTL expiry", || {
            job = self.json(&[], 0, &["job", id]);
            is_terminal(str_of(&job, "status"))
        });
        assert!(
            job["status"] == "canceled"
                && job["error"] == "ttl expired"
                && !job["expires_at"].is_null()
                && !job["finished_at"].is_null(),
            "expired job: {job}"
        );
        job
    }

    /// Writes `rocket.yaml` in the fixture project.
    pub fn write_manifest(&self, yaml: &str) {
        std::fs::write(self.project.join("rocket.yaml"), yaml).unwrap();
    }

    /// Daemon HTTP address (`host:port`) and token from `daemon.json`.
    pub fn daemon_info(&self) -> Value {
        let data = std::fs::read_to_string(self.home.join("daemon.json")).expect("daemon.json");
        serde_json::from_str(&data).expect("daemon.json is JSON")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // Runs even when the test failed: never leak daemons or fixture servers.
        let _ = self.spawn(&self.project, &[], &["down", "--everywhere"]);
        let _ = self.spawn(&self.project, &[], &["daemon", "stop"]);
        if !std::thread::panicking() {
            for p in FIXTURE_PORTS {
                assert!(port_free(p), "fixture port {p} still busy after cleanup");
            }
        }
    }
}

pub fn step(name: &str) {
    eprintln!("== {name}");
}

pub fn wait_for(timeout: Duration, what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for {what}");
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "canceled" | "lost")
}

pub fn arr(v: &Value, key: &str) -> Vec<Value> {
    v[key].as_array().cloned().unwrap_or_default()
}

pub fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

pub fn strings(v: &Value, key: &str) -> Vec<String> {
    arr(v, key)
        .iter()
        .map(|s| s.as_str().unwrap_or("").to_string())
        .collect()
}

pub fn tail_has(lines: &[String], needle: &str) -> bool {
    lines.join("\n").contains(needle)
}

pub fn tail_of(v: &Value, key: &str) -> Vec<String> {
    strings(v, key)
}

pub fn parse_time(v: &Value, key: &str) -> OffsetDateTime {
    OffsetDateTime::parse(str_of(v, key), &Rfc3339)
        .unwrap_or_else(|e| panic!("bad {key} in {v}: {e}"))
}

/// Minimal HTTP/1.0 GET (the server closes the connection at end of body, so no
/// chunked decoding is needed). Returns the buffered reader positioned after the
/// headers and the status code.
pub fn http_get(
    addr: &str,
    path: &str,
    token: Option<&str>,
    read_timeout: Duration,
) -> std::io::Result<(u16, BufReader<TcpStream>)> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(read_timeout))?;
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    write!(stream, "GET {path} HTTP/1.0\r\nHost: {addr}\r\n{auth}\r\n")?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    Ok((code, reader))
}

pub fn http_ok(port: u16) -> bool {
    let Ok(addr) = format!("127.0.0.1:{port}").parse::<std::net::SocketAddr>() else {
        return false;
    };
    let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else {
        return false;
    };
    drop(stream);
    match http_get(&addr.to_string(), "/", None, Duration::from_secs(1)) {
        Ok((code, mut r)) => {
            let mut sink = Vec::new();
            let _ = r.read_to_end(&mut sink);
            code == 200
        }
        Err(_) => false,
    }
}

/// True when the process group no longer exists.
pub fn group_gone(pgid: i64) -> bool {
    kill(Pid::from_raw(-(pgid as i32)), None) == Err(Errno::ESRCH)
}

pub fn sigkill(pid: i32) {
    let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
}

pub fn sigkill_group(pgid: i64) {
    let _ = kill(Pid::from_raw(-(pgid as i32)), Signal::SIGKILL);
}

/// Copies a directory tree.
pub fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}
