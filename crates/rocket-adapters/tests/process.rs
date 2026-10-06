//! Port of `adapters/process/process_integration_test.go`.
//! Only sh is needed; every test signals just the groups it started.
#![cfg(unix)]

use nix::sys::signal::{Signal, kill};
use nix::unistd::{Pid, getpgrp};
use rocket_adapters::process::Runner;
use rocket_domain::ports::{Error, ProcessHandle, ProcessRunner, ProcessSpec};
use std::time::{Duration, Instant};

struct Started {
    h: ProcessHandle,
    log: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn start_with(script: &str, env: Vec<String>) -> Started {
    let dir = tempfile::Builder::new()
        .prefix("rkt")
        .tempdir_in("/tmp")
        .unwrap();
    let log = dir.path().join("out.log");
    let f = std::fs::File::create(&log).unwrap();
    let h = Runner
        .start(ProcessSpec {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            dir: dir.path().to_path_buf(),
            env,
            output: Some(f),
        })
        .unwrap();
    Started { h, log, _dir: dir }
}

fn start(script: &str) -> Started {
    start_with(
        script,
        vec!["GREETING=hi".into(), "PATH=/usr/bin:/bin".into()],
    )
}

impl Drop for Started {
    fn drop(&mut self) {
        // Best-effort cleanup of the group this test started.
        let _ = kill(Pid::from_raw(-self.h.pgid), Signal::SIGKILL);
    }
}

async fn done(h: &mut ProcessHandle) -> i32 {
    tokio::time::timeout(Duration::from_secs(5), &mut h.done)
        .await
        .expect("timeout")
        .unwrap()
}

#[tokio::test]
async fn exit_code_and_output() {
    let mut s = start("echo $GREETING; exit 3");
    assert_eq!(done(&mut s.h).await, 3);
    assert_eq!(std::fs::read_to_string(&s.log).unwrap().trim(), "hi");
}

#[tokio::test]
async fn stop_kills_whole_group() {
    let s = start("sleep 30 & sleep 30 & wait");
    assert_eq!(s.h.pgid, s.h.pid, "leader must own its group");
    assert!(Runner.alive(s.h.pid, s.h.pgid));
    Runner.stop(s.h.pgid, Duration::from_secs(2)).await.unwrap();
    assert!(
        kill(Pid::from_raw(-s.h.pgid), None).is_err(),
        "group still has members"
    );
    assert!(!Runner.alive(s.h.pid, s.h.pgid), "leader still alive");
}

#[tokio::test]
async fn stop_escalates_to_sigkill() {
    let mut s = start("trap '' TERM; while true; do sleep 0.1; done");
    tokio::time::sleep(Duration::from_millis(200)).await; // let the trap install
    let begin = Instant::now();
    Runner
        .stop(s.h.pgid, Duration::from_millis(300))
        .await
        .unwrap();
    assert!(
        begin.elapsed() >= Duration::from_millis(300),
        "SIGKILL sent before grace elapsed"
    );
    assert_eq!(done(&mut s.h).await, 128 + Signal::SIGKILL as i32);
}

#[tokio::test]
async fn signal_exit_is_reported_as_128_plus_signal() {
    let mut s = start("kill -TERM $$; sleep 5");
    assert_eq!(done(&mut s.h).await, 128 + Signal::SIGTERM as i32);
}

#[tokio::test]
async fn stop_refuses_own_group_and_tiny_pgids() {
    let own = getpgrp().as_raw();
    for pgid in [own, 1, 0, -5] {
        let err = Runner
            .stop(pgid, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("refusing to signal process group"),
            "{err}"
        );
    }
}

#[tokio::test]
async fn stop_tolerates_a_group_that_is_already_gone() {
    let mut s = start("exit 0");
    done(&mut s.h).await;
    Runner
        .stop(s.h.pgid, Duration::from_millis(50))
        .await
        .unwrap();
}

#[test]
fn alive_checks_pid_and_group() {
    assert!(!Runner.alive(0, 0));
    assert!(!Runner.alive(-1, 0));
    let me = std::process::id() as i32;
    assert!(Runner.alive(me, 0), "pgid 0 skips the group check");
    assert!(Runner.alive(me, getpgrp().as_raw()));
    assert!(!Runner.alive(me, getpgrp().as_raw() + 1));
}

#[tokio::test]
async fn env_is_exactly_the_spec_env_and_cwd_is_the_spec_dir() {
    let mut s = start_with(
        "echo \"$FOO|${HOME-unset}|$(pwd -P)\"",
        vec!["FOO=bar".into(), "PATH=/usr/bin:/bin".into()],
    );
    assert_eq!(done(&mut s.h).await, 0);
    let out = std::fs::read_to_string(&s.log).unwrap();
    let cwd = std::fs::canonicalize(s._dir.path()).unwrap();
    assert_eq!(out.trim(), format!("bar|unset|{}", cwd.display()));
}

#[tokio::test]
async fn resolves_argv0_against_the_parent_path_like_go() {
    // The child env has no PATH, yet `sh` is found through the parent's PATH.
    let dir = tempfile::tempdir().unwrap();
    let mut h = Runner
        .start(ProcessSpec {
            argv: vec!["sh".into(), "-c".into(), "exit 4".into()],
            dir: dir.path().to_path_buf(),
            env: vec!["ONLY=1".into()],
            output: None,
        })
        .unwrap();
    assert_eq!(done(&mut h).await, 4);
}

#[test]
fn start_rejects_empty_argv_and_missing_binaries() {
    let spec = |argv: Vec<String>| ProcessSpec {
        argv,
        dir: std::env::temp_dir(),
        env: vec![],
        output: None,
    };
    let err = Runner.start(spec(vec![])).unwrap_err();
    assert_eq!(err.to_string(), "empty argv");
    let err = Runner
        .start(spec(vec!["definitely-not-a-binary-xyz".into()]))
        .unwrap_err();
    assert!(matches!(err, Error::Other(_)), "{err}");
}

#[tokio::test]
async fn children_are_reaped_without_zombies() {
    let mut s = start("exit 0");
    let pid = s.h.pid;
    assert_eq!(done(&mut s.h).await, 0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        kill(Pid::from_raw(pid), None).is_err(),
        "pid {pid} still present (zombie?)"
    );
}
