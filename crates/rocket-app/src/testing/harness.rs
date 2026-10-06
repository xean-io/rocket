//! The shared test harness (Go: `harness` in `app_test.go`).

use super::fakes::{
    FakeCompose, FakeEnv, FakeHealth, FakeLoader, FakeProbe, FakeRunner, FakeTask, FileLogs,
    MemStore, RecBus, TestClock,
};
use super::projects::{jobs_project, nuvara, other_project};
use crate::{App, Deps, Options};
use rocket_domain::api::{JobRequest, UpRequest, UpResult};
use rocket_domain::{Health, PortHolder, Run, RunState, ServiceKind};
use rocket_domain::{Job, JobStatus};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::datetime;
use tokio_util::sync::CancellationToken;

pub struct Harness {
    pub app: App,
    pub deps: Deps,
    pub store: Arc<MemStore>,
    pub runner: Arc<FakeRunner>,
    pub compose: Arc<FakeCompose>,
    pub probe: Arc<FakeProbe>,
    pub health: Arc<FakeHealth>,
    pub bus: Arc<RecBus>,
    pub clock: Arc<TestClock>,
    pub loader: Arc<FakeLoader>,
    pub env: Arc<FakeEnv>,
    pub base_env: Arc<Mutex<Vec<String>>>,
    _logs: tempfile::TempDir,
}

impl Harness {
    pub fn new() -> Self {
        let logs = tempfile::tempdir().expect("temp log dir");
        let store = Arc::new(MemStore::default());
        let runner = Arc::new(FakeRunner::default());
        let compose = Arc::new(FakeCompose::default());
        let probe = Arc::new(FakeProbe::default());
        let health = Arc::new(FakeHealth::default());
        let bus = Arc::new(RecBus::default());
        let clock = Arc::new(TestClock::new(datetime!(2026-01-01 10:00:00 UTC)));
        let loader = Arc::new(FakeLoader::default());
        for p in [nuvara(), other_project(), jobs_project()] {
            loader.set(p);
        }
        let env = Arc::new(FakeEnv::default());
        env.set(&[("FROM_DOTENV", "1")]);
        let base_env = Arc::new(Mutex::new(vec![
            "PATH=/usr/bin".to_string(),
            "ROCKET_OWNER=agent:leak".to_string(),
        ]));
        let (clock_now, base) = (clock.clone(), base_env.clone());
        let deps = Deps {
            store: store.clone(),
            manifests: loader.clone(),
            env: env.clone(),
            runner: runner.clone(),
            compose: compose.clone(),
            tasks: Arc::new(FakeTask),
            probe: probe.clone(),
            health: health.clone(),
            bus: Some(bus.clone()),
            logs: Arc::new(FileLogs {
                dir: logs.path().to_path_buf(),
            }),
            options: Options {
                now: Some(Arc::new(move || clock_now.now())),
                base_env: Some(Arc::new(move || {
                    base.lock().expect("base env lock").clone()
                })),
                new_id: None,
                health_interval: Duration::from_millis(1),
                stop_grace: Duration::from_millis(1),
                poll_interval: Duration::from_millis(5),
                start_grace: Duration::from_millis(5),
            },
        };
        Self {
            app: App::new(deps.clone()),
            deps,
            store,
            runner,
            compose,
            probe,
            health,
            bus,
            clock,
            loader,
            env,
            base_env,
            _logs: logs,
        }
    }

    /// Rebuilds the application after changing its dependencies. The fakes
    /// (and the state they hold) are shared with the previous instance.
    pub fn reconfigure(&mut self, f: impl FnOnce(&mut Deps)) {
        f(&mut self.deps);
        self.app = App::new(self.deps.clone());
    }

    pub fn set_base_env(&self, entries: &[&str]) {
        *self.base_env.lock().expect("base env lock") =
            entries.iter().map(|e| (*e).to_string()).collect();
    }

    /// Runs `up`, defaulting the project to `/code/nuvara`, and requires
    /// success.
    pub async fn up(&self, mut req: UpRequest) -> UpResult {
        if req.project.is_empty() {
            req.project = "/code/nuvara".into();
        }
        self.app.up(req).await.expect("up")
    }

    /// Starts a job, defaulting the project to `/code/jobs`, and requires a
    /// running job back.
    pub fn start_job(&self, mut req: JobRequest) -> Job {
        if req.project.is_empty() {
            req.project = "/code/jobs".into();
        }
        let job = self.app.start_job(req).expect("start job");
        assert!(
            !job.id.is_empty() && job.status == JobStatus::Running,
            "started job {job:?}"
        );
        job
    }

    /// Waits up to 5 seconds for the job to finish.
    pub async fn wait_job(&self, id: &str) -> Job {
        let cancel = CancellationToken::new();
        let wait = self.app.wait_job(id, &cancel);
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .unwrap_or_else(|_| panic!("job {id} did not finish"))
            .unwrap_or_else(|e| panic!("wait job {id}: {e}"))
    }

    /// Waits until the job's current step has a process group.
    pub async fn wait_job_pgid(&self, id: &str) -> Job {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(j) = self.app.get_job(id)
                && j.pgid > 0
            {
                return j;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        panic!("job {id} never started a process");
    }

    /// argv of every process started so far.
    pub fn argvs(&self) -> Vec<Vec<String>> {
        self.runner.started().into_iter().map(|s| s.argv).collect()
    }

    pub fn run(&self, project: &str, service: &str) -> Run {
        use rocket_domain::ports::Store;
        self.store
            .get_run(project, service)
            .expect("store")
            .unwrap_or_else(|| panic!("no run for {project}/{service}"))
    }

    /// The current manifest of `root` in the fake loader.
    pub fn loader_project(&self, root: &str) -> rocket_domain::Project {
        use rocket_domain::ports::ManifestLoader;
        self.loader
            .load(std::path::Path::new(root))
            .expect("project is registered in the fake loader")
    }

    pub fn now(&self) -> OffsetDateTime {
        self.clock.now()
    }
}

/// `UpRequest` for `project` and `services`.
pub fn req(project: &str, services: &[&str]) -> UpRequest {
    UpRequest {
        project: project.into(),
        services: services.iter().map(|s| (*s).to_string()).collect(),
        ..UpRequest::default()
    }
}

/// `UpRequest` for services of the default `/code/nuvara` project.
pub fn nuvara_req(services: &[&str]) -> UpRequest {
    req("/code/nuvara", services)
}

/// service -> action of an up result.
pub fn actions(res: &UpResult) -> BTreeMap<String, String> {
    res.services
        .iter()
        .map(|s| (s.service.clone(), s.action.clone()))
        .collect()
}

/// `KEY=value` entries as a map.
pub fn env_map(env: &[String]) -> BTreeMap<String, String> {
    env.iter()
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_string(), v.to_string())
        })
        .collect()
}

/// Polls `cond` for up to 2 seconds.
pub async fn eventually(mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("condition not met in time");
}

/// A run with only its identity and state set.
pub fn blank_run(project: &str, service: &str, kind: ServiceKind, state: RunState) -> Run {
    Run {
        project: project.into(),
        service: service.into(),
        env: String::new(),
        kind,
        state,
        health: Health::Unknown,
        pid: 0,
        pgid: 0,
        compose_project: String::new(),
        profiles: Vec::new(),
        container_id: String::new(),
        owner: String::new(),
        expires_at: None,
        started_at: None,
        stopped_at: None,
        exit_code: None,
        ports: BTreeMap::new(),
        log_path: String::new(),
        error: String::new(),
    }
}

/// A port holder with no working directory.
pub fn holder(pid: i32, command: &str) -> PortHolder {
    PortHolder {
        pid,
        command: command.into(),
        cwd: String::new(),
    }
}
