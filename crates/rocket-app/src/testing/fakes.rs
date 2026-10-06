//! In-memory port fakes (Go: `app/fakes_test.go`).

use rocket_domain::ports::{
    self, ComposeDriver, ComposeTarget, EnvSource, Error, EventBus, HealthProbe, LogSink,
    ManifestLoader, PortProbe, ProcessHandle, ProcessRunner, ProcessSpec, Store, Subscription,
    TaskDriver, async_trait,
};
use rocket_domain::{Event, HealthCheck, Job, Lease, PortHolder, Project, ProjectRef, Run};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn run_key(project: &str, service: &str) -> String {
    format!("{project}/{service}")
}

// --- store ---------------------------------------------------------------

#[derive(Default)]
struct StoreData {
    projects: BTreeMap<String, ProjectRef>,
    runs: BTreeMap<String, Run>,
    leases: BTreeMap<u16, Lease>,
    jobs: HashMap<String, Job>,
}

/// An in-memory [`Store`].
#[derive(Default)]
pub struct MemStore {
    data: Mutex<StoreData>,
}

impl Store for MemStore {
    fn upsert_project(&self, project: &ProjectRef) -> ports::Result<()> {
        lock(&self.data)
            .projects
            .insert(project.name.clone(), project.clone());
        Ok(())
    }

    fn get_project(&self, name: &str) -> ports::Result<Option<ProjectRef>> {
        Ok(lock(&self.data).projects.get(name).cloned())
    }

    fn list_projects(&self) -> ports::Result<Vec<ProjectRef>> {
        Ok(lock(&self.data).projects.values().cloned().collect())
    }

    fn delete_project(&self, name: &str) -> ports::Result<()> {
        lock(&self.data).projects.remove(name);
        Ok(())
    }

    fn save_run(&self, run: &Run) -> ports::Result<()> {
        lock(&self.data)
            .runs
            .insert(run_key(&run.project, &run.service), run.clone());
        Ok(())
    }

    fn get_run(&self, project: &str, service: &str) -> ports::Result<Option<Run>> {
        Ok(lock(&self.data)
            .runs
            .get(&run_key(project, service))
            .cloned())
    }

    fn list_runs(&self) -> ports::Result<Vec<Run>> {
        Ok(lock(&self.data).runs.values().cloned().collect())
    }

    fn delete_run(&self, project: &str, service: &str) -> ports::Result<()> {
        lock(&self.data).runs.remove(&run_key(project, service));
        Ok(())
    }

    fn acquire_lease(&self, lease: &Lease) -> ports::Result<()> {
        let mut data = lock(&self.data);
        if let Some(cur) = data.leases.get(&lease.port)
            && (cur.project != lease.project || cur.service != lease.service)
        {
            return Err(Error::lease_taken(lease.port, &cur.project, &cur.service));
        }
        data.leases.insert(lease.port, lease.clone());
        Ok(())
    }

    fn release_leases(&self, project: &str, service: &str) -> ports::Result<Vec<Lease>> {
        let mut data = lock(&self.data);
        let ports: Vec<u16> = data
            .leases
            .values()
            .filter(|l| l.project == project && l.service == service)
            .map(|l| l.port)
            .collect();
        Ok(ports
            .into_iter()
            .filter_map(|p| data.leases.remove(&p))
            .collect())
    }

    fn list_leases(&self) -> ports::Result<Vec<Lease>> {
        Ok(lock(&self.data).leases.values().cloned().collect())
    }

    fn save_job(&self, job: &Job) -> ports::Result<()> {
        lock(&self.data).jobs.insert(job.id.clone(), job.clone());
        Ok(())
    }

    fn get_job(&self, id: &str) -> ports::Result<Option<Job>> {
        Ok(lock(&self.data).jobs.get(id).cloned())
    }

    fn list_jobs(&self, project: &str, limit: i64) -> ports::Result<Vec<Job>> {
        let mut out: Vec<Job> = lock(&self.data)
            .jobs
            .values()
            .filter(|j| project.is_empty() || j.project == project)
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        if limit > 0 {
            out.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        Ok(out)
    }

    fn delete_job(&self, id: &str) -> ports::Result<()> {
        lock(&self.data).jobs.remove(id);
        Ok(())
    }
}

// --- manifests and env -----------------------------------------------------

/// A [`ManifestLoader`] over projects keyed by root. Go's fake returns
/// shared pointers that tests mutate; here tests use [`FakeLoader::update`].
#[derive(Default)]
pub struct FakeLoader {
    projects: Mutex<HashMap<String, Project>>,
}

impl FakeLoader {
    pub fn set(&self, p: Project) {
        lock(&self.projects).insert(p.root.clone(), p);
    }

    pub fn update(&self, root: &str, f: impl FnOnce(&mut Project)) {
        f(lock(&self.projects)
            .get_mut(root)
            .expect("project is registered in the fake loader"));
    }

    pub fn clear(&self) {
        lock(&self.projects).clear();
    }
}

impl ManifestLoader for FakeLoader {
    fn load(&self, dir: &Path) -> ports::Result<Project> {
        lock(&self.projects)
            .get(dir.to_string_lossy().as_ref())
            .cloned()
            .ok_or_else(|| Error::msg(format!("no rocket.yaml in {}", dir.display())))
    }
}

#[derive(Default)]
pub struct FakeEnv {
    values: Mutex<BTreeMap<String, String>>,
}

impl FakeEnv {
    pub fn set(&self, values: &[(&str, &str)]) {
        *lock(&self.values) = values
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
    }
}

impl EnvSource for FakeEnv {
    fn dotenv(&self, _root: &Path, _files: &[String]) -> ports::Result<BTreeMap<String, String>> {
        Ok(lock(&self.values).clone())
    }
}

// --- process runner ------------------------------------------------------------

/// What the fake runner was asked to start (`ProcessSpec` minus the log file).
#[derive(Debug, Clone)]
pub struct StartedSpec {
    pub argv: Vec<String>,
    pub dir: PathBuf,
    pub env: Vec<String>,
}

struct FakeProc {
    done: Option<oneshot::Sender<i32>>,
    dead: bool,
}

struct RunnerState {
    next: i32,
    procs: HashMap<i32, FakeProc>,
    started: Vec<StartedSpec>,
    stopped: Vec<i32>,
    /// argv substring -> immediate exit code.
    exit_now: Vec<(String, i32)>,
    /// Overrides for adopted pids.
    alive: HashMap<i32, bool>,
    start_err: Option<String>,
}

pub struct FakeRunner {
    state: Mutex<RunnerState>,
}

impl Default for FakeRunner {
    fn default() -> Self {
        Self {
            state: Mutex::new(RunnerState {
                next: 1000,
                procs: HashMap::new(),
                started: Vec::new(),
                stopped: Vec::new(),
                exit_now: Vec::new(),
                alive: HashMap::new(),
                start_err: None,
            }),
        }
    }
}

impl FakeRunner {
    pub fn started(&self) -> Vec<StartedSpec> {
        lock(&self.state).started.clone()
    }

    pub fn stopped(&self) -> Vec<i32> {
        lock(&self.state).stopped.clone()
    }

    pub fn exit_now(&self, argv_substring: &str, code: i32) {
        lock(&self.state)
            .exit_now
            .push((argv_substring.to_string(), code));
    }

    pub fn set_alive(&self, pid: i32, alive: bool) {
        lock(&self.state).alive.insert(pid, alive);
    }

    pub fn set_start_error(&self, message: Option<&str>) {
        lock(&self.state).start_err = message.map(str::to_string);
    }

    /// Makes `pid` exit with `code`.
    pub fn exit(&self, pid: i32, code: i32) {
        let mut st = lock(&self.state);
        if let Some(p) = st.procs.get_mut(&pid)
            && !p.dead
        {
            p.dead = true;
            if let Some(tx) = p.done.take() {
                let _ = tx.send(code);
            }
        }
    }

    /// The first started process whose directory ends with `cwd_suffix`.
    pub fn spec_for(&self, cwd_suffix: &str) -> StartedSpec {
        lock(&self.state)
            .started
            .iter()
            .find(|s| s.dir.to_string_lossy().ends_with(cwd_suffix))
            .cloned()
            .unwrap_or_else(|| panic!("no process started in {cwd_suffix}"))
    }
}

#[async_trait]
impl ProcessRunner for FakeRunner {
    fn start(&self, spec: ProcessSpec) -> ports::Result<ProcessHandle> {
        let mut st = lock(&self.state);
        if let Some(msg) = &st.start_err {
            return Err(Error::msg(msg.clone()));
        }
        st.next += 1;
        let pid = st.next;
        let (tx, rx) = oneshot::channel();
        let joined = spec.argv.join(" ");
        let mut proc = FakeProc {
            done: Some(tx),
            dead: false,
        };
        if let Some((_, code)) = st.exit_now.iter().find(|(sub, _)| joined.contains(sub)) {
            proc.dead = true;
            if let Some(tx) = proc.done.take() {
                let _ = tx.send(*code);
            }
        }
        st.procs.insert(pid, proc);
        st.started.push(StartedSpec {
            argv: spec.argv,
            dir: spec.dir,
            env: spec.env,
        });
        Ok(ProcessHandle {
            pid,
            pgid: pid,
            done: rx,
        })
    }

    async fn stop(&self, pgid: i32, _grace: Duration) -> ports::Result<()> {
        let mut st = lock(&self.state);
        st.stopped.push(pgid);
        if let Some(p) = st.procs.get_mut(&pgid)
            && !p.dead
        {
            p.dead = true;
            if let Some(tx) = p.done.take() {
                let _ = tx.send(143);
            }
        }
        st.alive.insert(pgid, false);
        Ok(())
    }

    fn alive(&self, pid: i32, _pgid: i32) -> bool {
        let st = lock(&self.state);
        if let Some(v) = st.alive.get(&pid) {
            return *v;
        }
        st.procs.get(&pid).is_some_and(|p| !p.dead)
    }
}

// --- compose -------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ComposeCall {
    pub op: String,
    pub target: ComposeTarget,
    pub service: String,
}

#[derive(Default)]
pub struct FakeCompose {
    calls: Mutex<Vec<ComposeCall>>,
    running: Mutex<HashMap<String, bool>>,
}

impl FakeCompose {
    fn record(&self, op: &str, t: &ComposeTarget, service: &str) {
        lock(&self.calls).push(ComposeCall {
            op: op.to_string(),
            target: t.clone(),
            service: service.to_string(),
        });
    }

    pub fn calls(&self) -> Vec<ComposeCall> {
        lock(&self.calls).clone()
    }

    /// `op:service` for every call, in order.
    pub fn ops(&self) -> Vec<String> {
        lock(&self.calls)
            .iter()
            .map(|c| format!("{}:{}", c.op, c.service))
            .collect()
    }
}

#[async_trait]
impl ComposeDriver for FakeCompose {
    async fn named_volumes(
        &self,
        _t: &ComposeTarget,
        _service: &str,
    ) -> ports::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn volume_exists(&self, _t: &ComposeTarget, _name: &str) -> ports::Result<bool> {
        Ok(false)
    }

    async fn up(
        &self,
        t: &ComposeTarget,
        service: &str,
        _out: &mut (dyn Write + Send),
    ) -> ports::Result<String> {
        self.record("up", t, service);
        lock(&self.running).insert(format!("{}/{service}", t.project_name), true);
        Ok(format!("cid-{service}"))
    }

    async fn stop(
        &self,
        t: &ComposeTarget,
        service: &str,
        _out: &mut (dyn Write + Send),
    ) -> ports::Result<()> {
        self.record("stop", t, service);
        lock(&self.running).insert(format!("{}/{service}", t.project_name), false);
        Ok(())
    }

    async fn down(&self, t: &ComposeTarget, _out: &mut (dyn Write + Send)) -> ports::Result<()> {
        self.record("down", t, "");
        Ok(())
    }

    async fn running(&self, compose_project: &str, service: &str) -> ports::Result<bool> {
        Ok(lock(&self.running)
            .get(&format!("{compose_project}/{service}"))
            .copied()
            .unwrap_or(false))
    }

    async fn logs(
        &self,
        _t: &ComposeTarget,
        _service: &str,
        _tail: usize,
    ) -> ports::Result<Vec<String>> {
        Ok(vec!["compose log".to_string()])
    }
}

// --- task, probes ------------------------------------------------------------------

pub struct FakeTask;

impl TaskDriver for FakeTask {
    fn argv(&self, task: &str, args: &[String]) -> Vec<String> {
        let mut out = vec!["task".to_string(), task.to_string()];
        out.extend(args.iter().cloned());
        out
    }

    /// Pretends only `/code/jobs` has a Taskfile.
    fn taskfile(&self, dir: &Path) -> Option<PathBuf> {
        (dir == Path::new("/code/jobs")).then(|| PathBuf::from("/code/jobs/Taskfile.yml"))
    }
}

#[derive(Default)]
pub struct FakeProbe {
    busy: Mutex<HashMap<u16, PortHolder>>,
}

impl FakeProbe {
    pub fn set_busy(&self, port: u16, holder: PortHolder) {
        lock(&self.busy).insert(port, holder);
    }

    pub fn clear_busy(&self, port: u16) {
        lock(&self.busy).remove(&port);
    }
}

impl PortProbe for FakeProbe {
    fn free(&self, port: u16) -> bool {
        !lock(&self.busy).contains_key(&port)
    }

    fn holder(&self, port: u16) -> ports::Result<Option<PortHolder>> {
        Ok(lock(&self.busy).get(&port).cloned())
    }
}

#[derive(Default)]
pub struct FakeHealth {
    failing: Mutex<HashSet<u16>>,
    checks: Mutex<Vec<HealthCheck>>,
}

impl FakeHealth {
    pub fn fail_port(&self, port: u16) {
        lock(&self.failing).insert(port);
    }

    pub fn checks(&self) -> Vec<HealthCheck> {
        lock(&self.checks).clone()
    }
}

#[async_trait]
impl HealthProbe for FakeHealth {
    async fn check(&self, c: &HealthCheck) -> ports::Result<()> {
        lock(&self.checks).push(c.clone());
        if lock(&self.failing).contains(&c.port) {
            return Err(Error::msg("connection refused"));
        }
        Ok(())
    }
}

// --- bus, logs, clock -----------------------------------------------------------------

#[derive(Default)]
pub struct RecBus {
    events: Mutex<Vec<Event>>,
}

impl RecBus {
    pub fn events(&self) -> Vec<Event> {
        lock(&self.events).clone()
    }
}

impl EventBus for RecBus {
    fn publish(&self, event: Event) {
        lock(&self.events).push(event);
    }

    fn subscribe(&self, _buffer: usize) -> Subscription {
        let (_tx, rx) = mpsc::channel(1);
        Subscription::new(rx, || {})
    }
}

/// Log files in a directory, one per `project-service` (Go: `fileLogs`).
pub struct FileLogs {
    pub dir: PathBuf,
}

impl FileLogs {
    fn path(&self, project: &str, service: &str) -> PathBuf {
        self.dir.join(format!("{project}-{service}.log"))
    }

    fn tail_of(&self, project: &str, service: &str, n: usize) -> ports::Result<Vec<String>> {
        let data = std::fs::read_to_string(self.path(project, service))?;
        let lines: Vec<String> = data
            .trim_end_matches('\n')
            .split('\n')
            .map(str::to_string)
            .collect();
        let skip = lines.len().saturating_sub(n);
        Ok(lines[skip..].to_vec())
    }

    fn open_of(&self, project: &str, service: &str) -> ports::Result<(File, String)> {
        let path = self.path(project, service);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok((file, path.to_string_lossy().into_owned()))
    }
}

impl LogSink for FileLogs {
    fn open(&self, project: &str, service: &str) -> ports::Result<(File, String)> {
        self.open_of(project, service)
    }

    fn tail(&self, project: &str, service: &str, n: usize) -> ports::Result<Vec<String>> {
        self.tail_of(project, service, n)
    }

    fn open_job(&self, project: &str, job_id: &str) -> ports::Result<(File, String)> {
        self.open_of(project, &format!("job-{job_id}"))
    }

    fn tail_job(&self, project: &str, job_id: &str, n: usize) -> ports::Result<Vec<String>> {
        self.tail_of(project, &format!("job-{job_id}"), n)
    }

    fn close_job(&self, _project: &str, _job_id: &str) {}

    fn remove_job(&self, project: &str, job_id: &str) -> ports::Result<()> {
        Ok(std::fs::remove_file(
            self.path(project, &format!("job-{job_id}")),
        )?)
    }
}

/// A settable clock.
pub struct TestClock {
    now: Mutex<OffsetDateTime>,
}

impl TestClock {
    pub fn new(now: OffsetDateTime) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    pub fn now(&self) -> OffsetDateTime {
        *lock(&self.now)
    }

    pub fn advance(&self, d: Duration) {
        let mut now = lock(&self.now);
        *now += d;
    }
}
