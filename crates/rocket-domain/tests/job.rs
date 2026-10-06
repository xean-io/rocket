//! Port of domain/job_test.go plus small helpers.

use rocket_domain::{Job, JobKind, JobStatus, Run, RunState, Step, is_agent_owner};
use time::macros::datetime;

fn base_job() -> Job {
    serde_json::from_str(
        r#"{"id":"j1","project":"p","name":"n","kind":"setup","owner":"user","steps":[],
            "status":"running","started_at":"2026-10-05T00:00:00Z","duration_ms":0}"#,
    )
    .unwrap()
}

fn job_with(expires: Option<time::OffsetDateTime>) -> Job {
    Job {
        expires_at: expires,
        ..base_job()
    }
}

#[test]
fn job_expired() {
    let deadline = datetime!(2026-10-05 00:00:00 UTC);
    let ns = time::Duration::nanoseconds(1);
    let hour = time::Duration::hours(1);
    let cases = [
        ("no deadline", None, deadline + hour, false),
        ("before deadline", Some(deadline), deadline - ns, false),
        ("at deadline", Some(deadline), deadline, true),
        ("after deadline", Some(deadline), deadline + ns, true),
    ];
    for (name, expires, now, want) in cases {
        assert_eq!(job_with(expires).expired(now), want, "{name}");
    }
}

#[test]
fn run_expired_mirrors_job() {
    let deadline = datetime!(2026-10-05 00:00:00 UTC);
    let mut run: Run = serde_json::from_str(
        r#"{"project":"p","service":"s","env":"dev","kind":"run","state":"running","health":"unknown"}"#,
    )
    .unwrap();
    assert!(!run.expired(deadline));
    run.expires_at = Some(deadline);
    assert!(run.expired(deadline));
    assert!(!run.expired(deadline - time::Duration::nanoseconds(1)));
}

#[test]
fn job_status_terminal_and_run_state_active() {
    assert!(!JobStatus::Running.terminal());
    for s in [
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Canceled,
        JobStatus::Lost,
    ] {
        assert!(s.terminal());
    }
    for s in [RunState::Starting, RunState::Running, RunState::Stopping] {
        assert!(s.active(), "{s} should be active");
    }
    for s in [
        RunState::Stopped,
        RunState::Exited,
        RunState::Failed,
        RunState::Dead,
    ] {
        assert!(!s.active(), "{s} should not be active");
    }
}

#[test]
fn step_describe_and_agent_owner() {
    assert_eq!(
        Step {
            task: "lint".into(),
            run: String::new()
        }
        .describe(),
        "task lint"
    );
    assert_eq!(
        Step {
            task: String::new(),
            run: "pnpm test".into()
        }
        .describe(),
        "run pnpm test"
    );
    assert!(is_agent_owner("agent:claude-1"));
    assert!(!is_agent_owner("user"));
    assert_eq!(JobKind::Pipeline.to_string(), "pipeline");
}
