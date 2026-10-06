//! Port of domain/health_test.go.

use rocket_domain::{HealthCheck, Service, ServiceKind, health_check_for};
use std::collections::BTreeMap;
use std::time::Duration;

fn ports() -> BTreeMap<String, u16> {
    [("debug", 9100), ("http", 8180), ("unknown", 9999)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn tcp(port: u16) -> Option<HealthCheck> {
    Some(HealthCheck {
        kind: "tcp".into(),
        port,
        path: String::new(),
    })
}

#[test]
fn health_check_for_probe_selection() {
    let cases: Vec<(&str, &str, Option<HealthCheck>)> = vec![
        (
            "legacy default",
            r#"{"kind":"run","ports":[{"name":"debug"},{"name":"http"}]}"#,
            tcp(9100),
        ),
        (
            "explicit true",
            r#"{"kind":"task","ports":[{"name":"debug","probe":true}]}"#,
            tcp(9100),
        ),
        (
            "first eligible",
            r#"{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http"}]}"#,
            tcp(8180),
        ),
        (
            "http on first eligible",
            r#"{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http"}],"health":{"http":"/ready"}}"#,
            Some(HealthCheck {
                kind: "http".into(),
                port: 8180,
                path: "/ready".into(),
            }),
        ),
        (
            "explicit eligible",
            r#"{"kind":"run","ports":[{"name":"debug"},{"name":"http","probe":true}],"health":{"port":"http"}}"#,
            tcp(8180),
        ),
        (
            "all disabled",
            r#"{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http","probe":false}],"health":{"http":"/ready"}}"#,
            None,
        ),
        (
            "explicit disabled is never probed",
            r#"{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http"}],"health":{"port":"debug"}}"#,
            None,
        ),
        (
            "unknown explicit port",
            r#"{"kind":"run","ports":[{"name":"debug"}],"health":{"port":"unknown"}}"#,
            None,
        ),
        (
            "compose owns default readiness",
            r#"{"kind":"compose","ports":[{"name":"http"}]}"#,
            None,
        ),
        (
            "compose explicit tcp eligible",
            r#"{"kind":"compose","ports":[{"name":"debug","probe":false},{"name":"http"}],"health":{"tcp":true}}"#,
            tcp(8180),
        ),
        (
            "compose tcp disabled",
            r#"{"kind":"compose","ports":[{"name":"http","probe":false}],"health":{"tcp":true}}"#,
            None,
        ),
        ("no ports", r#"{"kind":"run"}"#, None),
    ];
    for (name, json, want) in cases {
        let svc: Service = serde_json::from_str(json).unwrap();
        assert_eq!(health_check_for(&svc, &ports()), want, "{name}");
    }
    let svc = Service {
        kind: ServiceKind::Run,
        ports: vec![rocket_domain::PortSpec {
            name: "http".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(
        health_check_for(&svc, &BTreeMap::new()),
        None,
        "unresolved port must not be probed"
    );
}

#[test]
fn health_timeout_defaults_when_unset_or_non_positive() {
    let mut svc = Service::default();
    assert_eq!(svc.health_timeout(), Duration::from_secs(60));
    svc.health = Some(Default::default());
    assert_eq!(svc.health_timeout(), Duration::from_secs(60));
    svc.health.as_mut().unwrap().timeout = Duration::from_secs(5);
    assert_eq!(svc.health_timeout(), Duration::from_secs(5));
}
