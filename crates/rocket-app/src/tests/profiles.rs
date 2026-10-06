//! Ports of `profiles_test.go`.

use crate::AppError;
use crate::testing::*;
use rocket_domain::api::{DownRequest, UpRequest, service_action};
use rocket_domain::{Environment, Project, RunState, ServiceKind};
use std::collections::BTreeMap;

pub(super) const ROOT: &str = "/code/profiles";

pub(super) fn profile_project() -> Project {
    rocket_manifest::parse(
        br#"version: 1
name: profiles
default_env: dev
envs:
  dev: {compose: [compose.yaml], profiles: [base]}
  smoke: {compose: [compose.yaml], profiles: [trends]}
services:
  api: {run: dev, depends_on: [required]}
  required: {run: dependency, profiles: [internal]}
  ordinary: {run: ordinary}
  trends: {run: trends, profiles: [trends]}
  reporting: {compose: reporting, profiles: [reports, trends]}
groups:
  all: ["*"]
  explicit: [trends]
  mixed: ["*", trends]
"#,
        ROOT,
    )
    .expect("profile manifest parses")
}

fn profile_request(body: &str) -> UpRequest {
    let mut req: UpRequest = serde_json::from_str(body).expect("request JSON");
    req.project = ROOT.into();
    req
}

pub(super) fn profile_harness() -> Harness {
    let h = Harness::new();
    h.loader.set(profile_project());
    h
}

#[tokio::test]
async fn up_profile_selection() {
    let cases: [(&str, &str, &[&str]); 8] = [
        ("empty wildcard", "{}", &["api", "ordinary", "required"]),
        (
            "literal wildcard",
            r#"{"services":["*"]}"#,
            &["api", "ordinary", "required"],
        ),
        (
            "wildcard group",
            r#"{"services":["all"]}"#,
            &["api", "ordinary", "required"],
        ),
        (
            "requested profiles",
            r#"{"services":["all"],"profiles":["reports"]}"#,
            &["api", "ordinary", "reporting", "required"],
        ),
        (
            "selected environment profiles",
            r#"{"services":["all"],"env":"smoke"}"#,
            &["api", "ordinary", "reporting", "required", "trends"],
        ),
        (
            "explicit service",
            r#"{"services":["trends"]}"#,
            &["trends"],
        ),
        (
            "explicit group member",
            r#"{"services":["explicit"]}"#,
            &["trends"],
        ),
        (
            "mixed wildcard and explicit member",
            r#"{"services":["mixed"]}"#,
            &["api", "ordinary", "required", "trends"],
        ),
    ];
    for (name, body, want) in cases {
        let h = profile_harness();
        let res = h.up(profile_request(body)).await;
        let mut got = Vec::new();
        for service in &res.services {
            assert_eq!(
                service.action,
                service_action::STARTED,
                "{name}: startup failed: {res:?}"
            );
            got.push(service.service.clone());
        }
        got.sort();
        assert_eq!(got, want, "{name}: started");
    }
}

#[tokio::test]
async fn compose_profiles_persist_through_stop() {
    let h = profile_harness();
    let res = h
        .up(profile_request(
            r#"{"services":["reporting"],"profiles":["extra","base","extra"]}"#,
        ))
        .await;
    assert!(!res.failed(), "up = {res:?}");
    let want = ["base", "extra", "reports", "trends"];
    assert_eq!(h.compose.calls()[0].target.profiles, want);

    // Stop must retain the actual launch selection even after manifest changes.
    h.loader.update(ROOT, |p| {
        p.envs.insert(
            "dev".into(),
            Environment {
                name: "dev".into(),
                compose: vec!["compose.yaml".into()],
                ..Environment::default()
            },
        );
        p.services.get_mut("reporting").unwrap().profiles.clear();
    });
    h.app
        .down(DownRequest {
            project: ROOT.into(),
            ..DownRequest::default()
        })
        .await
        .unwrap();
    for call in h.compose.calls() {
        assert_eq!(call.target.profiles, want, "{} profiles", call.op);
    }
}

#[tokio::test]
async fn legacy_compose_run_reconstructs_profiles() {
    let h = profile_harness();
    let p = profile_project();
    let mut run = blank_run(
        &p.name,
        "reporting",
        ServiceKind::Compose,
        RunState::Running,
    );
    run.env = "dev".into();
    let target = h.app.compose_target_for_run(Some(&p), &run);
    assert_eq!(
        target.profiles,
        ["base", "reports", "trends"],
        "legacy run profiles"
    );
}

#[tokio::test]
async fn restart_profile_selection() {
    let h = profile_harness();
    h.up(profile_request(
        r#"{"services":["all"],"profiles":["trends"]}"#,
    ))
    .await;
    let old_trends = h.run("profiles", "trends");
    let res = h
        .app
        .restart(profile_request(r#"{"services":["all"]}"#))
        .await
        .unwrap();
    assert!(!res.failed(), "restart = {res:?}");
    let got = h.run("profiles", "trends");
    assert!(
        got.pid == old_trends.pid && got.state.active(),
        "unselected gated service changed: {got:?}"
    );
    assert_eq!(
        res.services.len(),
        3,
        "restart selected gated service: {res:?}"
    );

    let res = h
        .app
        .restart(profile_request(
            r#"{"services":["all"],"profiles":["trends"]}"#,
        ))
        .await
        .unwrap();
    assert!(
        !res.failed() && res.services.len() == 5,
        "profile restart = {res:?}"
    );
    assert_ne!(
        h.run("profiles", "trends").pid,
        old_trends.pid,
        "requested gated service was not restarted"
    );
}

#[tokio::test]
async fn restart_rejects_unknown_environment_before_stopping() {
    let h = profile_harness();
    h.up(profile_request(r#"{"services":["ordinary"]}"#)).await;
    let before = h.run("profiles", "ordinary");
    let err = h
        .app
        .restart(profile_request(
            r#"{"services":["ordinary"],"env":"unknown"}"#,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, AppError::Invalid(_)),
        "restart error = {err:?}, want invalid"
    );
    let after = h.run("profiles", "ordinary");
    assert!(
        after.pid == before.pid && after.state.active(),
        "invalid environment stopped service: {after:?}"
    );
}

#[tokio::test]
async fn restart_rejects_invalid_ttl_before_stopping() {
    for ttl in ["nope", "0s", "-1m"] {
        let h = profile_harness();
        h.up(profile_request(r#"{"services":["ordinary"]}"#)).await;
        let before = h.run("profiles", "ordinary");
        let mut req = profile_request(r#"{"services":["ordinary"]}"#);
        req.ttl = ttl.into();
        let err = h.app.restart(req).await.unwrap_err();
        assert!(
            matches!(err, AppError::Invalid(_)),
            "{ttl}: restart error = {err:?}, want invalid"
        );
        let after = h.run("profiles", "ordinary");
        assert!(
            after.pid == before.pid && after.state.active(),
            "{ttl}: invalid TTL stopped service: {after:?}"
        );
    }
}

#[tokio::test]
async fn restart_empty_wildcard_does_not_stop_gated_runs() {
    let h = profile_harness();
    h.loader.update(ROOT, |p| {
        let trends = p.services["trends"].clone();
        p.services = BTreeMap::from([("trends".to_string(), trends)]);
        p.groups = BTreeMap::from([("all".to_string(), vec!["*".to_string()])]);
    });
    h.up(profile_request(r#"{"services":["trends"]}"#)).await;
    let before = h.run("profiles", "trends");
    let res = h
        .app
        .restart(profile_request(r#"{"services":["all"]}"#))
        .await
        .unwrap();
    assert!(res.services.is_empty(), "empty restart = {res:?}");
    let after = h.run("profiles", "trends");
    assert!(
        after.pid == before.pid && after.state.active(),
        "empty wildcard stopped gated service: {after:?}"
    );
}

#[tokio::test]
async fn down_wildcard_and_everywhere_ignore_profiles() {
    let cases = [
        (
            "wildcard group",
            DownRequest {
                services: vec!["all".into()],
                ..DownRequest::default()
            },
        ),
        ("whole project", DownRequest::default()),
        (
            "everywhere",
            DownRequest {
                everywhere: true,
                ..DownRequest::default()
            },
        ),
    ];
    for (name, mut req) in cases {
        let h = profile_harness();
        h.up(profile_request(
            r#"{"services":["all"],"profiles":["trends"]}"#,
        ))
        .await;
        if !req.everywhere {
            req.project = ROOT.into();
        }
        let res = h.app.down(req).await.unwrap();
        assert_eq!(res.stopped.len(), 5, "{name}: down = {res:?}");
        for run in &res.stopped {
            assert!(!run.state.active(), "{name}: still active: {run:?}");
        }
    }
}
