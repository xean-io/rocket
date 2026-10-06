//! Daemon bootstrap: `find_rocket_bin` ordering and `ensure_daemon` with an
//! injected launcher script standing in for the rocket binary.
#![cfg(unix)]

mod common;

use axum::Router;
use axum::routing::get;
use common::*;
use rocket_client::{
    ClientError, EnsureOptions, Paths, TransportKind, candidate_rocket_bins, ensure_daemon,
    find_rocket_bin_in,
};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    write_executable(&p, &format!("#!/bin/sh\n{body}\n"));
    p
}

#[test]
fn candidate_order_prefers_installed_binaries_over_the_bundled_one() {
    let c = candidate_rocket_bins(
        Some(Path::new("/info/rocket")),
        Some(OsStr::new("/env/rocket-bin")),
        Some(OsStr::new("/p1:/p2")),
        Some(Path::new("/Users/me")),
        Some(Path::new("/App/Rocket.app/Contents/MacOS/rocket")),
    );
    let want: Vec<&str> = vec![
        "/info/rocket",
        "/env/rocket-bin",
        "/p1/rocket",
        "/p2/rocket",
        "/opt/homebrew/bin/rocket",
        "/usr/local/bin/rocket",
        "/Users/me/go/bin/rocket",
        "/Users/me/.local/bin/rocket",
        "/Users/me/bin/rocket",
        // The bundled sidecar is the last resort: an installed CLI wins.
        "/App/Rocket.app/Contents/MacOS/rocket",
    ];
    assert_eq!(
        c.iter().map(|p| p.to_str().unwrap()).collect::<Vec<_>>(),
        want
    );
    // Missing pieces are skipped, not replaced.
    let c = candidate_rocket_bins(None, None, None, None, None);
    assert_eq!(c.len(), 2);
}

#[test]
fn bundled_sidecar_is_used_only_when_nothing_else_is_executable() {
    let tmp = short_tmp();
    let bundled = script(tmp.path(), "sidecar", "exit 0");
    let c = candidate_rocket_bins(
        None,
        None,
        Some(OsStr::new("/nonexistent")),
        None,
        Some(&bundled),
    );
    // Last in the order, whatever else this machine has installed.
    assert_eq!(c.last(), Some(&bundled));
    // Skipped like any other candidate while missing, used once it is the
    // only executable one.
    assert_eq!(
        find_rocket_bin_in(&[PathBuf::from("/definitely/not/here"), bundled.clone()]),
        Some(bundled.clone())
    );

    let installed_dir = tmp.path().join("installed");
    std::fs::create_dir_all(&installed_dir).unwrap();
    let installed = script(&installed_dir, "rocket", "exit 0");
    let path = std::env::join_paths([&installed_dir]).unwrap();
    let c = candidate_rocket_bins(None, None, Some(path.as_os_str()), None, Some(&bundled));
    assert_eq!(find_rocket_bin_in(&c), Some(installed));
}

#[test]
fn first_executable_candidate_wins_and_non_executables_are_skipped() {
    let tmp = short_tmp();
    let a = tmp.path().join("a");
    let b = tmp.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let not_exec = a.join("rocket");
    std::fs::write(&not_exec, "x").unwrap();
    let exec = script(&b, "rocket", "exit 0");
    let path = std::env::join_paths([&a, &b]).unwrap();
    let c = candidate_rocket_bins(None, None, Some(path.as_os_str()), None, None);
    assert_eq!(find_rocket_bin_in(&c), Some(exec.clone()));
    // daemon.json's rocket_bin outranks PATH when it is executable.
    let c = candidate_rocket_bins(
        Some(exec.as_path()),
        None,
        Some(OsStr::new("/nonexistent")),
        None,
        None,
    );
    assert_eq!(find_rocket_bin_in(&c), Some(exec));
    assert_eq!(
        find_rocket_bin_in(&[PathBuf::from("/definitely/not/here")]),
        None
    );
}

fn health_app() -> Router {
    Router::new().route("/v1/health", get(|| async { json_response(200, HEALTH) }))
}

#[tokio::test]
async fn running_daemon_is_reused_without_spawning() {
    let tmp = short_tmp();
    let paths = Paths::from_home(tmp.path()).unwrap();
    let _srv = serve_unix(health_app(), &paths.socket);
    let launcher = script(
        tmp.path(),
        "fake-rocket",
        "echo spawned > \"$ROCKET_HOME/launched\"",
    );

    let mut opts = EnsureOptions::new(paths.clone());
    opts.rocket_bin = Some(launcher);
    let client = ensure_daemon(&opts).await.unwrap();
    assert!(client.health().await.unwrap().ok);
    assert!(!paths.home.join("launched").exists());
}

#[tokio::test]
async fn unreachable_daemon_is_started_detached_then_polled() {
    let tmp = short_tmp();
    let paths = Paths::from_home(tmp.path()).unwrap();
    let launcher = script(
        tmp.path(),
        "fake-rocket",
        "echo \"$@|$ROCKET_HOME|$(ps -o pgid= -p $$ | tr -d ' ')|$$\" > \"$ROCKET_HOME/launched\"\nexec sleep 3",
    );
    // The "daemon" starts listening only once the launcher has run.
    let sock = paths.socket.clone();
    let marker = paths.home.join("launched");
    let srv = tokio::spawn(async move {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = std::fs::remove_file(&sock);
        let _h = serve_unix(health_app(), &sock);
        std::future::pending::<()>().await;
    });

    let mut opts = EnsureOptions::new(paths.clone());
    opts.rocket_bin = Some(launcher);
    opts.timeout = Duration::from_secs(10);
    let client = ensure_daemon(&opts).await.unwrap();
    assert!(client.health().await.unwrap().ok);

    let line = std::fs::read_to_string(paths.home.join("launched")).unwrap();
    let parts: Vec<_> = line.trim().split('|').collect();
    assert_eq!(parts[0], "daemon run", "mirrors Go StartDetached");
    assert_eq!(parts[1], paths.home.to_str().unwrap());
    // setsid: the child leads its own session/process group.
    assert_eq!(
        parts[2], parts[3],
        "launcher must be a session leader (pgid == pid)"
    );
    assert!(paths.daemon_log.exists(), "stdout/stderr go to rocketd.log");
    srv.abort();
}

#[tokio::test]
async fn launcher_that_exits_nonzero_fails_fast_with_log_hint() {
    let tmp = short_tmp();
    let paths = Paths::from_home(tmp.path()).unwrap();
    let launcher = script(tmp.path(), "fake-rocket", "echo boom >&2\nexit 7");
    let mut opts = EnsureOptions::new(paths.clone());
    opts.rocket_bin = Some(launcher);
    opts.timeout = Duration::from_secs(10);
    let started = std::time::Instant::now();
    let err = ensure_daemon(&opts).await.err().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    match err {
        ClientError::Launch(msg) => {
            assert!(msg.contains("exit status: 7") || msg.contains('7'), "{msg}");
            assert!(msg.contains("rocketd.log"), "{msg}");
        }
        e => panic!("{e:?}"),
    }
    assert!(
        std::fs::read_to_string(&paths.daemon_log)
            .unwrap()
            .contains("boom")
    );
}

#[tokio::test]
async fn daemon_that_never_answers_times_out() {
    let tmp = short_tmp();
    let paths = Paths::from_home(tmp.path()).unwrap();
    let launcher = script(tmp.path(), "fake-rocket", "exec sleep 2");
    let mut opts = EnsureOptions::new(paths);
    opts.rocket_bin = Some(launcher);
    opts.timeout = Duration::from_millis(600);
    match ensure_daemon(&opts).await.err().unwrap() {
        ClientError::DaemonStartTimeout { waited, .. } => {
            assert!(waited >= Duration::from_millis(600))
        }
        e => panic!("{e:?}"),
    }
}

#[tokio::test]
async fn not_bundled_binary_is_reported() {
    let tmp = short_tmp();
    let paths = Paths::from_home(tmp.path()).unwrap();
    let mut opts = EnsureOptions::new(paths);
    opts.rocket_bin = Some(tmp.path().join("no-such-rocket"));
    let err = ensure_daemon(&opts).await.err().unwrap();
    assert!(matches!(err, ClientError::Launch(_)), "{err:?}");
}

#[tokio::test]
async fn tcp_transport_waits_for_daemon_json_and_authenticates() {
    let tmp = short_tmp();
    let paths = Paths::from_home(tmp.path()).unwrap();
    let auth = Auth::new("tok");
    let (addr, _srv) = serve_tcp(auth.clone().layer(health_app())).await;
    let launcher = script(
        tmp.path(),
        "fake-rocket",
        "echo x > \"$ROCKET_HOME/launched\"\nexec sleep 3",
    );
    let djson = paths.daemon_json.clone();
    let marker = paths.home.join("launched");
    let writer = tokio::spawn(async move {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(
            &djson,
            format!(r#"{{"http":"http://{addr}","token":"tok"}}"#),
        )
        .unwrap();
    });
    let mut opts = EnsureOptions::new(paths);
    opts.transport = TransportKind::Tcp;
    opts.rocket_bin = Some(launcher);
    opts.timeout = Duration::from_secs(10);
    let client = ensure_daemon(&opts).await.unwrap();
    assert!(client.health().await.unwrap().ok);
    assert!(
        auth.seen
            .lock()
            .unwrap()
            .iter()
            .all(|h| h.as_deref() == Some("Bearer tok"))
    );
    writer.await.unwrap();
}

/// Writes an executable script from a child process. Writing it from this
/// multithreaded test process would let a concurrent fork inherit the open
/// write descriptor, and executing the script then fails with ETXTBSY.
fn write_executable(path: &std::path::Path, body: &str) {
    use std::io::Write;
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    assert!(
        child.wait().unwrap().success(),
        "writing {}",
        path.display()
    );
}
