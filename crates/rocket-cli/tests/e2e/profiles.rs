use crate::common::*;

#[test]
fn cli_profiles_and_stop_all() {
    let Some(e) = setup() else { return };
    e.write_manifest(
        r#"version: 1
name: rocket-fixture
services:
  static:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    ports: {http: {default: 18431, env: PORT}}
    health: {http: /, timeout: 20s}
  trends:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    profiles: [trends]
    ports: {http: {default: 18432, env: PORT}}
    health: {http: /, timeout: 20s}
  reports:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    profiles: [reports]
    ports: {http: {default: 18433, env: PORT}}
    health: {http: /, timeout: 20s}
groups: {all: ["*"]}
"#,
    );
    let up = e.json(
        &[],
        0,
        &[
            "up",
            "all",
            "--profile",
            "trends",
            "--profile",
            "reports",
            "--profile",
            "trends",
        ],
    );
    assert!(
        arr(&up, "services").len() == 3 && !failed(&up),
        "repeated profiles startup = {up}"
    );
    let reports = e.ps()["reports"].clone();
    let restart = e.json(
        &[],
        0,
        &[
            "restart",
            "all",
            "--profile",
            "trends",
            "--profile",
            "trends",
        ],
    );
    assert!(
        arr(&restart, "services").len() == 2 && !failed(&restart),
        "profile restart = {restart}"
    );
    let after = e.ps()["reports"].clone();
    assert!(
        after["pid"] == reports["pid"] && is_active(str_of(&after, "state")),
        "restart touched unselected reports: {after}"
    );
    let down = e.json(&[], 0, &["down", "--all"]);
    let stopped = arr(&down, "stopped");
    assert_eq!(
        stopped.len(),
        3,
        "down --all filtered active profiles: {down}"
    );
    for run in &stopped {
        assert!(
            group_gone(run["pgid"].as_i64().unwrap_or(0)),
            "test-owned process group {} remains",
            run["pgid"]
        );
    }
    for port in [18431, 18432, 18433] {
        assert!(port_free(port), "test-owned listener {port} remains");
    }
    let up = e.json(&[], 0, &["up", "all"]);
    let svcs = arr(&up, "services");
    assert!(
        svcs.len() == 1 && svcs[0]["service"] == "static",
        "default wildcard = {up}"
    );
    let up = e.json(&[], 0, &["up", "*", "--profile", "reports"]);
    assert!(
        arr(&up, "services").len() == 2 && !failed(&up),
        "requested wildcard profile = {up}"
    );
    let down = e.json(&[], 0, &["down", "--everywhere"]);
    assert_eq!(
        arr(&down, "stopped").len(),
        2,
        "down --everywhere filtered active profiles: {down}"
    );
    let ports = e.json(&[], 0, &["ports"]);
    assert!(
        arr(&ports, "ports").is_empty(),
        "test leases remain: {ports}"
    );
}

/// Mirrors Go's `UpResult.Failed()`.
fn failed(up: &serde_json::Value) -> bool {
    arr(up, "services").iter().any(|s| s["action"] == "failed")
}

/// Mirrors Go's `RunState.Active()`.
fn is_active(state: &str) -> bool {
    matches!(state, "starting" | "running" | "stopping")
}
