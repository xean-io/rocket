//! Daemon lifecycle (ports of `daemon/*_test.go` plus the
//! composition-root behaviour of `daemon.Run`).
#![cfg(unix)]

mod common;

use common::{VERSION, paths_in, short_tmp, start, start_with};
use rocket_client::{Client, DaemonInfo, ErrorCode};
use rocket_domain::api::HealthInfo;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn startup_creates_private_files_and_daemon_json() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let d = start(&paths).await;

    assert_eq!(mode(&paths.home), 0o700);
    assert_eq!(mode(&paths.logs), 0o700);
    assert_eq!(mode(&paths.socket), 0o600);
    assert_eq!(mode(&paths.pid_file), 0o600);
    assert_eq!(mode(&paths.daemon_json), 0o600);
    assert!(paths.lock_file.exists());
    assert!(paths.db.exists());
    assert_eq!(
        std::fs::read_to_string(&paths.pid_file).unwrap(),
        std::process::id().to_string(),
        "pid file holds the bare pid"
    );

    let info = DaemonInfo::load(&paths.daemon_json).unwrap();
    assert_eq!(info.version, VERSION);
    assert_eq!(info.api, "v1");
    assert_eq!(info.pid as u32, std::process::id());
    assert_eq!(info.socket, paths.socket.to_str().unwrap());
    assert!(info.http.starts_with("http://127.0.0.1:"), "{}", info.http);
    assert_eq!(info.token.len(), 64);
    assert!(
        info.token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    assert_eq!(info.rocket_bin, exe.to_str().unwrap());

    // Same field set and order as the Go file: version, api, pid, socket,
    // http, token, rocket_bin, started_at (two-space indent, trailing newline).
    let raw = std::fs::read_to_string(&paths.daemon_json).unwrap();
    let keys: Vec<&str> = raw
        .lines()
        .filter_map(|l| l.strip_prefix("  \""))
        .map(|l| l.split('"').next().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "version",
            "api",
            "pid",
            "socket",
            "http",
            "token",
            "rocket_bin",
            "started_at"
        ]
    );
    assert!(raw.ends_with("}\n"));

    d.stop().await.unwrap();
}

#[tokio::test]
async fn health_matches_daemon_json() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let d = start(&paths).await;
    let info = DaemonInfo::load(&paths.daemon_json).unwrap();
    let h: HealthInfo = d.client().health().await.unwrap();
    assert!(h.ok);
    assert_eq!(h.api, "v1");
    assert_eq!(h.version, VERSION);
    assert_eq!(h.pid, info.pid);
    assert_eq!(h.home, paths.home.to_str().unwrap());
    assert_eq!(h.socket, info.socket);
    assert_eq!(h.http, info.http);
    assert_eq!(h.started_at, info.started_at);
    d.stop().await.unwrap();
}

#[tokio::test]
async fn stop_removes_discovery_files_and_a_restart_gets_a_fresh_token() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);

    let d = start(&paths).await;
    let first = DaemonInfo::load(&paths.daemon_json).unwrap();
    d.stop().await.unwrap();
    assert!(
        !paths.daemon_json.exists(),
        "daemon.json must go on clean shutdown"
    );
    assert!(!paths.socket.exists());
    assert!(!paths.pid_file.exists());
    assert!(paths.db.exists(), "state is kept");

    let d = start(&paths).await; // also proves the flock was released
    let second = DaemonInfo::load(&paths.daemon_json).unwrap();
    assert_ne!(first.token, second.token, "a new token on every start");
    d.stop().await.unwrap();
}

#[tokio::test]
async fn second_daemon_fails_with_gos_message_and_leaves_the_first_alone() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let first = start(&paths).await;
    let info_before = std::fs::read(&paths.daemon_json).unwrap();

    let err = rocket_daemon::run(rocket_daemon::RunOptions::new(paths.clone(), VERSION))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "another rocketd holds {}: resource temporarily unavailable",
            paths.lock_file.display()
        )
    );

    // Nothing of the running daemon was removed or rewritten.
    assert_eq!(std::fs::read(&paths.daemon_json).unwrap(), info_before);
    assert!(paths.socket.exists() && paths.pid_file.exists());
    assert!(first.client().is_running().await.is_some());
    first.stop().await.unwrap();
}

#[tokio::test]
async fn stale_socket_and_daemon_json_are_replaced() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    paths.ensure().unwrap();
    // A crashed daemon leaves its socket file (nobody listening) and daemon.json.
    drop(std::os::unix::net::UnixListener::bind(&paths.socket).unwrap());
    std::fs::write(
        &paths.daemon_json,
        r#"{"token":"stale","http":"http://127.0.0.1:1"}"#,
    )
    .unwrap();
    std::fs::write(&paths.pid_file, "1").unwrap();

    let d = start(&paths).await;
    let info = DaemonInfo::load(&paths.daemon_json).unwrap();
    assert_ne!(info.token, "stale");
    assert_eq!(
        std::fs::read_to_string(&paths.pid_file).unwrap(),
        std::process::id().to_string()
    );
    d.stop().await.unwrap();
}

#[tokio::test]
async fn too_long_socket_path_is_rejected_before_anything_is_created() {
    let tmp = short_tmp();
    let mut paths = paths_in(&tmp);
    paths.socket = paths.home.join("x".repeat(120)).join("rocketd.sock");
    let err = rocket_daemon::run(rocket_daemon::RunOptions::new(paths.clone(), VERSION))
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("is too long for a unix socket"), "{text}");
    assert!(
        text.contains("set ROCKET_HOME to a shorter directory"),
        "{text}"
    );
    assert!(!paths.home.exists());
}

#[tokio::test]
async fn tcp_listener_needs_the_token_and_the_unix_socket_does_not() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let d = start(&paths).await;
    let info = DaemonInfo::load(&paths.daemon_json).unwrap();

    // Unix socket: no token.
    assert!(d.client().health().await.is_ok());

    // TCP with the right token: the same API.
    let tcp = Client::tcp(&info).unwrap();
    assert_eq!(tcp.health().await.unwrap().pid, info.pid);
    assert!(tcp.projects().await.is_ok());

    // TCP with a wrong token (and no daemon.json to refresh from): 401.
    let bad = Client::tcp_with(&info.http, &"0".repeat(64), None).unwrap();
    let err = bad.health().await.unwrap_err();
    assert_eq!(err.code(), Some(&ErrorCode::Unauthorized), "{err:?}");
    let err = bad.shutdown().await.unwrap_err();
    assert_eq!(err.code(), Some(&ErrorCode::Unauthorized), "{err:?}");
    assert!(
        d.client().is_running().await.is_some(),
        "an unauthorized shutdown must not stop the daemon"
    );

    d.stop().await.unwrap();
}

#[tokio::test]
async fn shutdown_endpoint_stops_the_daemon_and_cleans_up() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let d = start(&paths).await;
    let client = d.client();
    let reply = client.shutdown().await.unwrap();
    assert!(reply.ok);
    assert_eq!(reply.pid as u32, std::process::id());
    d.join().await.unwrap();
    assert!(!paths.daemon_json.exists() && !paths.socket.exists() && !paths.pid_file.exists());
    assert!(client.is_running().await.is_none());
}

#[tokio::test]
async fn shutdown_closes_open_event_streams_without_hanging() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let d = start(&paths).await;
    let mut events = d.client().events(&Default::default()).await.unwrap();
    d.stop.cancel();
    // The 5 s graceful window must not be needed: streams are closed.
    let started = std::time::Instant::now();
    d.join().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    use futures_util::StreamExt;
    let end = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .unwrap();
    assert!(end.is_none() || end.unwrap().is_err());
}

#[tokio::test]
async fn reconcile_marks_vanished_runs_dead_at_start() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    paths.ensure().unwrap();
    {
        // A run whose process is long gone, persisted by a previous daemon.
        use rocket_domain::ports::Store;
        let store = rocket_adapters::sqlite::Store::open(&paths.db).unwrap();
        let run = serde_json::from_value(serde_json::json!({
            "project": "ghost", "service": "x", "env": "dev", "kind": "run",
            "state": "running", "health": "healthy", "pid": 2_000_000_000, "pgid": 2_000_000_000
        }))
        .unwrap();
        store.save_run(&run).unwrap();
    }
    let d = start(&paths).await;
    let ps = d.client().ps(None, true).await.unwrap();
    assert_eq!(ps.services.len(), 1);
    assert_eq!(ps.services[0].state, rocket_domain::RunState::Dead);
    d.stop().await.unwrap();
}

#[tokio::test]
async fn ttl_ticker_cancels_expired_jobs() {
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let d = start_with(&paths, |o| o.ttl_interval = Duration::from_millis(100)).await;
    {
        use rocket_domain::ports::Store;
        let store = rocket_adapters::sqlite::Store::open(&paths.db).unwrap();
        let now = time::OffsetDateTime::now_utc();
        let job = serde_json::from_value(serde_json::json!({
            "id": "jttl", "project": "ghost", "name": "x", "kind": "pipeline", "owner": "user",
            "steps": [{"run": "true"}], "status": "running",
            "started_at": now.format(&time::format_description::well_known::Rfc3339).unwrap(),
            "expires_at": (now - time::Duration::seconds(1))
                .format(&time::format_description::well_known::Rfc3339).unwrap(),
        }))
        .unwrap();
        store.save_job(&job).unwrap();
    }
    let client = d.client();
    for _ in 0..100 {
        let job = client.job("jttl", false).await.unwrap();
        if job.status == rocket_domain::JobStatus::Canceled {
            assert_eq!(job.error, "ttl expired");
            d.stop().await.unwrap();
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the TTL ticker never expired the job");
}

#[test]
fn default_ttl_interval_is_five_seconds() {
    assert_eq!(rocket_daemon::TTL_INTERVAL, Duration::from_secs(5));
}
