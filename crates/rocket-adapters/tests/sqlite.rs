//! Ports of `internal/adapters/sqlite/store_test.go` plus Go <-> Rust
//! `state.db` compatibility checks.

use rocket_adapters::sqlite::Store;
use rocket_domain::ports::{self, Error, Store as _};
use rocket_domain::{Job, Lease, ProjectRef, Run};
use serde_json::json;
use std::path::Path;
use time::OffsetDateTime;
use time::macros::datetime;

fn open_temp() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    (dir, store)
}

fn run(v: serde_json::Value) -> Run {
    serde_json::from_value(v).unwrap()
}

fn job(v: serde_json::Value) -> Job {
    serde_json::from_value(v).unwrap()
}

fn lease(port: u16, project: &str, service: &str, name: &str, at: OffsetDateTime) -> Lease {
    Lease {
        port,
        project: project.into(),
        service: service.into(),
        port_name: name.into(),
        created_at: at,
    }
}

#[test]
fn projects_round_trip() {
    let (_d, s) = open_temp();
    let now = datetime!(2026-01-01 10:00:00 UTC);
    let mut p = ProjectRef {
        name: "nuvara".into(),
        path: "/a".into(),
        added_at: now,
    };
    s.upsert_project(&p).unwrap();
    p.path = "/b".into();
    s.upsert_project(&p).unwrap();
    let got = s.get_project("nuvara").unwrap().unwrap();
    assert_eq!(got.path, "/b");
    assert_eq!(got.added_at, now);
    assert_eq!(s.list_projects().unwrap().len(), 1);
    s.delete_project("nuvara").unwrap();
    assert!(s.get_project("nuvara").unwrap().is_none());
}

#[test]
fn upsert_project_keeps_the_original_added_at() {
    let (_d, s) = open_temp();
    let first = datetime!(2026-01-01 10:00:00 UTC);
    let mut p = ProjectRef {
        name: "n".into(),
        path: "/a".into(),
        added_at: first,
    };
    s.upsert_project(&p).unwrap();
    p.added_at = datetime!(2027-02-02 00:00:00 UTC);
    s.upsert_project(&p).unwrap();
    assert_eq!(s.get_project("n").unwrap().unwrap().added_at, first);
}

#[test]
fn runs_round_trip() {
    let (_d, s) = open_temp();
    let mut r = run(json!({
        "project": "p", "service": "api", "env": "", "kind": "run", "state": "running",
        "health": "unknown", "pid": 42, "pgid": 42, "owner": "agent:x",
        "expires_at": "2026-01-01T10:01:00.123Z", "ports": {"http": 3100}
    }));
    s.save_run(&r).unwrap();
    r.state = rocket_domain::RunState::Stopped;
    s.save_run(&r).unwrap();
    let got = s.get_run("p", "api").unwrap().unwrap();
    assert_eq!(got.state, rocket_domain::RunState::Stopped);
    assert_eq!(got.ports["http"], 3100);
    assert_eq!(got.expires_at, r.expires_at);
    assert_eq!(s.list_runs().unwrap().len(), 1);
    s.delete_run("p", "api").unwrap();
    assert!(s.get_run("p", "api").unwrap().is_none());
}

#[test]
fn compose_run_profiles_round_trip() {
    let (_d, s) = open_temp();
    let r = run(json!({
        "project": "profiles", "service": "reporting", "env": "", "kind": "compose",
        "state": "running", "health": "unknown", "compose_project": "rocket-profiles-dev",
        "profiles": ["base", "extra", "reports"]
    }));
    s.save_run(&r).unwrap();
    let got = s.get_run("profiles", "reporting").unwrap().unwrap();
    assert_eq!(got.profiles, ["base", "extra", "reports"]);
}

#[test]
fn lease_conflicts() {
    let (_d, s) = open_temp();
    let now = OffsetDateTime::now_utc();
    let l = lease(3000, "a", "web", "http", now);
    s.acquire_lease(&l).unwrap();
    s.acquire_lease(&l).unwrap(); // re-acquire by the same service
    let err = s
        .acquire_lease(&lease(3000, "b", "web", "http", now))
        .unwrap_err();
    assert!(matches!(err, Error::LeaseTaken), "err {err}");
    let released = s.release_leases("a", "web").unwrap();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].port, 3000);
    assert!(s.list_leases().unwrap().is_empty());
}

#[test]
fn jobs_round_trip_and_ordering() {
    let (_d, s) = open_temp();
    let base = datetime!(2026-01-01 10:00:00 UTC);
    let deadline = base + time::Duration::minutes(30);
    let mut jobs = vec![
        job(json!({
            "id": "j1", "project": "a", "name": "ci", "kind": "pipeline", "env": "dev",
            "owner": "user", "status": "failed",
            "steps": [{"task": "lint"}, {"run": "go test ./..."}], "args": ["-v"],
            "exit_code": 3, "started_at": "2026-01-01T10:00:00Z",
            "expires_at": "2026-01-01T10:30:00Z", "duration_ms": 0, "log_path": "/l/j1.log"
        })),
        // 500ms later: RFC3339Nano would sort "10:00:00.5Z" before "10:00:00Z"
        job(json!({
            "id": "j2", "project": "a", "name": "setup", "kind": "setup", "owner": "agent:x",
            "status": "running", "started_at": "2026-01-01T10:00:00.5Z"
        })),
        job(json!({
            "id": "j3", "project": "b", "name": "prod", "kind": "deploy", "env": "prod",
            "owner": "user", "status": "succeeded", "started_at": "2026-01-01T10:00:01Z"
        })),
    ];
    for j in &jobs {
        s.save_job(j).unwrap();
    }
    let got = s.get_job("j1").unwrap().unwrap();
    assert_eq!(got.env, "dev");
    assert_eq!(got.expires_at, Some(deadline));
    assert_eq!(got.exit_code, Some(3));
    assert_eq!(got.steps.len(), 2);
    assert_eq!(got.args[0], "-v");
    assert_eq!(got.started_at, base);

    let ids = |v: Vec<Job>| v.into_iter().map(|j| j.id).collect::<Vec<_>>();
    assert_eq!(ids(s.list_jobs("", 0).unwrap()), ["j3", "j2", "j1"]);
    assert_eq!(ids(s.list_jobs("a", 1).unwrap()), ["j2"]);

    jobs[1].status = rocket_domain::JobStatus::Succeeded;
    s.save_job(&jobs[1]).unwrap();
    assert_eq!(
        s.get_job("j2").unwrap().unwrap().status,
        rocket_domain::JobStatus::Succeeded
    );
    s.delete_job("j2").unwrap();
    assert!(s.get_job("j2").unwrap().is_none());
}

#[test]
fn job_profiles_persist_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profiles.db");
    let store = Store::open(&path).unwrap();
    let j = job(json!({
        "id": "jprofile", "project": "fixture", "env": "smoke",
        "profiles": ["base", "extra"], "status": "succeeded", "kind": "setup", "name": ""
    }));
    store.save_job(&j).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let got = store.get_job("jprofile").unwrap().unwrap();
    let wire = serde_json::to_value(&got).unwrap();
    assert_eq!(wire["profiles"], json!(["base", "extra"]));
}

#[test]
fn open_applies_the_go_pragmas_and_schema() {
    let (_d, s) = open_temp();
    // Unknown names must read back as absent rather than error: the schema exists.
    assert!(s.get_job("nope").unwrap().is_none());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.db");
    drop(Store::open(&path).unwrap());
    let c = rusqlite::Connection::open(&path).unwrap();
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let mut stmt = c
        .prepare("SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        names,
        ["jobs", "jobs_project_started", "leases", "projects", "runs"]
    );
}

// ---- Go <-> Rust compatibility -------------------------------------------------

const GO_DB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/go_state.db");
const GO_EXPECTED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/go_state.expected.json"
);

#[derive(serde::Deserialize)]
struct Expected {
    projects: Vec<ProjectRef>,
    runs: Vec<Run>,
    leases: Vec<Lease>,
    jobs: Vec<Job>,
    jobs_nuvara_limit1: Vec<Job>,
}

/// A copy of the Go-written database (opening it creates WAL side files).
fn go_db_copy() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    std::fs::copy(GO_DB, &path).unwrap();
    let store = Store::open(&path).unwrap();
    (dir, store)
}

fn expected() -> Expected {
    serde_json::from_slice(&std::fs::read(GO_EXPECTED).unwrap()).unwrap()
}

#[test]
fn reads_a_state_db_written_by_the_go_adapter() {
    let (_d, s) = go_db_copy();
    let want = expected();
    assert_eq!(s.list_projects().unwrap(), want.projects);
    assert_eq!(s.list_runs().unwrap(), want.runs);
    assert_eq!(s.list_leases().unwrap(), want.leases);
    assert_eq!(s.list_jobs("", 0).unwrap(), want.jobs);
    assert_eq!(s.list_jobs("nuvara", 1).unwrap(), want.jobs_nuvara_limit1);

    // Spot checks on values that exercise formatting edge cases.
    let nuvara = s.get_project("nuvara").unwrap().unwrap();
    assert_eq!(
        nuvara.added_at,
        datetime!(2026-03-14 09:26:53.589793238 UTC)
    );
    let half = s.get_project("half").unwrap().unwrap();
    assert_eq!(half.added_at, datetime!(2026-03-14 09:26:53.5 UTC));
    let db = s.get_run("nuvara", "db").unwrap().unwrap();
    assert_eq!(db.exit_code, Some(137));
    assert_eq!(db.error, "boom \"quoted\" <tag> & ünï");
    assert_eq!(db.profiles, ["base", "extra"]);
    let j1 = s.get_job("j1").unwrap().unwrap();
    assert_eq!(
        j1.finished_at,
        Some(datetime!(2026-03-14 09:26:58.589793238 UTC))
    );
    assert_eq!(s.list_leases().unwrap().len(), 3);
}

#[test]
fn rust_writes_are_byte_compatible_with_go() {
    // Re-saving every Go-written record must reproduce Go's exact JSON `data`
    // blobs and column formats, so a Go daemon reads Rust writes unchanged.
    let (dir, s) = go_db_copy();
    let path = dir.path().join("state.db");
    let before = raw_rows(&path);
    for r in s.list_runs().unwrap() {
        s.save_run(&r).unwrap();
    }
    for j in s.list_jobs("", 0).unwrap() {
        s.save_job(&j).unwrap();
    }
    for p in s.list_projects().unwrap() {
        s.upsert_project(&p).unwrap();
    }
    for l in s.list_leases().unwrap() {
        s.acquire_lease(&l).unwrap();
    }
    let after = raw_rows(&path);
    // Two encoder differences that Go decodes identically: a nil slice is
    // `null` (Rust writes `[]`), and Go escapes `<`, `>` and `&` as \u003c,
    // \u003e and \u0026 (Rust writes them raw).
    let norm = |rows: Vec<String>| {
        rows.into_iter()
            .map(|r| {
                r.replace("\"steps\":null", "\"steps\":[]")
                    .replace("'null'", "'[]'")
                    .replace("\\u003c", "<")
                    .replace("\\u003e", ">")
                    .replace("\\u0026", "&")
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(norm(before), norm(after));
}

/// Every column of every row, quoted by SQLite, in primary-key order.
fn raw_rows(path: &Path) -> Vec<String> {
    let c = rusqlite::Connection::open(path).unwrap();
    let queries = [
        "SELECT quote(name)||'|'||quote(path)||'|'||quote(added_at) FROM projects ORDER BY name",
        "SELECT quote(project)||'|'||quote(service)||'|'||quote(data) FROM runs ORDER BY project, service",
        "SELECT quote(port)||'|'||quote(project)||'|'||quote(service)||'|'||quote(port_name)||'|'||quote(created_at) FROM leases ORDER BY port",
        "SELECT quote(id)||'|'||quote(project)||'|'||quote(name)||'|'||quote(kind)||'|'||quote(owner)||'|'||quote(steps)||'|'||quote(status)||'|'||quote(exit_code)||'|'||quote(started_at)||'|'||quote(finished_at)||'|'||quote(log_path)||'|'||quote(data) FROM jobs ORDER BY id",
    ];
    let mut rows = Vec::new();
    for q in queries {
        let mut stmt = c.prepare(q).unwrap();
        rows.extend(
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    rows
}

#[test]
fn rust_write_then_reread_round_trip() {
    let (_d, s) = open_temp();
    let want = expected();
    for p in &want.projects {
        s.upsert_project(p).unwrap();
    }
    for r in &want.runs {
        s.save_run(r).unwrap();
    }
    for l in &want.leases {
        s.acquire_lease(l).unwrap();
    }
    for j in &want.jobs {
        s.save_job(j).unwrap();
    }
    assert_eq!(s.list_projects().unwrap(), want.projects);
    assert_eq!(s.list_runs().unwrap(), want.runs);
    assert_eq!(s.list_leases().unwrap(), want.leases);
    assert_eq!(s.list_jobs("", 0).unwrap(), want.jobs);
    assert_eq!(s.list_jobs("nuvara", 1).unwrap(), want.jobs_nuvara_limit1);
}

#[test]
fn store_is_usable_as_a_port_object_across_threads() {
    let (_d, s) = open_temp();
    let s: std::sync::Arc<dyn ports::Store> = std::sync::Arc::new(s);
    let handles: Vec<_> = (0..4u16)
        .map(|i| {
            let s = s.clone();
            std::thread::spawn(move || {
                s.acquire_lease(&lease(
                    3000 + i,
                    "p",
                    &format!("s{i}"),
                    "http",
                    datetime!(2026-01-01 00:00:00 UTC),
                ))
                .unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(s.list_leases().unwrap().len(), 4);
}
