use serde_json::Value;

use crate::common::*;

#[test]
fn job_selection_metadata_and_down_guidance() {
    let Some(e) = setup() else { return };
    e.write_manifest(
        r#"version: 1
name: rocket-fixture
default_env: dev
envs:
  stage: {deploy: {run: "echo harmless-deploy-$COMPOSE_PROJECT_NAME", confirm: true}}
  smoke: {profiles: [smoke]}
  dev: {profiles: [base]}
services:
  core: {run: "exec sleep 600", depends_on: [dependency]}
  dependency: {run: "exec sleep 600", profiles: [internal]}
  trends: {run: "exec sleep 600", profiles: [trends]}
  smoke: {run: "exec sleep 600", profiles: [smoke]}
groups: {all: ["*"]}
setup:
  doctor: {run: "echo doctor-$COMPOSE_PROJECT_NAME"}
  install: {run: "echo install-$COMPOSE_PROJECT_NAME"}
  migrate: {run: "echo migrate-$COMPOSE_PROJECT_NAME"}
pipelines:
  selected: {needs: [all], steps: [{run: "echo pipeline-$COMPOSE_PROJECT_NAME"}]}
  alpha: [{run: "echo alpha"}]
"#,
    );

    step("status declares sorted available choices");
    let wire = e.json(&[], 0, &["status"]);
    let project = &wire["project"];
    assert!(
        strings(project, "envs") == ["dev", "smoke", "stage"]
            && strings(project, "pipelines") == ["alpha", "selected"]
            && strings(project, "deploy_envs") == ["stage"],
        "declared choices = {project}"
    );

    step("pipeline env and repeatable profile flags select prerequisites");
    let outcome = e.json(
        &[],
        0,
        &[
            "run",
            "selected",
            "--env",
            "smoke",
            "--profile",
            "trends",
            "--profile",
            "extra",
            "--profile",
            "extra",
        ],
    );
    assert!(
        outcome["status"] == "succeeded"
            && tail_has(
                &tail_of(&outcome, "tail"),
                "pipeline-rocket-rocket-fixture-smoke"
            ),
        "selected job outcome = {outcome}"
    );
    let wire = e.json(&[], 0, &["job", str_of(&outcome, "job")]);
    assert!(
        wire["env"] == "smoke" && strings(&wire, "profiles") == ["extra", "smoke", "trends"],
        "persisted job selection = {wire}"
    );
    for (name, run) in e.ps() {
        assert!(
            run["state"] == "running" && run["env"] == "smoke" && run["owner"] == "user",
            "selected prerequisite {name} = {run}"
        );
    }
    let down = e.json(&[], 0, &["down", "--all"]);
    assert_eq!(
        arr(&down, "stopped").len(),
        4,
        "stop-all omitted profile prerequisite: {down}"
    );

    step("setup commands share environment and profile flags");
    for args in [
        &["setup", "doctor"][..],
        &["doctor"],
        &["install"],
        &["migrate"],
    ] {
        let mut a = args.to_vec();
        a.extend(["--env", "smoke", "--profile", "extra", "--profile", "extra"]);
        let outcome = e.json(&[], 0, &a);
        assert!(
            tail_has(&tail_of(&outcome, "tail"), "rocket-rocket-fixture-smoke"),
            "setup environment {args:?}: {outcome}"
        );
        let wire = e.json(&[], 0, &["job", str_of(&outcome, "job")]);
        assert_eq!(
            strings(&wire, "profiles"),
            ["extra", "smoke"],
            "setup persisted profiles {args:?}"
        );
    }

    step("harmless deploy retains confirmation gate");
    let body = e.json(&[], 3, &["deploy", "stage"]);
    assert_eq!(
        body["code"], "confirmation_required",
        "deploy confirmation = {body}"
    );
    let outcome = e.json(&[], 0, &["deploy", "stage", "--yes"]);
    assert!(
        tail_has(
            &tail_of(&outcome, "tail"),
            "harmless-deploy-rocket-rocket-fixture-stage"
        ),
        "harmless deploy outcome = {outcome}"
    );
    let before = e.json(&[], 0, &["jobs"]);
    for flag in ["--profile", "--env"] {
        let (_, code) = e.rocket(&[], &["deploy", "stage", "--yes", flag, "extra"]);
        assert_eq!(
            code, 1,
            "unsupported deploy flag {flag} accepted: exit={code}"
        );
    }
    let after = e.json(&[], 0, &["jobs"]);
    assert_eq!(
        arr(&after, "jobs").len(),
        arr(&before, "jobs").len(),
        "unsupported deploy flag submitted a job"
    );

    step("outside-project JSON guidance preserves error and exit");
    for explicit in [false, true] {
        let missing = e.home.join("missing");
        let mut args = vec!["down", "--all", "--json"];
        if explicit {
            args.extend(["-p", missing.to_str().unwrap()]);
        }
        let (out, code) = e.rocket_at(&e.home, &[], &args);
        let body: Result<serde_json::Map<String, Value>, _> = serde_json::from_str(&out);
        let ok = matches!(&body, Ok(b)
            if code == 1
                && b.len() == 2
                && b["code"] == "error"
                && b["error"].as_str().unwrap_or("").contains("no rocket.yaml found")
                && b["error"].as_str().unwrap_or("").contains("--everywhere") != explicit);
        assert!(
            ok,
            "outside guidance explicit={explicit}: exit={code}, body={out}"
        );
    }
    let bad = e.home.join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("rocket.yaml"), "version: 99\n").unwrap();
    let (out, code) = e.rocket_at(
        &e.home,
        &[],
        &["down", "--all", "-p", bad.to_str().unwrap(), "--json"],
    );
    let body: Result<serde_json::Map<String, Value>, _> = serde_json::from_str(&out);
    let ok = matches!(&body, Ok(b)
        if code == 1
            && b.len() == 2
            && b["code"] == "invalid"
            && !b["error"].as_str().unwrap_or("").contains("--everywhere"));
    assert!(ok, "malformed project guidance: exit={code}, body={out}");
}
