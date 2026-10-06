//! Port of `adapters/task/{task_test,task_integration_test}.go`.

use rocket_adapters::task::Driver;
use rocket_domain::ports::TaskDriver;
#[cfg(unix)]
use {
    rocket_adapters::process::Runner,
    rocket_domain::ports::{ProcessRunner, ProcessSpec},
    std::time::Duration,
};

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

#[test]
fn argv() {
    let d = Driver::new();
    assert_eq!(d.argv("dev:causation", &[]), ["task", "dev:causation"]);
    let d = Driver::with_bin("/opt/task");
    assert_eq!(
        d.argv("test", &strings(&["-run", "X"])),
        ["/opt/task", "test", "--", "-run", "X"]
    );
}

#[test]
fn taskfile_detection() {
    let dir = tempfile::tempdir().unwrap();
    let d = Driver::new();
    assert_eq!(d.taskfile(dir.path()), None);
    std::fs::write(dir.path().join("Taskfile.yaml"), "version: '3'\n").unwrap();
    assert_eq!(
        d.taskfile(dir.path()),
        Some(dir.path().join("Taskfile.yaml"))
    );
}

#[test]
fn taskfile_prefers_go_task_order_and_skips_directories() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Taskfile.yml")).unwrap(); // a directory is not a Taskfile
    std::fs::write(dir.path().join("taskfile.dist.yaml"), "").unwrap();
    std::fs::write(dir.path().join("Taskfile.dist.yml"), "").unwrap();
    assert_eq!(
        Driver::new().taskfile(dir.path()),
        Some(dir.path().join("Taskfile.dist.yml"))
    );
}

#[cfg(unix)]
async fn run_and_read(argv: Vec<String>, dir: &std::path::Path, env: Vec<String>) -> (i32, String) {
    let log = dir.join("out.log");
    let out = std::fs::File::create(&log).unwrap();
    let h = Runner
        .start(ProcessSpec {
            argv,
            dir: dir.to_path_buf(),
            env,
            output: Some(out),
        })
        .unwrap();
    let pgid = h.pgid;
    let code = tokio::time::timeout(Duration::from_secs(20), h.done)
        .await
        .expect("timeout")
        .unwrap();
    let _ = Runner.stop(pgid, Duration::from_secs(1)).await;
    (code, std::fs::read_to_string(&log).unwrap())
}

/// Runs a real `task` binary under the process runner (skipped without go-task).
#[cfg(unix)]
#[tokio::test]
async fn task_runs_under_process_runner() {
    if which("task").is_none() {
        eprintln!("skipped: go-task not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Taskfile.yml"),
        "version: '3'\ntasks:\n  hello:\n    cmds:\n      - echo hello-$NAME\n",
    )
    .unwrap();
    let mut env: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    env.push("NAME=rocket".into());
    let (code, out) = run_and_read(Driver::new().argv("hello", &[]), dir.path(), env).await;
    assert_eq!(code, 0);
    assert!(out.contains("hello-rocket"), "output {out:?}");
}

/// Same wiring with a stand-in `task` executable, so it runs everywhere.
#[cfg(unix)]
#[tokio::test]
async fn task_argv_runs_under_process_runner_with_a_fixture_binary() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("task-fixture");
    std::fs::write(&bin, "#!/bin/sh\necho \"argv:$*\" \"name:$NAME\"\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let d = Driver::with_bin(bin.to_str().unwrap());
    let (code, out) = run_and_read(
        d.argv("hello", &strings(&["a", "b"])),
        dir.path(),
        vec!["NAME=rocket".into(), "PATH=/usr/bin:/bin".into()],
    )
    .await;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "argv:hello -- a b name:rocket");
}

#[cfg(unix)]
fn which(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(bin))
            .find(|c| c.is_file())
    })
}
