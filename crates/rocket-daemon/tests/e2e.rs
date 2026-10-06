//! End to end: the real daemon `run()` in this process on a short /tmp home,
//! driven through `rocket-client` over the unix socket and the token-guarded
//! TCP listener. Needs `python3` (the fixture's services are `http.server`s).
#![cfg(unix)]

mod common;

use common::{copy_fixture, paths_in, short_tmp, start};
use futures_util::FutureExt;
use futures_util::StreamExt;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use rocket_client::{Client, DaemonEvent, EventFilter, TransportKind};
use rocket_domain::api::{DownRequest, JobRequest, UpRequest};
use rocket_domain::{JobKind, JobStatus, RunState};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;
use tokio::sync::Mutex;

/// The fixture's services use fixed ports: one test at a time.
static PORTS: Mutex<()> = Mutex::const_new(());

fn alive(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}

/// Runs `body`, then always has the daemon stop everything it supervises, even
/// when an assertion failed, so no fixture process outlives the test.
async fn with_cleanup(client: &Client, body: impl Future<Output = ()>) {
    let outcome = AssertUnwindSafe(body).catch_unwind().await;
    let _ = client
        .down(&DownRequest {
            everywhere: true,
            ..Default::default()
        })
        .await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

async fn eventually(what: &str, mut cond: impl AsyncFnMut() -> bool) {
    for _ in 0..200 {
        if cond().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn full_flow_health_projects_up_events_jobs_down_shutdown() {
    let _ports = PORTS.lock().await;
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let project_dir = copy_fixture(&tmp.path().join("proj"));
    let d = start(&paths).await;
    let client = d.client();

    let flow = async {
        let health = client.health().await?;
        assert!(health.ok);
        assert_eq!(health.api, "v1");

        let project = client.add_project(project_dir.to_str().unwrap()).await?;
        assert_eq!(project.name, "rocket-fixture");
        assert_eq!(client.projects().await?.projects.len(), 1);

        // Subscribe before `up` so the service.state events are not missed.
        let mut events = client
            .events(&EventFilter {
                project: Some(project.name.clone()),
                types: vec!["service.state".into()],
                ..Default::default()
            })
            .await?;

        let up = client
            .up(&UpRequest {
                project: project.name.clone(),
                services: vec!["sleeper".into()],
                owner: "user".into(),
                ..Default::default()
            })
            .await?;
        assert!(!up.failed(), "{up:?}");
        let started: Vec<&str> = up.services.iter().map(|s| s.service.as_str()).collect();
        assert_eq!(started, ["static", "sleeper"], "dependencies first: {up:?}");
        let sleeper_pid = up.services[1].pid;
        assert!(sleeper_pid > 0 && alive(sleeper_pid));

        // service.state events arrive in the stream.
        let mut seen = Vec::new();
        while !seen
            .iter()
            .any(|(s, st)| s == "sleeper" && *st == Some(RunState::Running))
        {
            let ev = tokio::time::timeout(Duration::from_secs(10), events.next())
                .await
                .expect("a service.state event")
                .expect("stream open")?;
            let DaemonEvent::ServiceState(e) = ev else {
                panic!("{ev:?}")
            };
            assert!(e.run.is_some());
            seen.push((e.service, e.state));
        }

        let ps = client.ps(Some(&project.name), false).await?;
        let running: Vec<_> = ps
            .services
            .iter()
            .filter(|r| r.state == RunState::Running)
            .collect();
        assert_eq!(running.len(), 2, "{ps:?}");
        let logs = client.logs(&project.name, "sleeper", 20).await?;
        assert!(
            logs.lines.iter().any(|l| l.contains("greeting=hello")),
            "{logs:?}"
        );
        assert!(!client.ports().await?.ports.is_empty());

        // The same state through the token-protected TCP listener.
        let tcp = Client::from_paths(&paths, TransportKind::Tcp)?;
        assert_eq!(
            tcp.ps(Some(&project.name), false).await?.services.len(),
            ps.services.len()
        );

        // A pipeline job followed to its terminal event.
        let job = client
            .start_job(&JobRequest {
                project: project.name.clone(),
                kind: JobKind::Pipeline,
                name: "check".into(),
                ..Default::default()
            })
            .await?;
        assert_eq!(job.status, JobStatus::Running);
        let follow: Vec<_> = client.follow_job_logs(&job.id, None).await?.collect().await;
        let items: Vec<DaemonEvent> = follow.into_iter().collect::<Result<_, _>>()?;
        assert!(items.last().unwrap().is_terminal_job_state(), "{items:?}");
        let lines: Vec<&str> = items
            .iter()
            .filter_map(|e| match e {
                DaemonEvent::JobLog(ev) => Some(ev.line.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            lines.contains(&"step-one") && lines.contains(&"step-two greeting=hello"),
            "{lines:?}"
        );
        let done = client.job(&job.id, true).await?;
        assert_eq!(done.status, JobStatus::Succeeded);
        assert_eq!(done.exit_code, Some(0));
        let listed = client.jobs(&Default::default()).await?;
        assert!(listed.jobs.iter().any(|j| j.id == job.id));

        // down stops the processes it started.
        let down = client
            .down(&DownRequest {
                project: project.name.clone(),
                ..Default::default()
            })
            .await?;
        assert_eq!(down.stopped.len(), 2, "{down:?}");
        eventually("services to exit", async || !alive(sleeper_pid)).await;
        assert!(
            client
                .ps(Some(&project.name), false)
                .await?
                .services
                .iter()
                .all(|r| r.state != RunState::Running)
        );
        Ok::<_, rocket_client::ClientError>(())
    };
    with_cleanup(&client, async { flow.await.expect("client calls") }).await;

    // Shut the daemon down through the API.
    assert!(client.shutdown().await.unwrap().ok);
    d.join().await.unwrap();
    assert!(!paths.daemon_json.exists() && !paths.socket.exists());
}

#[tokio::test]
async fn services_survive_a_daemon_restart_and_are_adopted() {
    let _ports = PORTS.lock().await;
    let tmp = short_tmp();
    let paths = paths_in(&tmp);
    let project_dir = copy_fixture(&tmp.path().join("proj"));

    let d = start(&paths).await;
    let client = d.client();
    let up = client
        .up(&UpRequest {
            project: project_dir.to_str().unwrap().into(),
            services: vec!["sleeper".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    let pid = up
        .services
        .iter()
        .find(|s| s.service == "sleeper")
        .map_or(0, |s| s.pid);
    // The daemon stops; the services it started must not.
    let first = async {
        assert!(!up.failed(), "{up:?}");
        d.stop().await.unwrap();
        // "supervised services keep running and will be adopted on next start"
        assert!(
            alive(pid),
            "the daemon must not kill supervised services on stop"
        );
    };
    let outcome = AssertUnwindSafe(first).catch_unwind().await;

    let d = start(&paths).await;
    let client = d.client();
    let second = async {
        outcome.expect("first daemon phase");
        let ps = client.ps(None, true).await.unwrap();
        let sleeper = ps.services.iter().find(|r| r.service == "sleeper").unwrap();
        assert_eq!(sleeper.state, RunState::Running, "{ps:?}");
        assert_eq!(sleeper.pid, pid);
        let down = client
            .down(&DownRequest {
                everywhere: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(down.stopped.len(), 2, "{down:?}");
        eventually("adopted services to stop", async || !alive(pid)).await;
    };
    with_cleanup(&client, second).await;
    d.stop().await.unwrap();
}
