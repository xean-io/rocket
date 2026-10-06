//! The real `rocketd` binary: signals, logging and the one-daemon lock.
#![cfg(unix)]

mod common;

use common::{paths_in, short_tmp};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use rocket_client::DaemonInfo;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A child `rocketd` that is killed if its test fails before stopping it, so
/// no daemon outlives the run (only processes this test started).
struct Spawned(Option<Child>);

impl Spawned {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn into_child(mut self) -> Child {
        self.0.take().unwrap()
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn spawn(home: &std::path::Path) -> Spawned {
    Spawned(Some(spawn_child(home)))
}

fn spawn_child(home: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rocketd"))
        .env("ROCKET_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rocketd")
}

fn wait_ready(daemon: &mut Spawned, daemon_json: &std::path::Path) {
    let child = daemon.child();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if DaemonInfo::load(daemon_json).is_ok() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("rocketd exited early: {status}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("rocketd did not write daemon.json");
}

fn signal_and_collect(daemon: Spawned, sig: Signal) -> (std::process::ExitStatus, String) {
    let mut child = daemon.into_child();
    kill(Pid::from_raw(child.id() as i32), sig).unwrap(); // our own child
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "rocketd ignored {sig:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn sigterm_and_sigint_shut_down_cleanly_and_log_like_go() {
    for (sig, name) in [
        (Signal::SIGTERM, "terminated"),
        (Signal::SIGINT, "interrupt"),
    ] {
        let tmp = short_tmp();
        let paths = paths_in(&tmp);
        let mut child = spawn(&paths.home);
        wait_ready(&mut child, &paths.daemon_json);
        assert!(paths.socket.exists() && paths.pid_file.exists());
        let pid = std::fs::read_to_string(&paths.pid_file).unwrap();
        assert_eq!(pid, child.child().id().to_string());

        let (status, stderr) = signal_and_collect(child, sig);
        assert!(status.success(), "{sig:?}: {status} {stderr}");
        assert!(!paths.daemon_json.exists() && !paths.socket.exists() && !paths.pid_file.exists());
        // Go's `log` format: "YYYY/MM/DD HH:MM:SS message".
        for needle in [
            "reconcile: adopted=0 dead=0 released_leases=0 lost_jobs=0",
            "rocketd ",
            " listening on ",
            &format!("received {name}, shutting down"),
            "rocketd stopped; supervised services keep running and will be adopted on next start",
        ] {
            assert!(
                stderr.contains(needle),
                "{sig:?}: missing {needle:?} in:\n{stderr}"
            );
        }
        let first = stderr.lines().next().unwrap();
        let b = first.as_bytes();
        assert!(
            b.len() > 20
                && b[4] == b'/'
                && b[7] == b'/'
                && b[10] == b' '
                && b[13] == b':'
                && b[16] == b':'
                && b[19] == b' ',
            "log line is not in Go's log format: {first:?}"
        );
    }
}

#[test]
fn a_second_rocketd_exits_with_an_error_and_does_not_disturb_the_first() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let mut first = spawn(&paths.home);
    wait_ready(&mut first, &paths.daemon_json);

    let second = spawn(&paths.home).into_child();
    let out = second.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!(
            "another rocketd holds {}: resource temporarily unavailable",
            paths.lock_file.display()
        )),
        "{stderr}"
    );
    assert!(paths.daemon_json.exists() && paths.socket.exists());

    let (status, _) = signal_and_collect(first, Signal::SIGTERM);
    assert!(status.success());
}

#[test]
fn sighup_is_ignored() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let mut child = spawn(&paths.home);
    wait_ready(&mut child, &paths.daemon_json);
    kill(Pid::from_raw(child.child().id() as i32), Signal::SIGHUP).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.child().try_wait().unwrap().is_none(),
        "SIGHUP must not stop the daemon"
    );
    let (status, _) = signal_and_collect(child, Signal::SIGTERM);
    assert!(status.success());
}
