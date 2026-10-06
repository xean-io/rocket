//! Rust-only tests for behavior the Go suite covers through jobs (R6) or not
//! at all: cancellation, published events and the error-code mapping.

use crate::testing::*;
use crate::{AppError, CancellationToken};
use rocket_domain::api::service_action;
use rocket_domain::ports::Store;
use rocket_domain::{RunState, event_type};
use std::time::Duration;

#[tokio::test]
async fn cancel_stops_only_the_service_being_started() {
    let h = Harness::new();
    h.health.fail_port(3002); // api never becomes healthy
    let token = CancellationToken::new();
    let canceller = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        token.cancel();
    };
    let (res, ()) = tokio::join!(h.app.up_with(nuvara_req(&["web"]), &token, None), canceller);

    assert_eq!(res.unwrap_err(), AppError::Canceled);
    // postgres (already started) is untouched; api failed and was stopped;
    // web never started.
    assert_eq!(h.run("nuvara", "postgres").state, RunState::Running);
    let api = h.run("nuvara", "api");
    assert_eq!(api.state, RunState::Failed);
    assert_eq!(api.error, "context canceled");
    assert_eq!(h.runner.stopped(), [api.pgid]);
    assert_eq!(
        h.runner.started().len(),
        1,
        "web must not start after cancellation"
    );
    assert!(h.store.get_run("nuvara", "web").unwrap().is_none());
    assert!(
        h.store
            .list_leases()
            .unwrap()
            .iter()
            .all(|l| l.service != "api"),
        "canceled service kept its lease"
    );
}

#[tokio::test]
async fn up_with_a_canceled_token_fails_fast() {
    let h = Harness::new();
    let token = CancellationToken::new();
    token.cancel();
    let err = h
        .app
        .up_with(nuvara_req(&["api"]), &token, None)
        .await
        .unwrap_err();
    assert_eq!(err, AppError::Canceled);
    assert!(h.runner.started().is_empty());
}

#[tokio::test]
async fn deadline_sets_the_ttl_of_started_services() {
    let h = Harness::new();
    let deadline = h.now() + Duration::from_secs(90);
    let res = h
        .app
        .up_with(
            nuvara_req(&["causation"]),
            &CancellationToken::new(),
            Some(deadline),
        )
        .await
        .unwrap();
    assert_eq!(res.services[0].expires_at, Some(deadline));
}

#[tokio::test]
async fn up_and_down_publish_state_and_lease_events() {
    let h = Harness::new();
    h.up(nuvara_req(&["causation"])).await;
    h.app
        .down(rocket_domain::api::DownRequest {
            project: "/code/nuvara".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let events = h.bus.events();
    let kinds: Vec<&str> = events.iter().map(|e| e.r#type.as_str()).collect();
    assert!(kinds.contains(&event_type::PORT_LEASED), "{kinds:?}");
    assert!(kinds.contains(&event_type::PORT_RELEASED), "{kinds:?}");
    let states: Vec<_> = events
        .iter()
        .filter(|e| e.r#type == event_type::SERVICE_STATE)
        .filter_map(|e| e.state)
        .collect();
    assert_eq!(
        states,
        [
            RunState::Starting,
            RunState::Running,
            RunState::Stopping,
            RunState::Stopped
        ]
    );
    assert!(
        events
            .iter()
            .all(|e| e.project == "nuvara" && e.service == "causation")
    );
}

#[tokio::test]
async fn close_then_next_app_adopts_live_processes() {
    let h = Harness::new();
    h.up(nuvara_req(&["causation"])).await;
    h.app.close().await;
    let next = crate::App::new(h.deps.clone());
    let res = next.reconcile().await.unwrap();
    assert_eq!(res.adopted.len(), 1, "{res:?}");
    assert_eq!(res.adopted[0].service, "causation");
}

#[test]
fn error_codes_match_the_go_api_mapping() {
    let cases = [
        (AppError::invalid("x"), "invalid", 400, "invalid request: x"),
        (AppError::not_found("x"), "not_found", 404, "not found: x"),
        (AppError::conflict("x"), "conflict", 409, "conflict: x"),
        (
            AppError::confirmation_required("x"),
            "confirmation_required",
            428,
            "confirmation required: x",
        ),
        (AppError::internal("x"), "internal", 500, "x"),
        (AppError::Canceled, "internal", 500, "context canceled"),
    ];
    for (err, code, status, text) in cases {
        assert_eq!(
            (err.code(), err.http_status(), err.to_string().as_str()),
            (code, status, text)
        );
    }
}

#[tokio::test]
async fn unknown_project_and_service_errors_map_to_codes() {
    let h = Harness::new();
    let err = h.app.up(req("ghost", &[])).await.unwrap_err();
    assert_eq!(err.code(), "not_found", "{err}");
    let err = h.app.up(nuvara_req(&["nope"])).await.unwrap_err();
    assert_eq!(err.code(), "invalid", "{err}");
    let err = h.app.up(req("", &[])).await.unwrap_err();
    assert_eq!(err.code(), "invalid", "{err}");
    let err = h.app.add_project("relative/path").unwrap_err();
    assert_eq!(err.code(), "invalid", "{err}");
    let err = h
        .app
        .logs(crate::LogsRequest {
            project: "/code/nuvara".into(),
            service: "ghost".into(),
            tail: 0,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), "not_found", "{err}");
}

#[tokio::test]
async fn project_name_collision_is_a_conflict() {
    let h = Harness::new();
    h.app.add_project("/code/nuvara").unwrap();
    let mut clone = nuvara();
    clone.root = "/elsewhere/nuvara".into();
    h.loader.set(clone);
    let err = h.app.add_project("/elsewhere/nuvara").unwrap_err();
    assert_eq!(err.code(), "conflict", "{err}");
    assert!(
        err.to_string()
            .contains("already registered at /code/nuvara"),
        "{err}"
    );
}

#[tokio::test]
async fn ports_lists_leases_with_their_runs() {
    let h = Harness::new();
    h.up(nuvara_req(&["api"])).await;
    let ports = h.app.ports().unwrap().ports;
    let numbers: Vec<u16> = ports.iter().map(|p| p.lease.port).collect();
    assert_eq!(numbers, [3002, 5435]);
    assert_eq!(ports[0].state, Some(RunState::Running));
    assert_eq!(ports[0].owner, "user");
    assert_eq!(ports[0].env, "dev");
}

#[tokio::test]
async fn summary_reports_remapped_and_busy_conflicts() {
    let h = Harness::new();
    h.probe.set_busy(3000, holder(5, "node"));
    // busy while stopped
    let sum = h.app.summary("/code/nuvara").unwrap();
    let web = sum.conflicts.iter().find(|c| c.service == "web").unwrap();
    assert_eq!(web.kind, "port_busy");
    assert!(
        web.detail
            .contains("`rocket up web` will remap it via PORT"),
        "{}",
        web.detail
    );
    // remapped while running
    h.up(nuvara_req(&["web"])).await;
    let sum = h.app.summary("/code/nuvara").unwrap();
    let web = sum.conflicts.iter().find(|c| c.service == "web").unwrap();
    assert_eq!(
        (web.kind.as_str(), web.port, web.default),
        ("port_remapped", 3100, 3000)
    );
    assert_eq!(
        web.detail,
        "running on 3100 instead of 3000 (injected via PORT)"
    );
    assert!(
        sum.services
            .iter()
            .any(|r| r.service == "web" && r.state == RunState::Running)
    );
}

#[tokio::test]
async fn logs_tail_and_compose_logs() {
    let h = Harness::new();
    h.up(nuvara_req(&["api", "postgres"])).await;
    let log = h
        .app
        .logs(crate::LogsRequest {
            project: "/code/nuvara".into(),
            service: "api".into(),
            tail: 5,
        })
        .await
        .unwrap();
    assert!(
        log.lines[0].starts_with(
            "=== rocket: starting api (run, env dev, owner user) at 2026-01-01T10:00:00Z"
        ),
        "{:?}",
        log.lines
    );
    let compose = h
        .app
        .logs(crate::LogsRequest {
            project: "/code/nuvara".into(),
            service: "postgres".into(),
            tail: 0,
        })
        .await
        .unwrap();
    assert_eq!(compose.lines, ["compose log"]);
}

#[tokio::test]
async fn up_result_failed_counts_skipped_and_failed() {
    let h = Harness::new();
    h.probe.set_busy(3003, holder(9, "python3"));
    let res = h.up(nuvara_req(&["worker"])).await;
    assert!(res.failed());
    assert_eq!(res.services[1].action, service_action::SKIPPED);
    assert_eq!(res.services[1].error, "dependency \"causation\" failed");
}

#[tokio::test]
async fn start_failure_is_recorded_and_releases_leases() {
    let h = Harness::new();
    h.runner.set_start_error(Some("boom"));
    let res = h.up(nuvara_req(&["causation"])).await;
    assert_eq!(res.services[0].action, service_action::FAILED);
    assert_eq!(res.services[0].error, "start causation: boom");
    assert_eq!(h.run("nuvara", "causation").state, RunState::Failed);
    assert!(h.store.list_leases().unwrap().is_empty());
}
