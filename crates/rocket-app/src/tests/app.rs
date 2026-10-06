//! Ports of `app_test.go`.

use crate::AppError;
use crate::StatusRequest;
use crate::testing::*;
use rocket_domain::api::{DownRequest, UpRequest, service_action};
use rocket_domain::ports::Store;
use rocket_domain::{DEFAULT_OWNER, Health, HealthCheck, Lease, RunState, ServiceKind};
use std::time::Duration;

fn lease(port: u16, project: &str, service: &str, name: &str) -> Lease {
    Lease {
        port,
        project: project.into(),
        service: service.into(),
        port_name: name.into(),
        created_at: time::macros::datetime!(2026-01-01 10:00:00 UTC),
    }
}

#[tokio::test]
async fn up_starts_dependencies_in_order() {
    let h = Harness::new();
    let res = h.up(nuvara_req(&["web"])).await;

    for s in &res.services {
        assert_eq!(
            s.action,
            service_action::STARTED,
            "{}: {}",
            s.service,
            s.error
        );
    }
    let order: Vec<&str> = res.services.iter().map(|s| s.service.as_str()).collect();
    assert_eq!(order, ["postgres", "api", "web"]);
    assert_eq!(res.env, "dev");
    assert_eq!(res.project, "nuvara");

    let up = &h.compose.calls()[0];
    assert_eq!(up.target.project_name, "rocket-nuvara-dev");
    assert_eq!(up.service, "postgres");
    assert_eq!(up.target.profiles, ["deps"]);
    assert_eq!(up.target.files, ["/code/nuvara/docker-compose.yml"]);
    assert_eq!(
        env_map(&up.target.env)["POSTGRES_PORT"],
        "5435",
        "compose port var not injected"
    );

    let api = h.runner.spec_for("apps/api");
    assert_eq!(api.argv, ["/bin/sh", "-c", "bun run dev"]);
    let env = env_map(&api.env);
    assert_eq!(env["PORT"], "3002");
    assert_eq!(env["FROM_DOTENV"], "1");
    assert_eq!(env["PATH"], "/usr/bin");
    assert!(
        !env.contains_key("ROCKET_OWNER"),
        "ROCKET_* leaked into child env"
    );

    let r = h.run("nuvara", "api");
    assert_eq!(r.state, RunState::Running);
    assert_eq!(r.health, Health::Healthy);
    assert_eq!(r.owner, DEFAULT_OWNER);
    assert_eq!(r.ports["http"], 3002);
    assert_eq!(
        h.health.checks()[0],
        HealthCheck {
            kind: "http".into(),
            port: 3002,
            path: "/health".into()
        }
    );
    assert_eq!(h.store.list_leases().unwrap().len(), 3);
}

#[tokio::test]
async fn up_is_idempotent() {
    let h = Harness::new();
    h.up(nuvara_req(&["core"])).await;
    let started = h.runner.started().len();

    let res = h.up(nuvara_req(&["core"])).await;
    for s in &res.services {
        assert_eq!(s.action, service_action::ALREADY_RUNNING, "{}", s.service);
    }
    assert_eq!(
        h.runner.started().len(),
        started,
        "services were started twice"
    );
}

#[tokio::test]
async fn up_remaps_busy_port_and_injects_env() {
    let h = Harness::new();
    h.probe.set_busy(
        3000,
        rocket_domain::PortHolder {
            pid: 77,
            command: "node".into(),
            cwd: "/code/elsewhere".into(),
        },
    );

    let res = h.up(nuvara_req(&["web"])).await;
    let web = res.services.last().unwrap();
    assert_eq!(web.action, service_action::STARTED);
    assert_eq!(web.ports["http"], 3100);
    assert_eq!(web.remaps.len(), 1);
    let remap = &web.remaps[0];
    assert_eq!(
        (remap.from, remap.to, remap.env.as_str()),
        (3000, 3100, "PORT")
    );
    assert_eq!(remap.holder.as_ref().unwrap().pid, 77);
    assert_eq!(
        env_map(&h.runner.spec_for("apps/web").env)["PORT"],
        "3100",
        "remapped port not injected"
    );
}

#[tokio::test]
async fn up_remaps_port_leased_by_another_project() {
    let h = Harness::new();
    h.up(req("/code/shop", &[])).await;
    let res = h.up(nuvara_req(&["web"])).await;
    let web = res.services.last().unwrap();
    assert_eq!(web.ports["http"], 3100);
    assert!(
        web.remaps[0].reason.contains("shop/control-plane"),
        "{web:?}"
    );
}

#[tokio::test]
async fn up_fails_busy_port_without_env_and_skips_dependents() {
    let h = Harness::new();
    h.probe.set_busy(3003, holder(9, "python3"));

    let res = h.up(nuvara_req(&["worker"])).await;
    let acts = actions(&res);
    assert_eq!(acts["causation"], service_action::FAILED, "{acts:?}");
    assert_eq!(acts["worker"], service_action::SKIPPED, "{acts:?}");
    let msg = &res.services[0].error;
    for want in ["3003", "python3", "pid 9", "env"] {
        assert!(msg.contains(want), "error {msg:?} missing {want:?}");
    }
    assert!(h.runner.started().is_empty(), "nothing should have started");
    assert!(res.failed(), "result should report failure");
}

#[tokio::test]
async fn up_fails_when_process_exits_before_healthy() {
    let h = Harness::new();
    h.runner.exit_now("bun run dev", 1);
    h.health.fail_port(3002);

    let res = h.up(nuvara_req(&["api"])).await;
    assert_eq!(actions(&res)["api"], service_action::FAILED);
    assert!(
        res.services[1].error.contains("exited with code 1"),
        "error {:?}",
        res.services[1].error
    );
    assert_eq!(h.run("nuvara", "api").state, RunState::Failed);
    for l in h.store.list_leases().unwrap() {
        assert_ne!(l.service, "api", "failed service kept its lease");
    }
}

#[tokio::test]
async fn process_exit_is_recorded() {
    let h = Harness::new();
    h.up(nuvara_req(&["causation"])).await;
    let r = h.run("nuvara", "causation");
    h.runner.exit(r.pid, 2);
    eventually(|| h.run("nuvara", "causation").state == RunState::Exited).await;
    assert_eq!(h.run("nuvara", "causation").exit_code, Some(2));
    assert!(
        h.store.list_leases().unwrap().is_empty(),
        "leases not released"
    );
}

#[tokio::test]
async fn down_by_owner_only_stops_that_owner() {
    let h = Harness::new();
    h.up(UpRequest {
        owner: "user".into(),
        ..nuvara_req(&["postgres"])
    })
    .await;
    h.up(UpRequest {
        owner: "agent:a1".into(),
        ..nuvara_req(&["api"])
    })
    .await;

    assert_eq!(
        h.run("nuvara", "postgres").owner,
        "user",
        "postgres owner changed"
    );
    let res = h
        .app
        .down(DownRequest {
            project: "/code/nuvara".into(),
            owner: "agent:a1".into(),
            ..DownRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(res.stopped.len(), 1);
    assert_eq!(res.stopped[0].service, "api");
    assert_eq!(
        h.run("nuvara", "postgres").state,
        RunState::Running,
        "postgres should keep running"
    );
    assert_eq!(
        h.run("nuvara", "api").state,
        RunState::Stopped,
        "api should be stopped"
    );
}

#[tokio::test]
async fn down_project_stops_in_reverse_order_and_downs_compose() {
    let h = Harness::new();
    h.up(nuvara_req(&["web"])).await;
    let res = h
        .app
        .down(DownRequest {
            project: "/code/nuvara".into(),
            ..DownRequest::default()
        })
        .await
        .unwrap();
    let order: Vec<&str> = res.stopped.iter().map(|r| r.service.as_str()).collect();
    assert_eq!(order, ["web", "api", "postgres"]);
    assert_eq!(h.compose.ops(), ["up:postgres", "stop:postgres", "down:"]);
    assert!(h.store.list_leases().unwrap().is_empty(), "leases left");
    assert_eq!(res.compose_down, ["rocket-nuvara-dev"]);
}

#[tokio::test]
async fn down_everywhere() {
    let h = Harness::new();
    h.up(nuvara_req(&["api"])).await;
    h.up(req("/code/shop", &[])).await;
    let res = h
        .app
        .down(DownRequest {
            everywhere: true,
            ..DownRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(res.stopped.len(), 3, "{:?}", res.stopped);
}

#[tokio::test]
async fn ttl_expiry() {
    let h = Harness::new();
    let res = h
        .up(UpRequest {
            owner: "agent:t1".into(),
            ttl: "1m".into(),
            ..nuvara_req(&["causation"])
        })
        .await;
    assert!(res.services[0].expires_at.is_some(), "expires_at missing");
    assert!(
        h.app.expire_ttl().await.unwrap().is_empty(),
        "expired too early"
    );
    h.clock.advance(Duration::from_secs(120));
    let expired = h.app.expire_ttl().await.unwrap();
    assert_eq!(expired.len(), 1, "{expired:?}");
    assert_eq!(h.run("nuvara", "causation").state, RunState::Stopped);
}

#[tokio::test]
async fn invalid_ttl_rejected() {
    let h = Harness::new();
    let err = h
        .app
        .up(UpRequest {
            ttl: "soon".into(),
            ..req("/code/nuvara", &[])
        })
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Invalid(_)), "{err:?}");
    assert!(err.to_string().contains("ttl"), "{err}");
}

#[tokio::test]
async fn restart() {
    let h = Harness::new();
    h.up(nuvara_req(&["causation"])).await;
    let first = h.run("nuvara", "causation").pid;
    let res = h.app.restart(nuvara_req(&["causation"])).await.unwrap();
    assert_eq!(
        res.services[0].action,
        service_action::STARTED,
        "{:?}",
        res.services
    );
    assert_ne!(
        h.run("nuvara", "causation").pid,
        first,
        "{:?}",
        res.services
    );
}

#[tokio::test]
async fn reconcile() {
    let h = Harness::new();
    let now = h.now();
    let running = |service: &str, kind, pid: i32| {
        let mut r = blank_run("nuvara", service, kind, RunState::Running);
        (r.pid, r.pgid, r.started_at) = (pid, pid, Some(now));
        r
    };
    let mut postgres = blank_run(
        "nuvara",
        "postgres",
        ServiceKind::Compose,
        RunState::Running,
    );
    postgres.compose_project = "rocket-nuvara-dev".into();
    for r in [
        running("api", ServiceKind::Run, 10),
        running("web", ServiceKind::Run, 11),
        postgres,
        blank_run("nuvara", "worker", ServiceKind::Run, RunState::Stopped),
    ] {
        h.store.save_run(&r).unwrap();
    }
    h.store
        .acquire_lease(&lease(3000, "nuvara", "web", "http"))
        .unwrap();
    h.store
        .acquire_lease(&lease(4000, "nuvara", "worker", "x"))
        .unwrap();
    h.runner.set_alive(10, true);

    let res = h.app.reconcile().await.unwrap();
    assert_eq!(res.adopted.len(), 1, "{:?}", res.adopted);
    assert_eq!(res.adopted[0].service, "api");
    assert_eq!(res.dead.len(), 2, "{:?}", res.dead);
    assert_eq!(h.run("nuvara", "web").state, RunState::Dead);
    assert_eq!(h.run("nuvara", "postgres").state, RunState::Dead);
    assert!(
        h.store.list_leases().unwrap().is_empty(),
        "stale leases kept"
    );

    // The adopted process is polled; when it dies the run is updated.
    h.runner.set_alive(10, false);
    eventually(|| h.run("nuvara", "api").state == RunState::Exited).await;
}

#[tokio::test]
async fn status_lists_declared_services() {
    let h = Harness::new();
    h.up(nuvara_req(&["causation"])).await;
    let res = h
        .app
        .status(&StatusRequest {
            project: "/code/nuvara".into(),
            ..StatusRequest::default()
        })
        .unwrap();
    let states: std::collections::BTreeMap<_, _> = res
        .services
        .iter()
        .map(|r| (r.service.as_str(), r.state))
        .collect();
    assert_eq!(states.len(), 5, "{states:?}");
    assert_eq!(states["causation"], RunState::Running);
    assert_eq!(states["web"], RunState::Stopped);
}

#[tokio::test]
async fn gc_releases_stale_leases_and_prunes() {
    let h = Harness::new();
    let mut web = blank_run("nuvara", "web", ServiceKind::Run, RunState::Running);
    (web.pid, web.pgid) = (55, 55);
    h.store.save_run(&web).unwrap();
    h.store
        .acquire_lease(&lease(3000, "nuvara", "web", "http"))
        .unwrap();
    h.store
        .acquire_lease(&lease(9999, "ghost", "x", "http"))
        .unwrap();

    let res = h.app.gc().await.unwrap();
    assert!(res.actions.len() >= 2, "{:?}", res.actions);
    assert!(h.store.list_leases().unwrap().is_empty(), "leases left");
    assert!(
        h.store.get_run("nuvara", "web").unwrap().is_none(),
        "dead run record should be pruned"
    );
}

#[tokio::test]
async fn projects_registry() {
    let h = Harness::new();
    let reference = h.app.add_project("/code/nuvara").unwrap();
    assert_eq!(reference.name, "nuvara");
    h.app
        .up(req("nuvara", &["causation"]))
        .await
        .expect("up by name");
    assert!(
        matches!(h.app.remove_project("nuvara"), Err(AppError::Conflict(_))),
        "removing a project with running services must fail"
    );
    assert!(
        h.app.up(req("unknown", &[])).await.is_err(),
        "unknown project should fail"
    );
}
