//! Smoke test against a real rocket binary (the Rust `rocket` binary).
//!
//! ```sh
//! cargo build -p rocket-cli
//! ROCKET_BIN=$PWD/target/debug/rocket cargo test -p rocket-client -- --ignored
//! ```
#![cfg(unix)]

mod common;

use futures_util::StreamExt;
use rocket_client::{
    Client, DaemonEvent, EnsureOptions, ErrorCode, EventFilter, Paths, TransportKind, ensure_daemon,
};
use rocket_domain::JobKind;
use rocket_domain::api::{DownRequest, JobRequest, UpRequest};
use std::path::PathBuf;
use std::process::Command;

#[tokio::test]
#[ignore = "needs ROCKET_BIN pointing at a real rocket binary"]
async fn real_daemon_start_query_stop() {
    let Some(bin) = std::env::var_os("ROCKET_BIN").map(PathBuf::from) else {
        panic!("set ROCKET_BIN to a rocket binary to run this test");
    };
    let tmp = common::short_tmp(); // never the real ~/.rocket
    let paths = Paths::from_home(tmp.path()).unwrap();

    let mut opts = EnsureOptions::new(paths.clone());
    opts.rocket_bin = Some(bin.clone());
    let unix = ensure_daemon(&opts).await.expect("daemon starts");

    let result = async {
        let health = unix.health().await?;
        assert!(health.ok);
        assert_eq!(health.api, "v1");
        assert_eq!(health.home, paths.home.to_str().unwrap());
        assert!(unix.projects().await?.projects.is_empty());
        assert!(unix.ps(None, true).await?.services.is_empty());
        assert!(unix.ports().await?.ports.is_empty());
        assert!(unix.jobs(&Default::default()).await?.jobs.is_empty());
        assert!(unix.status(None).await?.services.is_empty());
        let _ = unix.gc().await?;

        // The same daemon over the token-protected TCP listener.
        let tcp = Client::from_paths(&paths, TransportKind::Tcp)?;
        assert_eq!(tcp.health().await?.pid, health.pid);

        // ensure_daemon is idempotent: it reuses the running daemon.
        assert_eq!(ensure_daemon(&opts).await?.health().await?.pid, health.pid);
        // Full flow on the fixture project: register, up, stream, job, deploy gate, down.
        let fixture = format!("{}/../../testdata/fixture", env!("CARGO_MANIFEST_DIR"));
        let fixture = std::fs::canonicalize(fixture).unwrap();
        let project = unix.add_project(fixture.to_str().unwrap()).await?;
        assert_eq!(project.name, "rocket-fixture");
        let mut events = unix
            .events(&EventFilter {
                project: Some(project.name.clone()),
                types: vec!["service.state".into()],
                ..Default::default()
            })
            .await?;
        let up = unix
            .up(&UpRequest {
                project: project.name.clone(),
                services: vec!["sleeper".into()],
                owner: "user".into(),
                ..Default::default()
            })
            .await?;
        assert!(up.services.iter().any(|s| s.service == "sleeper"), "{up:?}");
        let ev = tokio::time::timeout(std::time::Duration::from_secs(10), events.next())
            .await
            .expect("a service.state event")
            .expect("stream open")?;
        assert!(matches!(ev, DaemonEvent::ServiceState(_)), "{ev:?}");
        assert!(
            !unix
                .ps(Some(&project.name), false)
                .await?
                .services
                .is_empty()
        );
        let logs = unix.logs(&project.name, "sleeper", 20).await?;
        assert!(
            logs.lines.iter().any(|l| l.contains("greeting=")),
            "{logs:?}"
        );

        let job = unix
            .start_job(&JobRequest {
                project: project.name.clone(),
                kind: JobKind::Pipeline,
                name: "check".into(),
                ..Default::default()
            })
            .await?;
        let follow: Vec<_> = unix.follow_job_logs(&job.id, None).await?.collect().await;
        assert!(
            follow
                .last()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_terminal_job_state(),
            "{follow:?}"
        );
        let done = unix.job(&job.id, true).await?;
        assert_eq!(done.status, rocket_domain::JobStatus::Succeeded);

        let gate = unix
            .start_job(&JobRequest {
                project: project.name.clone(),
                kind: JobKind::Deploy,
                name: "stage".into(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(
            gate.code(),
            Some(&ErrorCode::ConfirmationRequired),
            "{gate:?}"
        );

        let down = unix
            .down(&DownRequest {
                project: project.name.clone(),
                ..Default::default()
            })
            .await?;
        assert!(!down.stopped.is_empty());
        assert_eq!(
            unix.remove_project(&project.name).await?.removed,
            project.name
        );
        Ok::<_, rocket_client::ClientError>(())
    }
    .await;

    // Always stop the daemon we started, even when an assertion failed.
    let stop = Command::new(&bin)
        .args(["daemon", "stop"])
        .env("ROCKET_HOME", &paths.home)
        .output()
        .expect("run daemon stop");
    result.expect("client calls");
    assert!(stop.status.success(), "daemon stop: {stop:?}");
    assert!(unix.is_running().await.is_none(), "daemon must be gone");
}
