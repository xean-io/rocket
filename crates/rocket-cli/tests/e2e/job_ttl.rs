use std::io::BufRead;
use std::time::Duration;

use serde_json::Value;

use crate::common::*;

#[test]
fn job_ttl_real_steps_startup_and_daemon_restart() {
    let Some(e) = setup() else { return };
    e.write_manifest(
        r#"version: 1
name: rocket-fixture
services:
  waiting:
    run: "exec sleep 600"
    ports: {http: {default: 18432, env: PORT}}
    health: {tcp: true, timeout: 20s}
envs:
  dev: {}
  stage: {deploy: {run: "echo harmless-deploy", confirm: true}}
setup: {doctor: {run: "echo doctor-ok"}}
pipelines:
  slow: [{run: "exec sleep 600"}]
  trailing:
    - run: >-
        exec python3 -c 'import signal,sys,time; signal.signal(signal.SIGTERM, lambda *_:(sys.stdout.write("trailing-partial"),sys.stdout.flush(),sys.exit(0))); print("ready", flush=True); time.sleep(600)'
  blocked: {needs: [waiting], steps: [{run: "echo never-step"}]}
"#,
    );

    step("step expiry drains partial output before one terminal event");
    let started = e.json(&[], 0, &["run", "trailing", "--ttl", "1s", "--detach"]);
    let id = str_of(&started, "id").to_string();
    let running = e.wait_job_pgid(&id);
    let info = e.daemon_info();
    let addr = str_of(&info, "http")
        .trim_start_matches("http://")
        .to_string();
    let (status, reader) = http_get(
        &addr,
        &format!("/v1/jobs/{id}/logs?follow=true&tail=-1"),
        Some(str_of(&info, "token")),
        Duration::from_secs(8),
    )
    .expect("follow request");
    assert_eq!(status, 200, "follow HTTP status");
    let (mut partial, mut terminal) = (0, 0);
    for line in reader.lines() {
        let line = line.expect("SSE stream error");
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        let event: Value = serde_json::from_str(data)
            .unwrap_or_else(|err| panic!("interleaved SSE frame: {line:?}, {err}"));
        assert_eq!(terminal, 0, "event after terminal: {event}");
        if event["type"] == "job.log" && event["line"] == "trailing-partial" {
            partial += 1;
        }
        if event["type"] == "job.state" && is_terminal(str_of(&event, "status")) {
            terminal += 1;
            assert!(
                partial == 1
                    && event["status"] == "canceled"
                    && event["job"]["error"] == "ttl expired",
                "terminal preceded trailing output or lost expiry reason: {event}, partial={partial}"
            );
        }
    }
    assert!(
        partial == 1 && terminal == 1,
        "stream result: partial={partial}, terminal={terminal}"
    );
    e.wait_expired_job(&id);
    assert!(
        group_gone(running["pgid"].as_i64().unwrap()),
        "expired step group {} remains",
        running["pgid"]
    );

    step("startup expiry shares deadline and releases attempted listener lease");
    let job = e.json(&[], 0, &["run", "blocked", "--ttl", "1s", "--detach"]);
    let mut waiting = Value::Null;
    wait_for(Duration::from_secs(3), "prerequisite startup", || {
        waiting = e.ps()["waiting"].clone();
        waiting["state"] == "starting" && waiting["pgid"].as_i64().unwrap_or(0) > 0
    });
    assert!(
        !job["expires_at"].is_null()
            && !waiting["expires_at"].is_null()
            && parse_time(&waiting, "expires_at") == parse_time(&job, "expires_at"),
        "prerequisite deadline differs: job={job}, run={waiting}"
    );
    let job_id = str_of(&job, "id");
    let finished = e.wait_expired_job(job_id);
    assert!(
        finished["step"].as_i64().unwrap_or(0) == 0
            && group_gone(waiting["pgid"].as_i64().unwrap()),
        "startup expiry leaked process or ran steps: {finished}, run={waiting}"
    );
    let logs = e.json(&[], 0, &["job", job_id, "logs"]);
    assert!(
        !tail_has(&strings(&logs, "lines"), "never-step"),
        "steps ran after failed startup: {logs}"
    );
    let ports = e.json(&[], 0, &["ports"]);
    assert!(
        arr(&ports, "ports").is_empty(),
        "expired startup retained lease: {ports}"
    );

    step("restart cancels expired no PID job and retains adopted deadline");
    let startup = e.json(&[], 0, &["run", "blocked", "--ttl", "1s", "--detach"]);
    let step_job = e.json(&[], 0, &["run", "slow", "--ttl", "4s", "--detach"]);
    let step_id = str_of(&step_job, "id");
    let running = e.wait_job_pgid(step_id);
    let (_, code) = e.rocket(&[], &["daemon", "stop"]);
    assert_eq!(code, 0, "stop test daemon: exit {code}");
    assert!(
        !group_gone(running["pgid"].as_i64().unwrap()),
        "daemon shutdown stopped retained job"
    );
    let startup_deadline = parse_time(&startup, "expires_at");
    wait_for(Duration::from_secs(3), "persisted startup deadline", || {
        time::OffsetDateTime::now_utc() > startup_deadline
    });
    e.json(&[], 0, &["daemon", "start"]);
    let finished = e.wait_expired_job(str_of(&startup, "id"));
    assert_eq!(
        finished["step"].as_i64().unwrap_or(0),
        0,
        "expired no PID job ran a step: {finished}"
    );
    let adopted = e.json(&[], 0, &["job", step_id]);
    assert!(
        adopted["status"] == "running"
            && !adopted["expires_at"].is_null()
            && parse_time(&adopted, "expires_at") == parse_time(&step_job, "expires_at"),
        "restart renewed deadline or failed adoption: before={step_job}, after={adopted}"
    );
    e.wait_expired_job(step_id);
    assert!(
        group_gone(running["pgid"].as_i64().unwrap()),
        "adopted expired job group remains"
    );

    step("blocking run and shared job flags preserve exit codes");
    let outcome = e.json(&[], 2, &["run", "slow", "--ttl", "100ms"]);
    assert!(
        outcome["status"] == "canceled" && outcome["error"] == "ttl expired",
        "blocking expiry outcome: {outcome}"
    );
    for args in [
        &["doctor", "--ttl", "1s"][..],
        &["deploy", "stage", "--yes", "--ttl", "1s"],
    ] {
        let outcome = e.json(&[], 0, args);
        let job = e.json(&[], 0, &["job", str_of(&outcome, "job")]);
        assert!(
            !job["expires_at"].is_null()
                && parse_time(&job, "expires_at") - parse_time(&job, "started_at")
                    == time::Duration::seconds(1),
            "shared TTL flag not persisted for {args:?}: {job}"
        );
    }
    let body = e.json(&[], 1, &["run", "slow", "--ttl", "0s"]);
    assert_eq!(
        body["code"], "invalid",
        "invalid TTL error shape changed: {body}"
    );
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "canceled" | "lost")
}
