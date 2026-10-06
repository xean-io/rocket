//! JSON wire compatibility with Go's encoding/json. Literals are what the Go
//! daemon writes: struct field order, `omitempty` applied, RFC3339Nano times,
//! sorted map keys.

use rocket_domain::{
    Deploy, Environment, Event, Health, HealthSpec, Job, JobKind, JobStatus, Lease, PortHolder,
    PortRemap, PortSpec, Project, ProjectRef, Run, RunState, Service, ServiceKind, Step,
    event_type,
};
use serde::{Serialize, de::DeserializeOwned};
use time::macros::datetime;

/// Decode, re-encode and require byte equality with the Go-formatted input.
fn roundtrip<T: Serialize + DeserializeOwned>(go_json: &str) -> T {
    let v: T = serde_json::from_str(go_json).expect("decode");
    assert_eq!(serde_json::to_string(&v).expect("encode"), go_json);
    v
}

#[test]
fn run_matches_go_encoding() {
    let json = r#"{"project":"rocket-fixture","service":"static","env":"dev","kind":"run","state":"running","health":"healthy","pid":8635,"pgid":8635,"owner":"user","started_at":"2026-10-05T00:14:10.938611Z","ports":{"http":18431},"log_path":"/Users/me/.rocket/logs/rocket-fixture/static.log"}"#;
    let run: Run = roundtrip(json);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.health, Health::Healthy);
    assert_eq!(run.kind, ServiceKind::Run);
    assert_eq!(
        run.started_at,
        Some(datetime!(2026-10-05 00:14:10.938611 UTC))
    );
    assert_eq!(run.ports["http"], 18431);
}

#[test]
fn run_from_api_readme_literal() {
    // Copied from internal/adapters/api/README.md ("Run"): explicit nulls and
    // empty strings decode, and re-encoding applies omitempty.
    let readme = r#"{
      "project": "rocket-fixture", "service": "static", "env": "dev", "kind": "run",
      "state": "running", "health": "healthy",
      "pid": 8635, "pgid": 8635,
      "compose_project": "", "container_id": "",
      "owner": "user", "expires_at": null,
      "started_at": "2026-10-05T00:14:10.938611Z", "stopped_at": null, "exit_code": null,
      "ports": { "http": 18431 },
      "log_path": "/Users/me/.rocket/logs/rocket-fixture/static.log",
      "error": ""
    }"#;
    let run: Run = serde_json::from_str(readme).unwrap();
    assert_eq!(
        serde_json::to_string(&run).unwrap(),
        r#"{"project":"rocket-fixture","service":"static","env":"dev","kind":"run","state":"running","health":"healthy","pid":8635,"pgid":8635,"owner":"user","started_at":"2026-10-05T00:14:10.938611Z","ports":{"http":18431},"log_path":"/Users/me/.rocket/logs/rocket-fixture/static.log"}"#
    );
}

#[test]
fn run_keeps_zero_valued_pointers_and_trims_fraction() {
    // A pointer to 0 is not "empty" in Go: exit_code 0 must survive.
    let json = r#"{"project":"p","service":"s","env":"dev","kind":"compose","state":"exited","health":"unknown","compose_project":"rocket-p-dev","profiles":["a","b"],"container_id":"abc","expires_at":"2026-10-05T01:00:00Z","started_at":"2026-10-05T00:00:00.5-05:00","stopped_at":"2026-10-05T00:00:01.000000001Z","exit_code":0}"#;
    let run: Run = roundtrip(json);
    assert_eq!(run.exit_code, Some(0));
}

#[test]
fn job_matches_go_encoding_and_readme() {
    let json = r#"{"id":"j3f9a0c12be","project":"nuvara","name":"ci","kind":"pipeline","env":"dev","profiles":["base","trends"],"owner":"agent:claude-123","steps":[{"task":"lint"},{"run":"go test ./..."}],"args":["-v"],"status":"failed","step":2,"pid":9120,"pgid":9120,"exit_code":1,"started_at":"2026-10-05T00:14:10Z","finished_at":"2026-10-05T00:14:42Z","duration_ms":32011,"log_path":"/Users/me/.rocket/logs/nuvara/jobs/j3f9a0c12be.log","error":"step 2 (run go test ./...) exited with code 1"}"#;
    let job: Job = roundtrip(json);
    assert_eq!(job.kind, JobKind::Pipeline);
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(
        job.steps,
        vec![
            Step {
                task: "lint".into(),
                run: String::new()
            },
            Step {
                task: String::new(),
                run: "go test ./...".into()
            }
        ]
    );
    assert_eq!(job.exit_code, Some(1));
}

#[test]
fn job_minimal_keeps_always_present_fields() {
    let json = r#"{"id":"j1","project":"p","name":"n","kind":"setup","owner":"user","steps":[],"status":"running","started_at":"2026-10-05T00:00:00Z","duration_ms":0}"#;
    roundtrip::<Job>(json);
    // `"steps": null` (Go nil slice) decodes to empty.
    let nil_steps = json.replace(r#""steps":[]"#, r#""steps":null"#);
    let job: Job = serde_json::from_str(&nil_steps).unwrap();
    assert!(job.steps.is_empty());
}

#[test]
fn zero_time_matches_go() {
    let json = r#"{"type":"log.line","time":"0001-01-01T00:00:00Z"}"#;
    let ev: Event = roundtrip(json);
    assert_eq!(ev.time.year(), 1);
    // A missing non-pointer time decodes to Go's zero time.
    let ev: Event = serde_json::from_str(r#"{"type":"log.line"}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"type":"log.line","time":"0001-01-01T00:00:00Z"}"#
    );
}

#[test]
fn service_state_event_with_embedded_run() {
    let json = r#"{"type":"service.state","time":"2026-10-05T00:14:10.5Z","project":"nuvara","service":"api","state":"running","run":{"project":"nuvara","service":"api","env":"dev","kind":"run","state":"running","health":"unknown"}}"#;
    let ev: Event = roundtrip(json);
    assert_eq!(ev.r#type, event_type::SERVICE_STATE);
    assert_eq!(ev.state, Some(RunState::Running));
    assert_eq!(ev.run.as_ref().unwrap().service, "api");
}

#[test]
fn job_state_event_and_lease_events() {
    let job_json = r#"{"id":"j1","project":"p","name":"n","kind":"deploy","owner":"user","steps":[],"status":"succeeded","started_at":"2026-10-05T00:00:00Z","duration_ms":5}"#;
    let json = format!(
        r#"{{"type":"job.state","time":"2026-10-05T00:00:05Z","project":"p","job_id":"j1","status":"succeeded","job":{job_json}}}"#
    );
    let ev: Event = roundtrip(&json);
    assert_eq!(ev.status, Some(JobStatus::Succeeded));
    assert_eq!(ev.job.as_ref().unwrap().id, "j1");

    let json = r#"{"type":"port.leased","time":"2026-10-05T00:00:00Z","project":"p","service":"s","lease":{"port":18431,"project":"p","service":"s","port_name":"http","created_at":"2026-10-05T00:14:10Z"}}"#;
    let ev: Event = roundtrip(json);
    assert_eq!(ev.lease.unwrap().port, 18431);

    let json = r#"{"type":"log.line","time":"2026-10-05T00:00:00Z","project":"p","service":"s","line":"hi"}"#;
    roundtrip::<Event>(json);
}

#[test]
fn lease_holder_remap_and_project_ref() {
    roundtrip::<Lease>(
        r#"{"port":3100,"project":"p","service":"s","port_name":"http","created_at":"2026-10-05T00:14:10Z"}"#,
    );
    roundtrip::<PortHolder>(r#"{"pid":42,"command":"node"}"#);
    roundtrip::<PortHolder>(r#"{"pid":42,"command":"node","cwd":"/srv"}"#);
    let remap: PortRemap = roundtrip(
        r#"{"name":"http","from":3000,"to":3100,"env":"PORT","holder":{"pid":42,"command":"node"},"reason":"busy"}"#,
    );
    assert_eq!(remap.to, 3100);
    roundtrip::<ProjectRef>(
        r#"{"name":"nuvara","path":"/code/nuvara","added_at":"2026-10-05T00:14:10.25Z"}"#,
    );
}

#[test]
fn project_matches_go_encoding() {
    let json = r#"{"name":"nuvara","root":"/code/nuvara","dotenv":[".env"],"default_env":"dev","setup":{"deps":{"run":"bun install"}},"envs":{"prod":{"name":"prod","deploy":{"task":"ship","confirm":true}}},"services":{"api":{"name":"api","kind":"run","run":"bun run dev","cwd":"api","env":{"A":"1"},"depends_on":["db"],"ports":[{"name":"http","default":3000,"env":"PORT","probe":false}],"health":{"http":"/ready","timeout":5000000000}},"db":{"name":"db","kind":"compose","compose":"postgres"}},"groups":{"all":["*"]},"pipelines":{"ci":[{"task":"lint"}]}}"#;
    let p: Project = roundtrip(json);
    assert_eq!(p.services["api"].ports[0].default, 3000);
    assert!(!p.services["api"].ports[0].probe_enabled());
    assert_eq!(
        p.services["api"].health.as_ref().unwrap().timeout,
        std::time::Duration::from_secs(5)
    );
    assert_eq!(p.envs["prod"].deploy.as_ref().unwrap().step().task, "ship");
}

#[test]
fn omitempty_and_json_dash_semantics() {
    let svc = Service {
        name: "x".into(),
        kind: ServiceKind::Task,
        ports: vec![PortSpec {
            name: "p".into(),
            default: 0,
            env: String::new(),
            env_bindings: [("A".to_string(), Default::default())].into(),
            probe: None,
        }],
        health: Some(HealthSpec::default()),
        ..Service::default()
    };
    // env_bindings is `json:"-"`; empty HealthSpec encodes as {}; default
    // (no omitempty) always appears.
    assert_eq!(
        serde_json::to_string(&svc).unwrap(),
        r#"{"name":"x","kind":"task","ports":[{"name":"p","default":0}],"health":{}}"#
    );
    let deploy = Deploy::default();
    assert_eq!(
        serde_json::to_string(&deploy).unwrap(),
        r#"{"confirm":false}"#
    );
    let env = Environment {
        name: "dev".into(),
        ..Environment::default()
    };
    assert_eq!(serde_json::to_string(&env).unwrap(), r#"{"name":"dev"}"#);
    let p = Project::default();
    assert_eq!(
        serde_json::to_string(&p).unwrap(),
        r#"{"name":"","root":"","default_env":"","services":{}}"#
    );
}

#[test]
fn unknown_fields_are_ignored_like_go() {
    let run: Run = serde_json::from_str(
        r#"{"project":"p","service":"s","env":"dev","kind":"run","state":"dead","health":"unhealthy","future":1}"#,
    )
    .unwrap();
    assert_eq!(run.state, RunState::Dead);
}
