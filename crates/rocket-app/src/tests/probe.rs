//! Ports of `probe_test.go`.

use crate::testing::*;
use rocket_domain::api::service_action;
use rocket_domain::ports::Store;
use rocket_domain::{HealthCheck, ServiceKind};

const ROOT: &str = "/code/probes";

fn check(kind: &str, port: u16, path: &str) -> HealthCheck {
    HealthCheck {
        kind: kind.into(),
        port,
        path: path.into(),
    }
}

#[tokio::test]
async fn up_probe_settings_preserve_leases_and_remapping() {
    // (name, kind, http probe attr, health, expected checks)
    let cases: [(&str, &str, &str, &str, Vec<HealthCheck>); 4] = [
        (
            "mixed local",
            "run: dev",
            "",
            "http: /ready",
            vec![check("http", 8180, "/ready")],
        ),
        (
            "disabled local",
            "task: dev",
            ", probe: false",
            "http: /ready",
            vec![],
        ),
        (
            "disabled compose",
            "compose: backend",
            ", probe: false",
            "tcp: true",
            vec![],
        ),
        (
            "mixed compose explicit tcp",
            "compose: backend",
            "",
            "tcp: true",
            vec![check("tcp", 8180, "")],
        ),
    ];
    for (name, kind, http_probe, health, want_checks) in cases {
        let yaml = format!(
            "version: 1\nname: probes\nenvs: {{dev: {{compose: [compose.yaml]}}}}\ndefault_env: dev\nservices:\n  api:\n    {kind}\n    ports:\n      debug: {{default: 9000, env: DEBUG_PORT, probe: false}}\n      http: {{default: 8080, env: PORT{http_probe}}}\n    health: {{{health}, timeout: 10ms}}\n"
        );
        let p =
            rocket_manifest::parse(yaml.as_bytes(), ROOT).unwrap_or_else(|e| panic!("{name}: {e}"));
        let is_compose = p.services["api"].kind == ServiceKind::Compose;
        let h = Harness::new();
        h.loader.set(p);
        h.probe.set_busy(9000, holder(99, "foreign"));
        h.probe.set_busy(8080, holder(98, "foreign"));
        h.health.fail_port(9100);
        if !http_probe.is_empty() {
            h.health.fail_port(8180);
        }
        let res = h.up(req(ROOT, &[])).await;
        assert!(
            !res.failed()
                && res.services.len() == 1
                && res.services[0].action == service_action::STARTED,
            "{name}: up = {res:?}"
        );
        let result = &res.services[0];
        assert!(
            result.ports["debug"] == 9100
                && result.ports["http"] == 8180
                && result.remaps.len() == 2,
            "{name}: ports/remaps = {result:?}"
        );
        let leases = h.store.list_leases().unwrap();
        assert!(
            leases.len() == 2 && leases[0].port == 8180 && leases[1].port == 9100,
            "{name}: leases = {leases:?}"
        );
        assert_eq!(h.health.checks(), want_checks, "{name}: health checks");
        let env = if is_compose {
            assert_eq!(
                h.compose.ops(),
                ["up:backend"],
                "{name}: compose readiness must still run"
            );
            env_map(&h.compose.calls()[0].target.env)
        } else {
            env_map(&h.runner.spec_for(ROOT).env)
        };
        assert!(
            env["DEBUG_PORT"] == "9100" && env["PORT"] == "8180",
            "{name}: injected ports = {env:?}"
        );
    }
}

#[tokio::test]
async fn up_disabled_probe_cannot_bypass_port_conflict() {
    let p = rocket_manifest::parse(
        b"version: 1\nname: probes\nservices: {api: {run: dev, ports: {http: {default: 8080, probe: false}}}}",
        ROOT,
    )
    .unwrap();
    let h = Harness::new();
    h.loader.set(p);
    h.probe.set_busy(8080, holder(99, ""));
    let res = h.up(req(ROOT, &[])).await;
    assert!(
        res.failed() && h.runner.started().is_empty(),
        "disabled probe bypassed lease conflict: {res:?}"
    );
    let leases = h.store.list_leases().unwrap();
    assert!(leases.is_empty(), "leases = {leases:?}");
}
