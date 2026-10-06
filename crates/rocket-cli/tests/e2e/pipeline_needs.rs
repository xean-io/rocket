use std::time::Duration;

use crate::common::*;

#[test]
fn pipeline_needs_start_and_cancel_real_prerequisites() {
    let Some(e) = setup() else { return };
    e.write_manifest(
        r#"version: 1
name: rocket-fixture
services:
  static:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    ports: {http: {default: 18431, env: PORT}}
    health: {http: /, timeout: 20s}
  waiting:
    run: "exec sleep 600"
    ports: {http: {default: 18432, env: PORT}}
    health: {tcp: true, timeout: 20s}
groups: {infra: [static]}
pipelines:
  check: {needs: [infra], steps: [{run: "echo step-ready compose=$COMPOSE_PROJECT_NAME"}]}
  blocked: {needs: [waiting], steps: [{run: "echo never-step"}]}
"#,
    );
    let outcome = e.json(&[("ROCKET_OWNER", "agent:check")], 0, &["run", "check"]);
    assert!(
        outcome["status"] == "succeeded"
            && http_ok(18431)
            && strings(&outcome, "tail")
                .join("\n")
                .contains("step-ready compose=rocket-rocket-fixture-dev"),
        "pipeline prerequisite outcome: {outcome}"
    );
    let run = e.ps()["static"].clone();
    assert!(
        run["owner"] == "agent:check" && run["state"] == "running",
        "prerequisite did not remain running with job owner: {run}"
    );
    let job = e.json(&[], 0, &["run", "blocked", "--detach"]);
    let mut waiting = serde_json::Value::Null;
    wait_for(
        Duration::from_secs(5),
        "blocked prerequisite startup",
        || {
            waiting = e.ps()["waiting"].clone();
            waiting["state"] == "starting" && waiting["pgid"].as_i64().unwrap_or(0) > 0
        },
    );
    let id = str_of(&job, "id");
    let canceled = e.json(&[], 0, &["job", "cancel", id]);
    assert!(
        canceled["status"] == "canceled"
            && canceled["step"].as_i64().unwrap_or(0) == 0
            && group_gone(waiting["pgid"].as_i64().unwrap()),
        "prerequisite cancellation failed: {canceled}, prerequisite={waiting}"
    );
    let logs = e.json(&[], 0, &["job", id, "logs"]);
    assert!(
        !strings(&logs, "lines").join("\n").contains("never-step"),
        "pipeline steps ran despite canceled prerequisite: {logs}"
    );
    assert!(
        http_ok(18431),
        "canceling another job stopped the ready prerequisite"
    );
    e.json(&[], 0, &["down", "--all"]);
    let ports = e.json(&[], 0, &["ports"]);
    assert!(
        arr(&ports, "ports").is_empty(),
        "test-owned leases remain: {ports}"
    );
}
