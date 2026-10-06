//! Ports of `jobs_test.go`, `job_ttl_test.go`, `job_profiles_test.go` and
//! `pipeline_needs_test.go`, plus Rust-only job tests at the end.

use super::profiles::{ROOT as PROFILES_ROOT, profile_harness};
use crate::jobs::{JobsRequest, KEEP_JOBS, outcome};
use crate::testing::*;
use crate::{AppError, CancellationToken};
use rocket_domain::api::{DownRequest, JobRequest, UpRequest, gc_action};
use rocket_domain::ports::{
    self, ComposeDriver, ComposeTarget, HealthProbe, ProcessHandle, ProcessRunner, ProcessSpec,
    Store, async_trait,
};
use rocket_domain::{
    HealthCheck, Job, JobKind, JobStatus, Project, RunState, compose_project_name, event_type,
    merge_profiles,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::{Notify, watch};

fn job_req(kind: JobKind, name: &str) -> JobRequest {
    JobRequest {
        kind,
        name: name.into(),
        ..JobRequest::default()
    }
}

fn running_job(id: &str, pid: i32, started_at: OffsetDateTime) -> Job {
    Job {
        id: id.into(),
        project: "jobs".into(),
        name: "slow".into(),
        kind: JobKind::Pipeline,
        env: String::new(),
        profiles: Vec::new(),
        owner: String::new(),
        steps: Vec::new(),
        args: Vec::new(),
        status: JobStatus::Running,
        step: 0,
        pid,
        pgid: pid,
        exit_code: None,
        started_at,
        expires_at: None,
        finished_at: None,
        duration_ms: 0,
        log_path: String::new(),
        error: String::new(),
    }
}

fn is_invalid<T>(r: &Result<T, AppError>) -> bool {
    matches!(r, Err(AppError::Invalid(_)))
}

// --- jobs_test.go ---------------------------------------------------------------

#[tokio::test]
async fn run_pipeline_runs_steps_in_order_with_project_env() {
    let h = Harness::new();
    for cmd in ["step-lint", "step-unit", "task e2e"] {
        h.runner.exit_now(cmd, 0);
    }
    let mut req = job_req(JobKind::Pipeline, "ci");
    req.args = vec!["-run".into(), "X".into()];
    req.owner = "agent:a".into();
    let job = h.start_job(req);
    let got = h.wait_job(&job.id).await;

    assert_eq!(got.status, JobStatus::Succeeded, "{got:?}");
    assert_eq!((got.exit_code, got.step), (Some(0), 3));
    assert_eq!(got.owner, "agent:a");
    assert_eq!((got.kind, got.name.as_str()), (JobKind::Pipeline, "ci"));
    assert!(
        !got.log_path.is_empty() && got.finished_at.is_some(),
        "{got:?}"
    );
    let want: Vec<Vec<String>> = vec![
        vec!["/bin/sh".into(), "-c".into(), "step-lint".into()],
        vec!["/bin/sh".into(), "-c".into(), "step-unit".into()],
        // args go to the last step only
        vec!["task".into(), "e2e".into(), "-run".into(), "X".into()],
    ];
    assert_eq!(h.argvs(), want);
    let spec = h.runner.started().remove(0);
    let env = env_map(&spec.env);
    assert_eq!(spec.dir.to_string_lossy(), "/code/jobs");
    assert_eq!(env["FROM_DOTENV"], "1");
    assert!(!env.contains_key("ROCKET_OWNER"));
    let states: Vec<JobStatus> = h
        .bus
        .events()
        .iter()
        .filter(|e| e.r#type == event_type::JOB_STATE && e.job_id == job.id)
        .filter_map(|e| e.status)
        .collect();
    assert!(
        states.len() >= 2
            && states[0] == JobStatus::Running
            && states[states.len() - 1] == JobStatus::Succeeded,
        "job.state events {states:?}"
    );
}

#[tokio::test]
async fn run_pipeline_stops_on_first_failure() {
    let h = Harness::new();
    h.runner.exit_now("step-ok", 0);
    h.runner.exit_now("step-boom", 3);
    let job = h.start_job(job_req(JobKind::Pipeline, "broken"));
    let got = h.wait_job(&job.id).await;
    assert_eq!(got.status, JobStatus::Failed, "{got:?}");
    assert_eq!((got.exit_code, got.step), (Some(3), 2));
    assert_eq!(h.argvs().len(), 2, "stop on first failure");
}

#[tokio::test]
async fn run_args_append_to_shell_step() {
    let h = Harness::new();
    h.runner.exit_now("step-slow", 0);
    let mut req = job_req(JobKind::Pipeline, "slow");
    req.args = vec!["a".into(), "b c".into()];
    let job = h.start_job(req);
    h.wait_job(&job.id).await;
    let want: Vec<String> = ["/bin/sh", "-c", "step-slow \"$@\"", "rocket", "a", "b c"]
        .map(String::from)
        .to_vec();
    assert_eq!(h.argvs()[0], want);
}

#[tokio::test]
async fn run_falls_back_to_taskfile_task() {
    let h = Harness::new();
    h.runner.exit_now("task build", 0);
    let job = h.start_job(job_req(JobKind::Pipeline, "build"));
    let got = h.wait_job(&job.id).await;
    assert_eq!(got.status, JobStatus::Succeeded, "{got:?}");
    assert_eq!(got.steps.len(), 1);
    assert_eq!(got.steps[0].task, "build");

    let mut req = job_req(JobKind::Pipeline, "build");
    req.project = "/code/nuvara".into();
    let res = h.app.start_job(req);
    assert!(is_invalid(&res), "no Taskfile: want invalid, got {res:?}");
}

#[tokio::test]
async fn setup_runs_doctor_install_migrate_skipping_undefined() {
    let h = Harness::new();
    for cmd in ["doctor-ok", "task db:migrate", "seed-it"] {
        h.runner.exit_now(cmd, 0);
    }
    let job = h.start_job(job_req(JobKind::Setup, ""));
    let got = h.wait_job(&job.id).await;
    assert_eq!(got.status, JobStatus::Succeeded, "{got:?}");
    assert_eq!(got.name, "setup");
    assert_eq!(got.steps.len(), 2);
    assert_eq!(got.steps[0].run, "doctor-ok");
    assert_eq!(got.steps[1].task, "db:migrate");

    let job = h.start_job(job_req(JobKind::Setup, "seed"));
    let seed = h.wait_job(&job.id).await;
    assert_eq!((seed.name.as_str(), seed.steps.len()), ("seed", 1));
    assert_eq!(seed.status, JobStatus::Succeeded);

    for (project, name) in [("/code/jobs", "install"), ("/code/nuvara", "")] {
        let mut req = job_req(JobKind::Setup, name);
        req.project = project.into();
        let res = h.app.start_job(req);
        assert!(is_invalid(&res), "{project} setup {name:?}: {res:?}");
    }
}

#[tokio::test]
async fn deploy_gate() {
    struct Case {
        name: &'static str,
        target: &'static str,
        owner: &'static str,
        yes: bool,
        allow: bool,
        want: Option<&'static str>,
    }
    let case = |name, target, owner, yes, allow, want| Case {
        name,
        target,
        owner,
        yes,
        allow,
        want,
    };
    let cases = [
        case(
            "env without deploy",
            "dev",
            "",
            false,
            false,
            Some("invalid"),
        ),
        case("unknown env", "nope", "", false, false, Some("invalid")),
        case(
            "confirm env without --yes",
            "prod",
            "",
            false,
            false,
            Some("confirmation_required"),
        ),
        case("confirm env with --yes", "prod", "", true, false, None),
        case("no-confirm env by user", "stage", "", false, false, None),
        case(
            "agent without flags",
            "stage",
            "agent:x",
            false,
            false,
            Some("confirmation_required"),
        ),
        case(
            "agent with --yes only",
            "stage",
            "agent:x",
            true,
            false,
            Some("confirmation_required"),
        ),
        case(
            "agent with allow only",
            "stage",
            "agent:x",
            false,
            true,
            Some("confirmation_required"),
        ),
        case(
            "agent with --yes and allow",
            "prod",
            "agent:x",
            true,
            true,
            None,
        ),
    ];
    for c in cases {
        let h = Harness::new();
        h.runner.exit_now("deploy-stage", 0);
        h.runner.exit_now("task release:prod", 0);
        let mut req = job_req(JobKind::Deploy, c.target);
        req.project = "/code/jobs".into();
        req.owner = c.owner.into();
        req.yes = c.yes;
        req.allow_agent_deploy = c.allow;
        let res = h.app.start_job(req);
        if let Some(code) = c.want {
            let err = res.expect_err(c.name);
            assert_eq!(err.code(), code, "{}: {err}", c.name);
            assert!(h.argvs().is_empty(), "{}: refused deploy started", c.name);
            continue;
        }
        let job = res.unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let got = h.wait_job(&job.id).await;
        assert_eq!(got.kind, JobKind::Deploy, "{}", c.name);
        assert_eq!((got.name.as_str(), got.env.as_str()), (c.target, c.target));
        assert_eq!(got.status, JobStatus::Succeeded, "{}: {got:?}", c.name);
    }
}

#[tokio::test]
async fn cancel_job_stops_process_group() {
    let h = Harness::new();
    let job = h.start_job(job_req(JobKind::Pipeline, "slow"));
    let running = h.wait_job_pgid(&job.id).await;

    let got = h.app.cancel_job(&job.id).await.unwrap();
    assert_eq!(got.status, JobStatus::Canceled, "{got:?}");
    assert_eq!(got.exit_code, Some(143));
    assert!(
        h.runner.stopped().contains(&running.pgid),
        "pgid {} not stopped ({:?})",
        running.pgid,
        h.runner.stopped()
    );
    let err = h.app.cancel_job("nope").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err}");
}

#[tokio::test]
async fn reconcile_marks_vanished_jobs_lost() {
    let h = Harness::new();
    let start = h.now();
    h.store
        .save_job(&running_job("jdead", 5555, start))
        .unwrap();
    h.store
        .save_job(&running_job("jlive", 6666, start))
        .unwrap();
    h.runner.set_alive(6666, true);

    let rec = h.app.reconcile().await.unwrap();
    assert_eq!(rec.lost_jobs, ["jdead"]);
    let dead = h.app.get_job("jdead").unwrap();
    assert_eq!(dead.status, JobStatus::Lost);
    assert!(dead.finished_at.is_some());
    let live = h.app.get_job("jlive").unwrap();
    assert_eq!(live.status, JobStatus::Running, "adopted: {live:?}");
    h.runner.set_alive(6666, false);
    assert_eq!(h.wait_job("jlive").await.status, JobStatus::Lost);
}

#[tokio::test]
async fn down_by_owner_cancels_that_owners_jobs() {
    let h = Harness::new();
    let mut req = job_req(JobKind::Pipeline, "slow");
    req.owner = "agent:z".into();
    let agent_job = h.start_job(req);
    h.clock.advance(Duration::from_secs(1));
    let user_job = h.start_job(job_req(JobKind::Pipeline, "slow"));
    h.wait_job_pgid(&agent_job.id).await;
    h.wait_job_pgid(&user_job.id).await;

    let res = h
        .app
        .down(DownRequest {
            project: "/code/jobs".into(),
            owner: "agent:z".into(),
            ..DownRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(
        res.canceled_jobs.as_slice(),
        std::slice::from_ref(&agent_job.id)
    );
    let survivor = h.app.get_job(&user_job.id).unwrap();
    assert_eq!(survivor.status, JobStatus::Running, "{survivor:?}");
    let list = h
        .app
        .list_jobs(&JobsRequest {
            project: "/code/jobs".into(),
            ..JobsRequest::default()
        })
        .unwrap();
    assert_eq!(list.jobs.len(), 2);
    assert_eq!(list.jobs[0].id, user_job.id, "newest first");
    h.app.cancel_job(&user_job.id).await.unwrap();
}

#[tokio::test]
async fn get_job_unknown() {
    let h = Harness::new();
    let err = h.app.get_job("missing").unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err}");
}

// --- job_profiles_test.go ----------------------------------------------------------

#[tokio::test]
async fn job_profiles_select_wildcard_prerequisites_and_persist() {
    let cases: [(&str, &str, &[&str], &[&str]); 3] = [
        (
            "default env",
            "{}",
            &["base"],
            &["api", "ordinary", "required"],
        ),
        (
            "requested deduplicated",
            r#"{"profiles":["reports","base","reports","extra"]}"#,
            &["base", "extra", "reports"],
            &["api", "ordinary", "reporting", "required"],
        ),
        (
            "selected env",
            r#"{"env":"smoke","profiles":["extra"]}"#,
            &["extra", "trends"],
            &["api", "ordinary", "reporting", "required", "trends"],
        ),
    ];
    for (name, body, profiles, services) in cases {
        let h = profile_harness();
        h.loader.update(PROFILES_ROOT, |p| {
            p.pipelines = BTreeMap::from([(
                "check".to_string(),
                vec![rocket_domain::Step {
                    run: "job-step".into(),
                    ..Default::default()
                }],
            )]);
            p.pipeline_needs = BTreeMap::from([("check".to_string(), vec!["all".to_string()])]);
        });
        h.runner.exit_now("job-step", 0);
        let mut value: serde_json::Value = serde_json::from_str(body).unwrap();
        value["kind"] = "pipeline".into();
        let mut req: JobRequest = serde_json::from_value(value).unwrap();
        req.project = PROFILES_ROOT.into();
        req.name = "check".into();
        let started = h.start_job(req);
        let finished = h.wait_job(&started.id).await;
        assert_eq!(
            finished.status,
            JobStatus::Succeeded,
            "{name}: {finished:?}"
        );
        assert_eq!(started.profiles, profiles, "{name}: start");
        assert_eq!(finished.profiles, profiles, "{name}: finish");
        let runs = h.store.list_runs().unwrap();
        let mut got: Vec<String> = runs.iter().map(|r| r.service.clone()).collect();
        for r in &runs {
            assert_eq!(r.env, finished.env, "{name}: prerequisite env");
        }
        got.sort();
        assert_eq!(got, services, "{name}: wildcard prerequisites");
        if services.contains(&"reporting") {
            let declared = h.loader_project(PROFILES_ROOT).services["reporting"]
                .profiles
                .clone();
            let want =
                merge_profiles(&[profiles.iter().map(|s| s.to_string()).collect(), declared]);
            assert_eq!(h.run("profiles", "reporting").profiles, want, "{name}");
        }
    }
}

#[tokio::test]
async fn deploy_rejects_requested_profiles_before_persistence() {
    let h = Harness::new();
    let req: JobRequest = serde_json::from_str(
        r#"{"project":"/code/jobs","kind":"deploy","name":"prod","yes":true,"profiles":["extra"]}"#,
    )
    .unwrap();
    let res = h.app.start_job(req);
    let jobs = h.store.list_jobs("", 0).unwrap();
    assert!(
        is_invalid(&res) && jobs.is_empty() && h.argvs().is_empty(),
        "unsupported deployment profiles accepted: {res:?}, {jobs:?}"
    );
}

// --- compose_identity_test.go (job cases) -----------------------------------------------

#[tokio::test]
async fn jobs_force_compose_project_identity_and_persist_environment() {
    struct Case(
        &'static str,
        JobKind,
        &'static str,
        &'static str,
        &'static str,
        usize,
    );
    let cases = [
        Case("default pipeline", JobKind::Pipeline, "ci", "", "dev", 3),
        Case(
            "selected pipeline",
            JobKind::Pipeline,
            "ci",
            "stage",
            "stage",
            3,
        ),
        Case("default setup sequence", JobKind::Setup, "", "", "dev", 2),
        Case(
            "selected setup",
            JobKind::Setup,
            "doctor",
            "stage",
            "stage",
            1,
        ),
        Case(
            "task fallback",
            JobKind::Pipeline,
            "build",
            "prod",
            "prod",
            1,
        ),
        Case("deployment target", JobKind::Deploy, "prod", "", "prod", 1),
        Case(
            "matching deploy env",
            JobKind::Deploy,
            "prod",
            "prod",
            "prod",
            1,
        ),
    ];
    for Case(name, kind, job_name, env, want_env, want_steps) in cases {
        let h = Harness::new();
        h.set_base_env(&[
            "COMPOSE_PROJECT_NAME=inherited-override",
            "ROCKET_OWNER=agent:leak",
        ]);
        h.env.set(&[
            ("COMPOSE_PROJECT_NAME", "dotenv-override"),
            ("SHARED", "value"),
        ]);
        h.runner.exit_now("", 0);
        let mut req = job_req(kind, job_name);
        req.owner = "agent:owner".into();
        req.yes = true;
        req.allow_agent_deploy = true;
        req.env = env.into();
        let job = h.start_job(req);
        assert_eq!(job.env, want_env, "{name}: returned env");
        let finished = h.wait_job(&job.id).await;
        assert_eq!(finished.env, want_env, "{name}");
        assert_eq!(
            finished.status,
            JobStatus::Succeeded,
            "{name}: {finished:?}"
        );
        let started = h.runner.started();
        assert_eq!(started.len(), want_steps, "{name}");
        for (i, spec) in started.iter().enumerate() {
            let env = env_map(&spec.env);
            assert_eq!(
                env["COMPOSE_PROJECT_NAME"],
                format!("rocket-jobs-{want_env}"),
                "{name}: step {}",
                i + 1
            );
            assert!(!env.contains_key("ROCKET_OWNER"), "{name}: owner leaked");
            assert_eq!(env["SHARED"], "value", "{name}: lost dotenv value");
        }
    }
}

#[tokio::test]
async fn jobs_reject_invalid_environment_before_persistence() {
    let cases = [
        ("unknown pipeline env", JobKind::Pipeline, "ci", "missing"),
        ("unknown setup env", JobKind::Setup, "doctor", "missing"),
        ("inconsistent deploy env", JobKind::Deploy, "prod", "stage"),
    ];
    for (name, kind, job_name, env) in cases {
        let h = Harness::new();
        let mut req = job_req(kind, job_name);
        req.project = "/code/jobs".into();
        req.yes = true;
        req.env = env.into();
        let res = h.app.start_job(req);
        match &res {
            Err(AppError::Invalid(msg)) if msg.contains(env) => {}
            other => panic!("{name}: want invalid env {env:?}, got {other:?}"),
        }
        let jobs = h.store.list_jobs("", 0).unwrap();
        assert!(jobs.is_empty() && h.argvs().is_empty(), "{name}: {jobs:?}");
    }
}

// --- pipeline_needs_test.go helpers ---------------------------------------------------

const NEEDS_ROOT: &str = "/code/needs";

fn needs_harness() -> (Harness, Project) {
    let p = rocket_manifest::parse(
        br"version: 1
name: needs
envs: {dev: {}, smoke: {}}
services:
  db: {run: db-serve, cwd: db, ports: {main: {default: 6432, env: DB_PORT}}}
  api: {run: api-serve, cwd: api, depends_on: [db], ports: {http: {default: 8080, env: API_PORT}}}
groups: {infra: [db]}
pipelines:
  check: {needs: [infra, api], steps: [{run: job-step}]}
",
        NEEDS_ROOT,
    )
    .expect("parse pipeline needs");
    let h = Harness::new();
    h.loader.set(p.clone());
    h.runner.exit_now("job-step", 0);
    (h, p)
}

fn check_req() -> JobRequest {
    let mut req = job_req(JobKind::Pipeline, "check");
    req.project = NEEDS_ROOT.into();
    req
}

/// Health probe that blocks readiness of one port until released.
struct BlockingHealth {
    port: u16,
    entered: Notify,
    release: watch::Sender<bool>,
}

#[async_trait]
impl HealthProbe for BlockingHealth {
    async fn check(&self, c: &HealthCheck) -> ports::Result<()> {
        if c.port != self.port {
            return Ok(());
        }
        self.entered.notify_one();
        let mut rx = self.release.subscribe();
        let _ = rx.wait_for(|released| *released).await;
        Ok(())
    }
}

fn block_needs_health(h: &mut Harness, port: u16) -> Arc<BlockingHealth> {
    let b = Arc::new(BlockingHealth {
        port,
        entered: Notify::new(),
        release: watch::channel(false).0,
    });
    let probe = b.clone();
    h.reconfigure(|d| d.health = probe);
    b
}

async fn wait_entered(entered: &Notify) {
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .expect("prerequisite startup never reached readiness");
}

fn assert_no_job_steps(h: &Harness) {
    for argv in h.argvs() {
        assert!(
            !argv.join(" ").contains("job-step"),
            "job step started before prerequisites completed: {argv:?}"
        );
    }
}

/// Compose driver whose `up` records the call, then never returns.
struct BlockingCompose {
    inner: Arc<FakeCompose>,
    entered: Notify,
}

#[async_trait]
impl ComposeDriver for BlockingCompose {
    async fn named_volumes(&self, t: &ComposeTarget, service: &str) -> ports::Result<Vec<String>> {
        self.inner.named_volumes(t, service).await
    }
    async fn volume_exists(&self, t: &ComposeTarget, name: &str) -> ports::Result<bool> {
        self.inner.volume_exists(t, name).await
    }
    async fn up(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> ports::Result<String> {
        let _ = self.inner.up(t, service, out).await;
        self.entered.notify_one();
        std::future::pending().await
    }
    async fn stop(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> ports::Result<()> {
        self.inner.stop(t, service, out).await
    }
    async fn down(&self, t: &ComposeTarget, out: &mut (dyn Write + Send)) -> ports::Result<()> {
        self.inner.down(t, out).await
    }
    async fn running(&self, compose_project: &str, service: &str) -> ports::Result<bool> {
        self.inner.running(compose_project, service).await
    }
    async fn logs(
        &self,
        t: &ComposeTarget,
        service: &str,
        tail: usize,
    ) -> ports::Result<Vec<String>> {
        self.inner.logs(t, service, tail).await
    }
}

/// Makes `db` a Compose service and blocks its startup.
fn block_needs_compose(h: &mut Harness) -> Arc<BlockingCompose> {
    h.loader.update(NEEDS_ROOT, |p| {
        let db = p.services.get_mut("db").unwrap();
        db.kind = rocket_domain::ServiceKind::Compose;
        db.run = String::new();
        db.compose = "database".into();
    });
    let b = Arc::new(BlockingCompose {
        inner: h.compose.clone(),
        entered: Notify::new(),
    });
    let driver = b.clone();
    h.reconfigure(|d| d.compose = driver);
    b
}

fn not_active(h: &Harness, project: &str, service: &str) -> bool {
    let r = h.run(project, service);
    !r.state.active() && !h.runner.alive(r.pid, r.pgid)
}

// --- pipeline_needs_test.go -----------------------------------------------------------------

#[tokio::test]
async fn pipeline_needs_persist_before_async_startup_with_exact_deadline() {
    let (mut h, p) = needs_harness();
    let block = block_needs_health(&mut h, 6432);
    let started = h.now();
    let mut req = check_req();
    req.env = "smoke".into();
    req.owner = "agent:check".into();
    req.ttl = "30m".into();
    let job = h.start_job(req);
    let persisted = h.store.get_job(&job.id).unwrap().expect("persisted job");
    assert_eq!(persisted.status, JobStatus::Running);
    wait_entered(&block.entered).await;
    assert_no_job_steps(&h);
    h.clock.advance(Duration::from_secs(7 * 60));
    block.release.send_replace(true);
    let finished = h.wait_job(&job.id).await;
    assert_eq!(finished.status, JobStatus::Succeeded, "{finished:?}");
    let deadline = job.expires_at.expect("job deadline");
    assert_eq!(deadline, started + time::Duration::minutes(30));
    for name in ["db", "api"] {
        let run = h.run(&p.name, name);
        assert_eq!(run.state, RunState::Running, "{name}");
        assert_eq!(run.owner, "agent:check", "{name}");
        assert_eq!(run.env, "smoke", "{name}");
        assert_eq!(run.expires_at, Some(deadline), "{name}");
    }
}

#[tokio::test]
async fn pipeline_needs_preserve_existing_owner_and_deadline() {
    let (h, p) = needs_harness();
    h.up(UpRequest {
        project: NEEDS_ROOT.into(),
        services: vec!["db".into()],
        owner: "user".into(),
        ttl: "1h".into(),
        ..UpRequest::default()
    })
    .await;
    let existing = h.run(&p.name, "db");
    h.clock.advance(Duration::from_secs(60));
    let mut req = check_req();
    req.owner = "agent:check".into();
    req.ttl = "30m".into();
    let job = h.start_job(req);
    let finished = h.wait_job(&job.id).await;
    assert_eq!(finished.status, JobStatus::Succeeded, "{finished:?}");
    let db = h.run(&p.name, "db");
    assert_eq!(db.pid, existing.pid);
    assert_eq!(db.owner, "user");
    assert_eq!(db.expires_at, existing.expires_at);
    let api = h.run(&p.name, "api");
    assert_eq!(api.expires_at, job.expires_at);
    assert_eq!(api.owner, "agent:check");
}

#[tokio::test]
async fn pipeline_needs_failure_prevents_steps() {
    let (h, p) = needs_harness();
    h.loader.update(NEEDS_ROOT, |p| {
        p.services.get_mut("db").unwrap().ports[0].env = String::new();
    });
    h.probe.set_busy(6432, holder(77, ""));
    let job = h.start_job(check_req());
    let finished = h.wait_job(&job.id).await;
    assert_eq!(finished.status, JobStatus::Failed, "{finished:?}");
    assert!(finished.error.contains("prerequisite"), "{finished:?}");
    assert_eq!(finished.step, 0);
    assert_no_job_steps(&h);
    let _ = p;
}

#[tokio::test]
async fn pipeline_needs_cancellation_during_readiness() {
    let (mut h, p) = needs_harness();
    let block = block_needs_health(&mut h, 8080);
    let job = h.start_job(check_req());
    wait_entered(&block.entered).await;
    let finished = tokio::time::timeout(Duration::from_secs(1), h.app.cancel_job(&job.id))
        .await
        .expect("cancel timed out")
        .unwrap();
    assert_eq!(finished.status, JobStatus::Canceled, "{finished:?}");
    assert_no_job_steps(&h);
    assert!(
        not_active(&h, &p.name, "api"),
        "interrupted startup remains active"
    );
    assert_eq!(
        h.run(&p.name, "db").state,
        RunState::Running,
        "previously ready prerequisite should remain running"
    );
}

#[tokio::test]
async fn pipeline_needs_cancellation_while_waiting_project_lock() {
    let (h, p) = needs_harness();
    let _gate = h.app.lock_project(&p.name).await;
    let job = h.start_job(check_req());
    let finished = tokio::time::timeout(Duration::from_secs(1), h.app.cancel_job(&job.id))
        .await
        .expect("queued startup must cancel before lock release")
        .unwrap();
    assert_eq!(finished.status, JobStatus::Canceled, "{finished:?}");
    assert!(h.argvs().is_empty(), "{:?}", h.argvs());
}

#[tokio::test]
async fn up_cancellation_while_waiting_project_lock() {
    let (h, p) = needs_harness();
    let gate = h.app.lock_project(&p.name).await;
    let token = CancellationToken::new();
    let app = h.app.clone();
    let tok = token.clone();
    let mut req = req(NEEDS_ROOT, &["api"]);
    req.project = NEEDS_ROOT.into();
    let handle = tokio::spawn(async move { app.up_with(req, &tok, None).await });
    token.cancel();
    let res = tokio::time::timeout(Duration::from_millis(200), handle).await;
    drop(gate);
    let res = res
        .expect("canceled up waited for the held project lock")
        .unwrap();
    assert_eq!(res.unwrap_err(), AppError::Canceled);
    assert!(h.argvs().is_empty(), "{:?}", h.argvs());
}

#[tokio::test]
async fn down_cancels_pipeline_before_prerequisite_snapshot() {
    let (mut h, p) = needs_harness();
    let block = block_needs_health(&mut h, 8080);
    let job = h.start_job(check_req());
    wait_entered(&block.entered).await;
    let down = tokio::time::timeout(
        Duration::from_secs(1),
        h.app.down(DownRequest {
            project: NEEDS_ROOT.into(),
            ..DownRequest::default()
        }),
    )
    .await
    .expect("down deadlocked behind prerequisite startup")
    .unwrap();
    assert!(down.canceled_jobs.contains(&job.id), "{down:?}");
    assert_no_job_steps(&h);
    for run in h.store.list_runs().unwrap() {
        assert!(
            !run.state.active() && !h.runner.alive(run.pid, run.pgid),
            "down left a prerequisite active: {run:?}"
        );
    }
    assert!(h.store.list_leases().unwrap().is_empty());
    let _ = p;
}

#[tokio::test]
async fn pipeline_needs_cancellation_stops_attempted_compose_service() {
    let (mut h, p) = needs_harness();
    let compose = block_needs_compose(&mut h);
    let job = h.start_job(check_req());
    wait_entered(&compose.entered).await;
    let finished = tokio::time::timeout(Duration::from_secs(1), h.app.cancel_job(&job.id))
        .await
        .expect("cancel timed out")
        .unwrap();
    assert_eq!(finished.status, JobStatus::Canceled, "{finished:?}");
    assert_eq!(
        h.compose.ops(),
        ["up:database", "stop:database"],
        "cancellation must stop only the attempted compose service"
    );
    let name = compose_project_name(&p.name, "dev");
    assert!(!h.compose.running(&name, "database").await.unwrap());
    assert_no_job_steps(&h);
    for request in [
        DownRequest {
            project: NEEDS_ROOT.into(),
            owner: "user".into(),
            ..DownRequest::default()
        },
        DownRequest {
            project: NEEDS_ROOT.into(),
            services: vec!["db".into()],
            ..DownRequest::default()
        },
    ] {
        let down = h.app.down(request).await.unwrap();
        assert!(
            down.compose_down.is_empty() && down.stopped.is_empty(),
            "selective/owner down must not remove a whole compose project: {down:?}"
        );
    }
    let whole = DownRequest {
        project: NEEDS_ROOT.into(),
        ..DownRequest::default()
    };
    let down = h.app.down(whole.clone()).await.unwrap();
    assert_eq!(down.compose_down, [name], "{down:?}");
    let again = h.app.down(whole).await.unwrap();
    assert!(
        again.stopped.is_empty() && again.compose_down.is_empty(),
        "{again:?}"
    );
}

#[tokio::test]
async fn up_cancellation_releases_attempted_service_leases() {
    let (mut h, p) = needs_harness();
    h.up(UpRequest {
        project: NEEDS_ROOT.into(),
        services: vec!["db".into()],
        ..UpRequest::default()
    })
    .await;
    let block = block_needs_health(&mut h, 8080);
    let token = CancellationToken::new();
    let (app, tok) = (h.app.clone(), token.clone());
    let request = req(NEEDS_ROOT, &["api"]);
    let handle = tokio::spawn(async move { app.up_with(request, &tok, None).await });
    wait_entered(&block.entered).await;
    token.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("canceled startup did not unwind")
        .unwrap();
    match result {
        Ok(r) => assert!(r.failed(), "canceled startup did not fail: {r:?}"),
        Err(e) => assert_eq!(e, AppError::Canceled),
    }
    let leases = h.store.list_leases().unwrap();
    assert!(
        leases.len() == 1 && leases[0].service == "db",
        "cancellation must release only attempted startup leases: {leases:?}"
    );
    assert!(not_active(&h, &p.name, "api"), "orphan startup process");
}

#[tokio::test]
async fn pipeline_needs_reject_invalid_ttl_and_targets_before_persistence() {
    for invalid in ["not-duration", "0s", "-1s", "unknown-target"] {
        let (h, _p) = needs_harness();
        let mut request = check_req();
        if invalid == "unknown-target" {
            h.loader.update(NEEDS_ROOT, |p| {
                p.pipeline_needs
                    .insert("check".into(), vec!["missing".into()]);
            });
        } else {
            request.ttl = invalid.into();
        }
        let res = h.app.start_job(request);
        let jobs = h.store.list_jobs("", 0).unwrap();
        assert!(
            is_invalid(&res) && jobs.is_empty() && h.argvs().is_empty(),
            "{invalid}: side effects: {res:?}, {jobs:?}"
        );
    }
}

// --- job_ttl_test.go ---------------------------------------------------------------------------

fn assert_job_expired(h: &Harness, id: &str) -> Job {
    let j = h.app.get_job(id).unwrap();
    assert!(
        j.status == JobStatus::Canceled && j.error == "ttl expired" && j.finished_at.is_some(),
        "expired job = {j:?}"
    );
    let terminals = h
        .bus
        .events()
        .iter()
        .filter(|e| {
            e.job_id == id
                && e.r#type == event_type::JOB_STATE
                && e.status.is_some_and(JobStatus::terminal)
        })
        .count();
    assert_eq!(terminals, 1, "terminal events");
    j
}

/// Wraps the fake runner with clock side effects (Go: `deadlineRunner`,
/// `deadlineLiveness`).
struct ClockRunner {
    inner: Arc<FakeRunner>,
    clock: Arc<TestClock>,
    advance_on_start: bool,
    dead_on_alive: bool,
}

#[async_trait]
impl ProcessRunner for ClockRunner {
    fn start(&self, spec: ProcessSpec) -> ports::Result<ProcessHandle> {
        let h = self.inner.start(spec);
        if self.advance_on_start {
            self.clock.advance(Duration::from_secs(3600));
        }
        h
    }
    async fn stop(&self, pgid: i32, grace: Duration) -> ports::Result<()> {
        self.inner.stop(pgid, grace).await
    }
    fn alive(&self, pid: i32, pgid: i32) -> bool {
        if self.dead_on_alive {
            self.clock.advance(Duration::from_secs(3600));
            return false;
        }
        self.inner.alive(pid, pgid)
    }
}

struct ClockHealth(Arc<TestClock>);

#[async_trait]
impl HealthProbe for ClockHealth {
    async fn check(&self, _c: &HealthCheck) -> ports::Result<()> {
        self.0.advance(Duration::from_secs(3600));
        Ok(())
    }
}

#[tokio::test]
async fn gc_expires_job_step_with_reason_and_identity() {
    let h = Harness::new();
    let mut req = job_req(JobKind::Pipeline, "slow");
    req.ttl = "1h".into();
    let j = h.start_job(req);
    let running = h.wait_job_pgid(&j.id).await;
    assert_eq!(running.expires_at, Some(h.now() + time::Duration::hours(1)));
    h.clock.advance(Duration::from_secs(3600));
    let res = h.app.gc().await.unwrap();
    assert_job_expired(&h, &j.id);
    assert!(
        res.actions
            .iter()
            .any(|a| a.action == gc_action::EXPIRED_JOB && a.detail == j.id && a.project == "jobs"),
        "GC omitted expired_job identity: {res:?}"
    );
    let count = h
        .runner
        .stopped()
        .iter()
        .filter(|p| **p == running.pgid)
        .count();
    assert_eq!(
        count,
        1,
        "step stopped {count} times: {:?}",
        h.runner.stopped()
    );
}

#[tokio::test]
async fn reconcile_expired_persisted_jobs_before_lost_or_adopted() {
    for live in [false, true] {
        let h = Harness::new();
        let deadline = h.now() - time::Duration::seconds(1);
        let mut j = running_job("jexpired", 0, deadline - time::Duration::hours(1));
        (j.pid, j.pgid) = (0, 0);
        j.expires_at = Some(deadline);
        if live {
            (j.pid, j.pgid) = (6666, 6666);
            h.runner.set_alive(6666, true);
        }
        h.store.save_job(&j).unwrap();
        let res = h.app.reconcile().await.unwrap();
        assert!(
            res.lost_jobs.is_empty(),
            "live={live}: expired job marked lost: {res:?}"
        );
        assert_job_expired(&h, &j.id);
        assert!(
            !(live && h.runner.alive(j.pid, j.pgid)),
            "expired adopted process remains alive"
        );
    }
}

#[tokio::test]
async fn adopted_job_retains_deadline() {
    let h = Harness::new();
    let deadline = h.now() + time::Duration::hours(1);
    let mut j = running_job("jadopted", 6666, h.now());
    j.expires_at = Some(deadline);
    h.runner.set_alive(6666, true);
    h.store.save_job(&j).unwrap();
    h.app.reconcile().await.unwrap();
    let got = h.app.get_job(&j.id).unwrap();
    assert_eq!(got.status, JobStatus::Running, "{got:?}");
    assert_eq!(got.expires_at, Some(deadline));
    h.clock.advance(Duration::from_secs(3600));
    h.app.gc().await.unwrap();
    assert_job_expired(&h, &j.id);
}

#[tokio::test]
async fn job_ttl_timer_expires_without_maintenance() {
    let mut h = Harness::new();
    h.reconfigure(|d| d.options.now = Some(Arc::new(OffsetDateTime::now_utc)));
    let mut req = job_req(JobKind::Pipeline, "slow");
    req.ttl = "25ms".into();
    let j = h.start_job(req);
    h.wait_job_pgid(&j.id).await;
    let got = h.wait_job(&j.id).await;
    assert_eq!(got.status, JobStatus::Canceled, "{got:?}");
    assert_eq!(got.error, "ttl expired");
}

#[tokio::test]
async fn job_ttl_interrupts_blocked_prerequisite_and_project_gate() {
    for mode in ["readiness", "project gate", "compose"] {
        let (mut h, p) = needs_harness();
        let (health, compose);
        let mut gate = None;
        match mode {
            "readiness" => health = Some(block_needs_health(&mut h, 6432)),
            "project gate" => {
                health = None;
                gate = Some(h.app.lock_project(&p.name).await);
            }
            _ => health = None,
        }
        if mode == "compose" {
            compose = Some(block_needs_compose(&mut h));
        } else {
            compose = None;
        }
        let mut req = check_req();
        req.ttl = "1h".into();
        let job = h.start_job(req);
        if let Some(b) = &health {
            wait_entered(&b.entered).await;
        }
        if let Some(b) = &compose {
            wait_entered(&b.entered).await;
        }
        h.clock.advance(Duration::from_secs(3600));
        tokio::time::timeout(Duration::from_secs(1), h.app.gc())
            .await
            .unwrap_or_else(|_| panic!("expiry blocked by {mode}"))
            .unwrap();
        let j = assert_job_expired(&h, &job.id);
        assert_eq!(j.step, 0, "{mode}: steps ran after startup expiry: {j:?}");
        drop(gate);
    }
}

#[tokio::test]
async fn reconcile_deadline_crossing_reports_expired_not_lost() {
    let mut h = Harness::new();
    let deadline = h.now() + time::Duration::hours(1);
    let mut job = running_job("jcrossing", 6666, h.now());
    job.expires_at = Some(deadline);
    h.store.save_job(&job).unwrap();
    let wrapped = Arc::new(ClockRunner {
        inner: h.runner.clone(),
        clock: h.clock.clone(),
        advance_on_start: false,
        dead_on_alive: true,
    });
    h.reconfigure(|d| d.runner = wrapped);
    let result = h.app.reconcile().await.unwrap();
    assert!(result.lost_jobs.is_empty(), "{result:?}");
    assert_eq!(result.expired_jobs, [job.id.clone()]);
    assert_job_expired(&h, &job.id);
}

#[tokio::test]
async fn job_deadline_checked_at_transitions_before_first_step() {
    let (mut h, p) = needs_harness();
    let clock = h.clock.clone();
    h.reconfigure(|d| d.health = Arc::new(ClockHealth(clock)));
    let mut req = check_req();
    req.ttl = "1h".into();
    let job = h.start_job(req);
    h.wait_job(&job.id).await;
    let finished = assert_job_expired(&h, &job.id);
    assert_eq!(finished.step, 0, "expired startup ran a step: {finished:?}");
    assert_no_job_steps(&h);
    let _ = p;
}

#[tokio::test]
async fn job_deadline_checked_at_transitions_between_steps() {
    for pipeline in ["ci", "slow"] {
        let mut h = Harness::new();
        for command in ["step-lint", "step-unit", "task e2e", "step-slow"] {
            h.runner.exit_now(command, 0);
        }
        let wrapped = Arc::new(ClockRunner {
            inner: h.runner.clone(),
            clock: h.clock.clone(),
            advance_on_start: true,
            dead_on_alive: false,
        });
        h.reconfigure(|d| d.runner = wrapped);
        let mut req = job_req(JobKind::Pipeline, pipeline);
        req.ttl = "1h".into();
        let job = h.start_job(req);
        h.wait_job(&job.id).await;
        let finished = assert_job_expired(&h, &job.id);
        assert!(
            finished.step == 1 && h.argvs().len() == 1,
            "{pipeline}: steps continued after deadline: {finished:?}, {:?}",
            h.argvs()
        );
    }
}

#[tokio::test]
async fn gc_retains_failed_compose_prerequisite_until_whole_down() {
    let (mut h, p) = needs_harness();
    let compose = block_needs_compose(&mut h);
    let mut req = check_req();
    req.ttl = "1h".into();
    let job = h.start_job(req);
    wait_entered(&compose.entered).await;
    h.clock.advance(Duration::from_secs(3600));
    h.app.gc().await.unwrap();
    assert_job_expired(&h, &job.id);
    let down = h
        .app
        .down(DownRequest {
            project: NEEDS_ROOT.into(),
            ..DownRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(
        down.compose_down,
        [compose_project_name(&p.name, "dev")],
        "GC discarded the failed Compose cleanup record: {down:?}"
    );
    h.app.gc().await.unwrap();
    assert!(
        h.store.get_run(&p.name, "db").unwrap().is_none(),
        "successfully stopped Compose record was not pruned"
    );
}

// --- Rust-only ---------------------------------------------------------------------------------

#[tokio::test]
async fn wait_job_returns_canceled_when_token_fires() {
    let h = Harness::new();
    let job = h.start_job(job_req(JobKind::Pipeline, "slow"));
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        h.app.wait_job(&job.id, &token).await.unwrap_err(),
        AppError::Canceled
    );
    h.app.cancel_job(&job.id).await.unwrap();
}

#[tokio::test]
async fn invalid_job_ttl_and_unknown_kind_inputs() {
    let h = Harness::new();
    let mut req = job_req(JobKind::Pipeline, "slow");
    req.ttl = "5".into();
    assert!(is_invalid(&h.app.start_job(req)));
    let res = h.app.start_job(job_req(JobKind::Pipeline, ""));
    assert!(is_invalid(&res), "{res:?}");
}

#[tokio::test]
async fn list_jobs_limits_and_job_logs_tail() {
    let h = Harness::new();
    h.runner.exit_now("step-slow", 0);
    let mut ids = Vec::new();
    for _ in 0..3 {
        let j = h.start_job(job_req(JobKind::Pipeline, "slow"));
        h.wait_job(&j.id).await;
        h.clock.advance(Duration::from_secs(1));
        ids.push(j.id);
    }
    let limited = h
        .app
        .list_jobs(&JobsRequest {
            all_projects: true,
            limit: 2,
            ..JobsRequest::default()
        })
        .unwrap();
    assert_eq!(limited.jobs.len(), 2);
    assert_eq!(limited.jobs[0].id, ids[2]);
    let logs = h.app.job_logs(&ids[0], 2).unwrap();
    assert_eq!(logs.project, "jobs");
    assert_eq!(logs.lines.len(), 2);
    assert!(logs.lines[1].contains("succeeded"), "{:?}", logs.lines);
    let o = outcome(&h.app.get_job(&ids[0]).unwrap(), logs.lines);
    assert_eq!((o.status, o.exit_code), (JobStatus::Succeeded, Some(0)));
}

#[tokio::test]
async fn gc_prunes_finished_jobs_beyond_the_newest_hundred() {
    let h = Harness::new();
    let base = h.now();
    for i in 0..(KEEP_JOBS + 3) {
        let mut j = running_job(
            &format!("j{i:04}"),
            0,
            base + time::Duration::seconds(i as i64),
        );
        j.status = JobStatus::Succeeded;
        j.finished_at = Some(j.started_at);
        h.store.save_job(&j).unwrap();
    }
    let mut running = running_job("jrunning", 0, base - time::Duration::days(1));
    running.pid = 6666;
    running.pgid = 6666;
    h.runner.set_alive(6666, true);
    h.store.save_job(&running).unwrap();
    let res = h.app.gc().await.unwrap();
    let pruned: Vec<&str> = res
        .actions
        .iter()
        .filter(|a| a.action == gc_action::PRUNED_JOB)
        .map(|a| a.detail.as_str())
        .collect();
    assert_eq!(pruned.len(), 3, "{pruned:?}");
    assert!(pruned.contains(&"j0000") && pruned.contains(&"j0002"));
    assert_eq!(h.store.list_jobs("", 0).unwrap().len(), KEEP_JOBS + 1);
}
