use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use serde_json::Value;

use crate::common::*;

const BEGIN_MARKER: &str = "<!-- rocket:begin -->";

#[test]
fn jobs_and_ai() {
    let Some(e) = setup() else { return };

    step("pipeline success returns the outcome");
    let out = e.json(&[], 0, &["run", "check"]);
    assert!(
        out["status"] == "succeeded" && out["exit_code"] == 0 && out["kind"] == "pipeline",
        "outcome {out}"
    );
    let tail = tail_of(&out, "tail");
    assert!(
        tail_has(&tail, "step-one") && tail_has(&tail, "step-two greeting=hello"),
        "tail {tail:?}"
    );
    assert!(
        std::path::Path::new(str_of(&out, "log_path")).exists(),
        "log {}",
        out["log_path"]
    );

    step("pipeline failure exits 2 and stops at the failing step");
    let out = e.json(&[], 2, &["run", "broken"]);
    let tail = tail_of(&out, "tail");
    assert!(
        out["status"] == "failed" && out["exit_code"] == 3,
        "outcome {out}"
    );
    assert!(
        tail_has(&tail, "failing") && !tail_has(&tail, "never-runs"),
        "tail {tail:?}"
    );

    step("args after -- reach the last step");
    let (stdout, code) = e.rocket(&[], &["run", "echo-args", "--json", "--", "a", "b c"]);
    let out: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("exit {code} err {err}\n{stdout}"));
    assert_eq!(code, 0, "exit {code}\n{stdout}");
    assert!(
        tail_has(&tail_of(&out, "tail"), "args: a b c"),
        "tail {out}"
    );

    step("unknown pipeline without a Taskfile is invalid");
    let body = e.json(&[], 1, &["run", "nope"]);
    assert!(
        body["code"] == "invalid" && str_of(&body, "error").contains("no Taskfile"),
        "body {body}"
    );

    step("setup runs doctor then install");
    let out = e.json(&[], 0, &["setup"]);
    let joined = tail_of(&out, "tail").join("\n");
    let doctor_at = joined.find("doctor-ok");
    let install_at = joined.find("install-ok greeting=hello");
    assert!(
        out["name"] == "setup"
            && doctor_at.is_some()
            && install_at.is_some()
            && doctor_at < install_at,
        "setup {out}"
    );
    let out = e.json(&[], 0, &["doctor"]);
    assert!(
        out["name"] == "doctor" && tail_has(&tail_of(&out, "tail"), "doctor-ok"),
        "doctor {out}"
    );

    step("human run streams the log");
    let (stdout, code) = e.rocket(&[], &["run", "check"]);
    assert!(
        code == 0 && stdout.contains("step-one\n") && stdout.contains("step-two greeting=hello"),
        "exit {code} stdout:\n{stdout}"
    );

    step("detach, list and cancel");
    let job = e.json(&[], 0, &["run", "slow", "--detach"]);
    assert!(
        job["status"] == "running" && !str_of(&job, "id").is_empty(),
        "detached {job}"
    );
    let id = str_of(&job, "id");
    let running = e.wait_job_pgid(id);
    let list = e.json(&[], 0, &["jobs"]);
    let jobs = arr(&list, "jobs");
    assert!(
        !jobs.is_empty() && jobs[0]["id"] == id && jobs[0]["status"] == "running",
        "jobs {list}"
    );
    let canceled = e.json(&[], 0, &["job", "cancel", id]);
    assert_eq!(canceled["status"], "canceled", "canceled {canceled}");
    assert!(
        group_gone(running["pgid"].as_i64().unwrap()),
        "process group {} still alive",
        running["pgid"]
    );
    let logs = e.json(&[], 0, &["job", id, "logs"]);
    assert!(
        tail_has(&strings(&logs, "lines"), "slow-start"),
        "logs {logs}"
    );

    step("deploy confirmation gate exits 3");
    let body = e.json(&[], 3, &["deploy", "stage"]);
    assert_eq!(body["code"], "confirmation_required", "body {body}");
    let out = e.json(&[], 0, &["deploy", "stage", "--yes"]);
    assert!(
        out["kind"] == "deploy" && tail_has(&tail_of(&out, "tail"), "deploying greeting=hello"),
        "deploy {out}"
    );
    let agent = [("ROCKET_OWNER", "agent:t9")];
    e.json(&agent, 3, &["deploy", "stage", "--yes"]);
    let allowed = [("ROCKET_OWNER", "agent:t9"), ("ROCKET_ALLOW_DEPLOY", "1")];
    let out = e.json(&allowed, 0, &["deploy", "stage", "--yes"]);
    assert_eq!(out["status"], "succeeded", "allowed agent deploy {out}");

    step("status summary and owner cleanup of jobs");
    e.json(&[], 0, &["up", "static"]);
    let job = e.json(
        &[("ROCKET_OWNER", "agent:t3")],
        0,
        &["run", "slow", "--detach"],
    );
    let job_id = str_of(&job, "id");
    let running = e.wait_job_pgid(job_id);
    let sum = e.json(&[], 0, &["status"]);
    let sum_jobs = arr(&sum, "jobs");
    assert!(
        sum["project"]["name"] == "rocket-fixture"
            && sum_jobs.len() == 1
            && sum_jobs[0]["id"] == job_id
            && sum["conflicts"].is_array(),
        "summary {sum}"
    );
    let states: std::collections::HashMap<String, String> = arr(&sum, "services")
        .iter()
        .map(|r| {
            (
                str_of(r, "service").to_string(),
                str_of(r, "state").to_string(),
            )
        })
        .collect();
    assert!(
        states["static"] == "running" && states["web"] == "stopped",
        "service states {states:?}"
    );
    let down = e.json(&[], 0, &["down", "--owner", "agent:t3"]);
    assert!(
        strings(&down, "canceled_jobs") == [job_id] && arr(&down, "stopped").is_empty(),
        "owner down {down}"
    );
    assert!(
        group_gone(running["pgid"].as_i64().unwrap()),
        "agent job still running"
    );
    e.json(&[], 0, &["down", "--all"]);

    step("tcp listener requires the bearer token");
    let path = e.home.join("daemon.json");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "daemon.json perm {mode:o}");
    let info = e.daemon_info();
    let http = str_of(&info, "http");
    assert!(
        http.starts_with("http://127.0.0.1:")
            && str_of(&info, "token").len() == 64
            && !str_of(&info, "rocket_bin").is_empty(),
        "info {info}"
    );
    let addr = http.trim_start_matches("http://");
    let get = |token: Option<&str>| -> u16 {
        let (code, mut r) = http_get(addr, "/v1/health", token, Duration::from_secs(5)).unwrap();
        let mut sink = Vec::new();
        let _ = r.read_to_end(&mut sink);
        code
    };
    assert_eq!(get(None), 401, "no token");
    assert_eq!(get(Some(&"0".repeat(64))), 401, "wrong token");
    assert_eq!(get(Some(str_of(&info, "token"))), 200, "valid token");

    step("daemon crash: vanished job is marked lost");
    let job = e.json(&[], 0, &["run", "slow", "--detach"]);
    let job_id = str_of(&job, "id");
    let running = e.wait_job_pgid(job_id);
    let st = e.json(&[], 0, &["daemon", "status"]);
    sigkill(st["info"]["pid"].as_i64().unwrap() as i32);
    sigkill_group(running["pgid"].as_i64().unwrap());
    wait_for(Duration::from_secs(5), "daemon gone", || {
        e.rocket(&[], &["daemon", "status"]).1 == 1
    });
    e.json(&[], 0, &["daemon", "start"]);
    let got = e.json(&[], 0, &["job", job_id]);
    assert_eq!(got["status"], "lost", "job after crash {got}");

    step("init on an autodropshipping-shaped project");
    let dir = e.home.join("autodropshipping");
    copy_tree(&testdata().join("init/autodropshipping"), &dir);
    let (out, code) = e.rocket_at(&dir, &[], &["init", "--json"]);
    let res: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    assert!(
        code == 0 && res["written"] == true,
        "init exit {code}\n{out}"
    );
    assert!(dir.join("rocket.yaml").exists(), "rocket.yaml missing");
    let (out, code) = e.rocket_at(&dir, &[], &["init", "--json"]);
    assert!(
        code == 1 && out.contains("already exists"),
        "second init must refuse: exit {code}\n{out}"
    );
    let (_, code) = e.rocket_at(&dir, &[], &["init", "--force"]);
    assert_eq!(code, 0, "init --force exit {code}");
    let (out, code) = e.rocket_at(&dir, &[], &["init", "--print"]);
    assert!(
        code == 0 && out.contains("compose: postgres"),
        "init --print exit {code}\n{out}"
    );
    // The daemon loads and validates the generated manifest.
    let ps = e.json(&[], 0, &["ps", "-p", dir.to_str().unwrap()]);
    let stopped: std::collections::HashMap<String, bool> = arr(&ps, "services")
        .iter()
        .map(|r| (str_of(r, "service").to_string(), r["state"] == "stopped"))
        .collect();
    assert!(
        stopped.get("postgres") == Some(&true) && stopped.get("cp-dev") == Some(&true),
        "services from generated manifest {ps}"
    );

    step("agent install with HOME override is idempotent");
    let home = e.home.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let henv = [("HOME", home.to_str().unwrap())];
    let (out, code) = e.rocket_at(
        &e.project,
        &henv,
        &["agent", "install", "--global", "--json"],
    );
    let res: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    assert!(
        code == 0 && arr(&res, "actions").len() == 3,
        "install exit {code}\n{out}"
    );
    assert!(
        home.join(".claude/skills/rocket/SKILL.md").exists(),
        "global skill missing"
    );
    for f in ["AGENTS.md", "CLAUDE.md"] {
        let data = std::fs::read_to_string(e.project.join(f)).unwrap_or_default();
        assert_eq!(data.matches(BEGIN_MARKER).count(), 1, "{f}:\n{data}");
    }
    let (out, _) = e.rocket_at(
        &e.project,
        &henv,
        &["agent", "install", "--global", "--json"],
    );
    let res: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    for a in arr(&res, "actions") {
        assert_eq!(a["action"], "unchanged", "second install changed {a}");
    }
    let (_, code) = e.rocket_at(
        &e.project,
        &henv,
        &["agent", "install", "--target", "claude"],
    );
    assert_eq!(code, 0, "project skill install failed");
    assert!(
        e.project.join(".claude/skills/rocket/SKILL.md").exists(),
        "project skill missing"
    );

    step("daemon stop removes daemon.json");
    let (_, code) = e.rocket(&[], &["daemon", "stop"]);
    assert_eq!(code, 0, "daemon stop exit {code}");
    assert!(
        !e.home.join("daemon.json").exists(),
        "daemon.json left behind"
    );
}
