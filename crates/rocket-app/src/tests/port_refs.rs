//! Ports of `port_refs_test.go`.

use crate::AppError;
use crate::testing::*;
use rocket_domain::api::{DownRequest, service_action};
use rocket_domain::ports::ManifestLoader;
use rocket_domain::{Project, ServiceKind};
use std::path::Path;

const ROOT: &str = "/code/refs";

fn port_refs_harness() -> (Harness, Project) {
    let p = rocket_manifest::parse(
        br#"version: 1
name: refs
envs: {dev: {}, smoke: {}}
services:
  api: {run: api-server, cwd: api, ports: {http: {default: 8080, env: PORT}}}
  web:
    task: web:dev
    cwd: web
    env:
      API_URL: "http://127.0.0.1:{api.http}/{api.http}/${api.http}"
      STATIC_VALUE: "literal {value} ${ghost.http}"
"#,
        ROOT,
    )
    .expect("port refs manifest parses");
    let h = Harness::new();
    h.loader.set(p.clone());
    h.probe.set_busy(8080, holder(77, "foreign"));
    (h, p)
}

fn smoke(services: &[&str]) -> rocket_domain::api::UpRequest {
    rocket_domain::api::UpRequest {
        env: "smoke".into(),
        ..req(ROOT, services)
    }
}

fn web_url(h: &Harness) -> String {
    env_map(&h.runner.spec_for("web").env)["API_URL"].clone()
}

#[tokio::test]
async fn up_cross_service_port_references() {
    let (h, p) = port_refs_harness();
    let res = h.up(smoke(&["web"])).await;
    let names: Vec<&str> = res.services.iter().map(|s| s.service.as_str()).collect();
    for svc in &res.services {
        assert_eq!(
            svc.action,
            service_action::STARTED,
            "startup failed: {res:?}"
        );
    }
    assert_eq!(names, ["api", "web"], "provider must start before consumer");
    let env = env_map(&h.runner.spec_for("web").env);
    assert_eq!(env["API_URL"], "http://127.0.0.1:8180/8180/${api.http}");
    assert_eq!(env["STATIC_VALUE"], "literal {value} ${ghost.http}");
    assert_eq!(h.run(&p.name, "api").env, "smoke");
    assert_eq!(
        h.run(&p.name, "web").env,
        "smoke",
        "provider and consumer must use selected environment"
    );
    let manifest = h.loader.load(Path::new(ROOT)).unwrap();
    assert_eq!(
        manifest.services["web"].env["API_URL"],
        "http://127.0.0.1:{api.http}/{api.http}/${api.http}",
        "runtime expansion mutated the manifest"
    );
}

#[tokio::test]
async fn up_port_reference_provider_failure_skips_consumer() {
    let (h, _) = port_refs_harness();
    h.loader.update(ROOT, |p| {
        p.services.get_mut("api").unwrap().ports[0].env.clear()
    });
    let res = h.up(req(ROOT, &["web"])).await;
    let acts = actions(&res);
    assert!(
        acts["api"] == service_action::FAILED
            && acts["web"] == service_action::SKIPPED
            && h.runner.started().is_empty(),
        "provider failure must prevent consumer start: {res:?}"
    );
}

#[tokio::test]
async fn up_port_references_reject_live_environment_mismatch() {
    for name in ["provider", "consumer"] {
        let (h, _) = port_refs_harness();
        let initial: &[&str] = if name == "provider" {
            &["api"]
        } else {
            &["web"]
        };
        h.up(rocket_domain::api::UpRequest {
            env: "dev".into(),
            ..req(ROOT, initial)
        })
        .await;
        if name == "consumer" {
            h.app
                .down(DownRequest {
                    project: ROOT.into(),
                    services: vec!["api".into()],
                    ..DownRequest::default()
                })
                .await
                .unwrap();
            h.up(smoke(&["api"])).await;
        }
        let (started, stopped) = (h.runner.started().len(), h.runner.stopped().len());
        let err = h.app.up(smoke(&["web"])).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, AppError::Invalid(_)) && msg.contains("dev") && msg.contains("smoke"),
            "{name}: environment mismatch error = {msg}"
        );
        assert!(
            h.runner.started().len() == started && h.runner.stopped().len() == stopped,
            "{name}: mismatched up must leave existing services untouched"
        );
    }
}

#[tokio::test]
async fn up_port_references_use_compose_provider_alias() {
    for reuse in [false, true] {
        let (h, _) = port_refs_harness();
        h.loader.update(ROOT, |p| {
            let api = p.services.get_mut("api").unwrap();
            api.kind = ServiceKind::Compose;
            api.run.clear();
            api.compose = "backend".into();
        });
        if reuse {
            h.up(req(ROOT, &["api"])).await;
        }
        let res = h.up(req(ROOT, &["web"])).await;
        let acts = actions(&res);
        assert!(
            !res.failed() && acts["web"] == service_action::STARTED,
            "reuse={reuse}: compose alias must resolve as a live provider: {res:?}"
        );
        if reuse {
            assert_eq!(
                acts["api"],
                service_action::ALREADY_RUNNING,
                "live compose provider must be reused: {res:?}"
            );
        }
        assert_eq!(
            web_url(&h),
            "http://127.0.0.1:8180/8180/${api.http}",
            "reuse={reuse}"
        );
    }
}

#[tokio::test]
async fn up_port_references_use_live_provider_and_preserve_consumer_idempotency() {
    let (h, _) = port_refs_harness();
    h.up(smoke(&["api"])).await;
    let res = h.up(smoke(&["web"])).await;
    let acts = actions(&res);
    assert!(
        acts["api"] == service_action::ALREADY_RUNNING && acts["web"] == service_action::STARTED,
        "live provider should be reused: {res:?}"
    );
    assert_eq!(web_url(&h), "http://127.0.0.1:8180/8180/${api.http}");

    h.app
        .down(DownRequest {
            project: ROOT.into(),
            services: vec!["api".into()],
            ..DownRequest::default()
        })
        .await
        .unwrap();
    h.probe.clear_busy(8080);
    h.up(smoke(&["api"])).await;
    let before = h.runner.started().len();
    let res = h.up(smoke(&["web"])).await;
    assert!(
        actions(&res)["web"] == service_action::ALREADY_RUNNING
            && h.runner.started().len() == before,
        "up must preserve existing consumer: {res:?}"
    );
    let res = h.app.restart(smoke(&["web"])).await.unwrap();
    assert_eq!(
        actions(&res)["web"],
        service_action::STARTED,
        "restart = {res:?}"
    );
    let started = h.runner.started();
    let last_env = env_map(&started.last().unwrap().env);
    assert_eq!(
        last_env["API_URL"], "http://127.0.0.1:8080/8080/${api.http}",
        "restart must refresh consumer URL"
    );
}
