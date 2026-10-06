//! Wire DTOs of the daemon HTTP API (`rocket_domain::api`): byte-compatible
//! with Go's encoding/json, using literals from crates/rocket-api/README.md.

use rocket_domain::api::{
    AddProjectRequest, DownRequest, DownResult, ErrorBody, GcResult, HealthInfo, JobLogsResult,
    JobOutcome, JobRequest, JobsResult, LogsResult, PortsResult, ProjectsResult, RemovedProject,
    ShutdownResult, StatusResult, Summary, UpRequest, UpResult,
};
use rocket_domain::{Health, JobKind, JobStatus, RunState};
use serde::{Serialize, de::DeserializeOwned};

/// Decode, re-encode and require byte equality with the Go-formatted input.
fn roundtrip<T: Serialize + DeserializeOwned>(go_json: &str) -> T {
    let v: T = serde_json::from_str(go_json).expect("decode");
    assert_eq!(serde_json::to_string(&v).expect("encode"), go_json);
    v
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture")
}

#[test]
fn health_info_roundtrip_and_http_omitted() {
    let json = r#"{"ok":true,"api":"v1","version":"0.1.0-dev","pid":4242,"started_at":"2026-10-05T00:14:10Z","home":"/Users/me/.rocket","socket":"/Users/me/.rocket/rocketd.sock"}"#;
    let h: HealthInfo = roundtrip(json);
    assert!(h.ok);
    assert_eq!(h.http, "");
    let with_http = r#"{"ok":true,"api":"v1","version":"0.1.0-dev","pid":4242,"started_at":"2026-10-05T00:14:10Z","home":"/h","socket":"/h/s","http":"http://127.0.0.1:53124"}"#;
    let h: HealthInfo = roundtrip(with_http);
    assert_eq!(h.http, "http://127.0.0.1:53124");
}

#[test]
fn health_info_tolerates_unknown_response_fields() {
    let h: HealthInfo = serde_json::from_str(
        r#"{"ok":true,"api":"v1","version":"x","pid":1,"started_at":"2026-10-05T00:14:10Z","home":"h","socket":"s","future":[1,2]}"#,
    )
    .unwrap();
    assert_eq!(h.api, "v1");
}

#[test]
fn error_body() {
    let b: ErrorBody = roundtrip(
        r#"{"error":"invalid request: unknown service or group \"nope\" in project shop","code":"invalid"}"#,
    );
    assert_eq!(b.code, "invalid");
}

#[test]
fn up_request_matches_go_and_rejects_unknown_fields() {
    let json = r#"{"project":"/Users/me/code/nuvara","services":["core"],"env":"dev","profiles":["trends"],"owner":"agent:claude-123","ttl":"30m"}"#;
    let r: UpRequest = roundtrip(json);
    assert_eq!(r.services, ["core"]);
    // omitempty: only `project` is always present.
    let min = UpRequest {
        project: "shop".into(),
        ..UpRequest::default()
    };
    assert_eq!(
        serde_json::to_string(&min).unwrap(),
        r#"{"project":"shop"}"#
    );
    // Go decodes request bodies with DisallowUnknownFields.
    assert!(serde_json::from_str::<UpRequest>(r#"{"project":"x","bogus":1}"#).is_err());
}

#[test]
fn down_request_omits_zero_values() {
    let json = r#"{"project":"/Users/me/code/nuvara","services":["web"],"owner":"agent:x","everywhere":true}"#;
    roundtrip::<DownRequest>(json);
    assert_eq!(
        serde_json::to_string(&DownRequest::default()).unwrap(),
        "{}"
    );
    assert!(serde_json::from_str::<DownRequest>(r#"{"nope":true}"#).is_err());
}

#[test]
fn add_project_request_and_projects_result() {
    roundtrip::<AddProjectRequest>(r#"{"path":"/abs/dir"}"#);
    assert!(serde_json::from_str::<AddProjectRequest>(r#"{"path":"/a","x":1}"#).is_err());
    let p: ProjectsResult = serde_json::from_str(&fixture("projects.json")).unwrap();
    assert_eq!(p.projects[0].name, "nuvara");
    assert_eq!(
        serde_json::to_string(&ProjectsResult::default()).unwrap(),
        r#"{"projects":[]}"#
    );
    let null: ProjectsResult = serde_json::from_str(r#"{"projects":null}"#).unwrap();
    assert!(null.projects.is_empty());
}

#[test]
fn removed_and_shutdown() {
    let r: RemovedProject = roundtrip(r#"{"removed":"nuvara"}"#);
    assert_eq!(r.removed, "nuvara");
    let s: ShutdownResult = roundtrip(r#"{"ok":true,"pid":123}"#);
    assert_eq!(s.pid, 123);
}

#[test]
fn up_result_from_readme() {
    let r: UpResult = serde_json::from_str(&fixture("up_result.json")).unwrap();
    assert_eq!(r.services.len(), 3);
    let web = &r.services[1];
    assert_eq!(web.action, "started");
    assert_eq!(web.state, RunState::Running);
    assert_eq!(web.health, Some(Health::Healthy));
    assert_eq!(web.ports["http"], 3100);
    assert_eq!(web.remaps[0].holder.as_ref().unwrap().pid, 77);
    assert!(web.expires_at.is_some());
    assert_eq!(
        r.services[2].error,
        "port 3003 (http) for nuvara/causation is busy"
    );
    // Re-encode and decode again: stable.
    let again: UpResult = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(again, r);
}

#[test]
fn up_result_omits_empty_fields() {
    let json = r#"{"project":"p","env":"dev","services":[{"service":"api","action":"started","state":"running","health":"healthy","pid":1,"ports":{"http":3000},"owner":"user"}],"hints":["rocket migrate -p p"]}"#;
    roundtrip::<UpResult>(json);
    let json = r#"{"project":"p","env":"dev","services":[{"service":"api","action":"failed","state":"failed","error":"boom"}]}"#;
    roundtrip::<UpResult>(json);
}

#[test]
fn down_result_matches_go() {
    let json = r#"{"stopped":[{"project":"p","service":"s","env":"dev","kind":"run","state":"stopped","health":"unknown"}],"compose_down":["rocket-nuvara-dev"],"canceled_jobs":["j3f9a0c12be"],"errors":["x"]}"#;
    let r: DownResult = roundtrip(json);
    assert_eq!(r.canceled_jobs, ["j3f9a0c12be"]);
    // `stopped` is never omitted, the rest are.
    assert_eq!(
        serde_json::to_string(&DownResult::default()).unwrap(),
        r#"{"stopped":[]}"#
    );
}

#[test]
fn status_logs_ports_gc() {
    let s: StatusResult = serde_json::from_str(&fixture("ps.json")).unwrap();
    assert!(!s.services.is_empty());
    let l: LogsResult = serde_json::from_str(&fixture("logs.json")).unwrap();
    assert!(!l.lines.is_empty());
    let p: PortsResult = serde_json::from_str(
        r#"{"ports":[{"port":18431,"project":"rocket-fixture","service":"static","port_name":"http","created_at":"2026-10-05T00:14:10Z","owner":"user","state":"running","pid":8635,"env":"dev"},{"port":3100,"project":"nuvara","service":"web","port_name":"http","created_at":"2026-10-05T00:14:10Z"}]}"#,
    )
    .unwrap();
    assert_eq!(p.ports[0].lease.port, 18431);
    assert_eq!(p.ports[0].state, Some(RunState::Running));
    assert_eq!(p.ports[1].owner, "");
    // Lease fields come first (embedded struct), then the row extras.
    assert_eq!(
        serde_json::to_string(&p).unwrap(),
        r#"{"ports":[{"port":18431,"project":"rocket-fixture","service":"static","port_name":"http","created_at":"2026-10-05T00:14:10Z","owner":"user","state":"running","pid":8635,"env":"dev"},{"port":3100,"project":"nuvara","service":"web","port_name":"http","created_at":"2026-10-05T00:14:10Z"}]}"#
    );
    let gc = r#"{"actions":[{"action":"released_lease","project":"ghost","service":"x","port":9999},{"action":"pruned","project":"nuvara","service":"web","detail":"stopped"}]}"#;
    let g: GcResult = roundtrip(gc);
    assert_eq!(g.actions[0].port, 9999);
}

#[test]
fn summary_distinguishes_absent_metadata_from_empty() {
    let s: Summary = serde_json::from_str(&fixture("summary.json")).unwrap();
    let p = s.project.as_ref().unwrap();
    assert_eq!(p.default_env, "dev");
    assert_eq!(p.envs, None, "older daemons omit envs");
    assert_eq!(s.conflicts.len(), 2);
    assert_eq!(s.conflicts[1].holder.as_ref().unwrap().command, "node");
    assert!(!s.conflicts[1].remappable);

    let full = r#"{"project":{"name":"n","root":"/r","default_env":"dev","envs":["dev"],"pipelines":[],"deploy_envs":["prod"]},"services":[],"jobs":[],"conflicts":[]}"#;
    let s: Summary = roundtrip(full);
    assert_eq!(s.project.unwrap().pipelines, Some(vec![]));
    // No project: omitted.
    roundtrip::<Summary>(r#"{"services":[],"jobs":[],"conflicts":[]}"#);
}

#[test]
fn job_request_matches_go() {
    let json = r#"{"project":"/Users/me/code/nuvara","kind":"pipeline","name":"test:api","env":"dev","profiles":["trends"],"ttl":"30m","args":["-run","TestLogin"],"owner":"agent:claude-123","yes":true,"allow_agent_deploy":true}"#;
    let r: JobRequest = roundtrip(json);
    assert_eq!(r.kind, JobKind::Pipeline);
    let min = JobRequest {
        project: "p".into(),
        kind: JobKind::Setup,
        ..JobRequest::default()
    };
    assert_eq!(
        serde_json::to_string(&min).unwrap(),
        r#"{"project":"p","kind":"setup"}"#
    );
    assert!(serde_json::from_str::<JobRequest>(r#"{"project":"p","kind":"setup","x":1}"#).is_err());
}

#[test]
fn jobs_and_job_logs() {
    let j: JobsResult =
        serde_json::from_str(&format!(r#"{{"jobs":[{}]}}"#, fixture("job.json"))).unwrap();
    assert_eq!(j.jobs[0].status, JobStatus::Failed);
    assert_eq!(
        serde_json::to_string(&JobsResult::default()).unwrap(),
        r#"{"jobs":[]}"#
    );
    let l: JobLogsResult = serde_json::from_str(&fixture("job_logs.json")).unwrap();
    assert!(!l.lines.is_empty());
    roundtrip::<JobLogsResult>(r#"{"job":"j1","project":"nuvara","lines":["a","b"]}"#);
}

#[test]
fn job_outcome_always_has_exit_code() {
    let json = r#"{"job":"j3f9a0c12be","project":"nuvara","kind":"pipeline","name":"ci","status":"failed","exit_code":1,"duration_ms":32011,"log_path":"/l","tail":["x"],"error":"boom"}"#;
    roundtrip::<JobOutcome>(json);
    let json = r#"{"job":"j","project":"p","kind":"setup","name":"","status":"canceled","exit_code":null,"duration_ms":0,"log_path":"","tail":[]}"#;
    roundtrip::<JobOutcome>(json);
}
