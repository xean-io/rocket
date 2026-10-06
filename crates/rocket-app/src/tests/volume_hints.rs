//! Ports of `volume_hints_test.go`.

use crate::testing::*;
use crate::volume_hints::compose_volume_hints;
use rocket_domain::ports::{self, ComposeDriver, ComposeTarget, Error, async_trait};
use rocket_domain::{Environment, Project, Service, ServiceKind, Step};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Wraps [`FakeCompose`] with scripted volume evidence (Go: `hintCompose`).
struct HintCompose {
    inner: Arc<FakeCompose>,
    volumes: Vec<String>,
    exists: Mutex<HashMap<String, bool>>,
    created: Vec<String>,
    config_err: bool,
    inspect_err_at: usize,
    inspections: Mutex<usize>,
    up_err: bool,
    targets: Mutex<Vec<ComposeTarget>>,
}

impl HintCompose {
    fn new(inner: Arc<FakeCompose>, volumes: &[&str]) -> Self {
        Self {
            inner,
            volumes: volumes.iter().map(|v| (*v).to_string()).collect(),
            exists: Mutex::new(HashMap::new()),
            created: Vec::new(),
            config_err: false,
            inspect_err_at: 0,
            inspections: Mutex::new(0),
            up_err: false,
            targets: Mutex::new(Vec::new()),
        }
    }

    fn targets(&self) -> Vec<ComposeTarget> {
        self.targets.lock().unwrap().clone()
    }
}

#[async_trait]
impl ComposeDriver for HintCompose {
    async fn named_volumes(&self, t: &ComposeTarget, _service: &str) -> ports::Result<Vec<String>> {
        self.targets.lock().unwrap().push(t.clone());
        if self.config_err {
            return Err(Error::msg("invalid config"));
        }
        Ok(self.volumes.clone())
    }

    async fn volume_exists(&self, _t: &ComposeTarget, name: &str) -> ports::Result<bool> {
        let mut n = self.inspections.lock().unwrap();
        *n += 1;
        if *n == self.inspect_err_at {
            return Err(Error::msg("volume listing unavailable"));
        }
        Ok(self
            .exists
            .lock()
            .unwrap()
            .get(name)
            .copied()
            .unwrap_or(false))
    }

    async fn up(
        &self,
        t: &ComposeTarget,
        service: &str,
        out: &mut (dyn Write + Send),
    ) -> ports::Result<String> {
        for volume in &self.created {
            self.exists.lock().unwrap().insert(volume.clone(), true);
        }
        if self.up_err {
            return Err(Error::msg("startup failed"));
        }
        self.inner.up(t, service, out).await
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

fn step_run(run: &str) -> Step {
    Step {
        run: run.into(),
        ..Step::default()
    }
}

fn step_task(task: &str) -> Step {
    Step {
        task: task.into(),
        ..Step::default()
    }
}

fn setup(items: &[(&str, Step)]) -> BTreeMap<String, Step> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

struct Case {
    name: &'static str,
    setup: BTreeMap<String, Step>,
    exists: bool,
    created: bool,
    config_err: bool,
    inspect_err_at: usize,
    up_err: bool,
    want: &'static [&'static str],
}

#[tokio::test]
async fn up_new_compose_volume_hints() {
    let migrate = || setup(&[("migrate", step_task("migrate"))]);
    let case = |name: &'static str,
                setup: BTreeMap<String, Step>,
                exists: bool,
                created: bool,
                config_err: bool,
                inspect_err_at: usize,
                up_err: bool,
                want: &'static [&'static str]| Case {
        name,
        setup,
        exists,
        created,
        config_err,
        inspect_err_at,
        up_err,
        want,
    };
    let cases = [
        case(
            "fresh both actions",
            setup(&[
                ("migrate", step_task("db:migrate")),
                ("assets", step_task("infra:assets")),
            ]),
            false,
            true,
            false,
            0,
            false,
            &["rocket migrate", "rocket setup assets"],
        ),
        case(
            "fresh migration only",
            setup(&[("migrate", step_run("migrate"))]),
            false,
            true,
            false,
            0,
            false,
            &["rocket migrate"],
        ),
        case(
            "fresh assets only",
            setup(&[("assets", step_task("infra:assets"))]),
            false,
            true,
            false,
            0,
            false,
            &["rocket setup assets"],
        ),
        case(
            "fresh no configured actions",
            BTreeMap::new(),
            false,
            true,
            false,
            0,
            false,
            &[],
        ),
        case(
            "existing volume",
            migrate(),
            true,
            true,
            false,
            0,
            false,
            &[],
        ),
        case("config error", migrate(), false, true, true, 0, false, &[]),
        case(
            "before inspect error",
            migrate(),
            false,
            true,
            false,
            1,
            false,
            &[],
        ),
        case(
            "after inspect error",
            migrate(),
            false,
            true,
            false,
            2,
            false,
            &[],
        ),
        case("not created", migrate(), false, false, false, 0, false, &[]),
        case(
            "failed startup without creation",
            migrate(),
            false,
            false,
            false,
            0,
            true,
            &[],
        ),
        case(
            "failed startup with after evidence",
            migrate(),
            false,
            true,
            false,
            0,
            true,
            &["rocket migrate"],
        ),
    ];
    for tc in cases {
        let mut h = Harness::new();
        let p = Project {
            name: "volumes".into(),
            root: "/code/volumes".into(),
            default_env: "dev".into(),
            setup: tc.setup.clone(),
            envs: BTreeMap::from([(
                "dev".to_string(),
                Environment {
                    name: "dev".into(),
                    compose: vec!["compose.yaml".into()],
                    profiles: vec!["infra".into()],
                    ..Environment::default()
                },
            )]),
            services: BTreeMap::from([(
                "db".to_string(),
                Service {
                    name: "db".into(),
                    kind: ServiceKind::Compose,
                    compose: "database".into(),
                    profiles: vec!["data".into()],
                    ..Service::default()
                },
            )]),
            ..Project::default()
        };
        h.loader.set(p.clone());
        let mut c = HintCompose::new(
            h.compose.clone(),
            &["rocket-volumes-dev_data", "rocket-volumes-dev_data"],
        );
        c.exists = Mutex::new(HashMap::from([(
            "rocket-volumes-dev_data".to_string(),
            tc.exists,
        )]));
        c.inspect_err_at = tc.inspect_err_at;
        c.config_err = tc.config_err;
        c.up_err = tc.up_err;
        if tc.created {
            c.created = vec!["rocket-volumes-dev_data".into()];
        }
        let c = Arc::new(c);
        h.reconfigure(|d| d.compose = c.clone());
        let mut request = req(&p.root, &["db"]);
        request.profiles = vec!["requested".into()];
        let res = h.up(request).await;
        assert_eq!(
            res.failed(),
            tc.up_err,
            "{}: volume evidence changed startup result: {res:?}",
            tc.name
        );
        let hints = &res.hints;
        if tc.want.is_empty() {
            assert!(
                hints.is_empty(),
                "{}: unsupported new-volume claim: {hints:?}",
                tc.name
            );
        } else {
            assert!(
                hints.len() == 1 && hints[0].matches("rocket-volumes-dev_data").count() == 1,
                "{}: new volume hint must be deduplicated: {hints:?}",
                tc.name
            );
            for action in tc.want {
                assert!(
                    hints[0].contains(action),
                    "{}: hint {:?} missing {action:?}",
                    tc.name,
                    hints[0]
                );
            }
        }
        let targets = c.targets();
        assert!(
            targets.len() == 1
                && targets[0].project_name == "rocket-volumes-dev"
                && targets[0].profiles == ["data", "infra", "requested"]
                && env_map(&targets[0].env)["COMPOSE_PROJECT_NAME"] == "rocket-volumes-dev",
            "{}: volume config target must match launch identity/environment: {targets:?}",
            tc.name
        );
    }
}

#[tokio::test]
async fn up_volume_hints_require_actual_new_startup() {
    let mut h = Harness::new();
    let mut p = nuvara();
    p.setup = setup(&[("migrate", step_task("migrate"))]);
    h.loader.set(p);
    let mut c = HintCompose::new(h.compose.clone(), &["rocket-nuvara-dev_data"]);
    c.created = vec!["rocket-nuvara-dev_data".into()];
    let c = Arc::new(c);
    h.reconfigure(|d| d.compose = c.clone());
    let first = h.up(nuvara_req(&["postgres"])).await;
    assert_eq!(
        first.hints.len(),
        1,
        "first startup hint missing: {first:?}"
    );
    let second = h.up(nuvara_req(&["postgres"])).await;
    assert!(
        second.hints.is_empty() && c.targets().len() == 1,
        "idempotent up repeated volume hint: {second:?}; config calls={}",
        c.targets().len()
    );
}

#[tokio::test]
async fn up_volume_hints_sort_and_deduplicate_across_services() {
    let mut h = Harness::new();
    let mut p = nuvara();
    p.setup = setup(&[("migrate", step_task("migrate"))]);
    p.services = ["one", "two"]
        .into_iter()
        .map(|n| {
            (
                n.to_string(),
                Service {
                    name: n.into(),
                    kind: ServiceKind::Compose,
                    compose: n.into(),
                    ..Service::default()
                },
            )
        })
        .collect();
    h.loader.set(p);
    let mut c = HintCompose::new(
        h.compose.clone(),
        &["rocket-fixture_z", "rocket-fixture_a", "rocket-fixture_z"],
    );
    c.created = vec!["rocket-fixture_z".into(), "rocket-fixture_a".into()];
    let c = Arc::new(c);
    h.reconfigure(|d| d.compose = c.clone());
    let res = h.up(nuvara_req(&["one", "two"])).await;
    let hints = &res.hints;
    assert!(
        hints.len() == 1
            && hints[0].contains("rocket-fixture_a, rocket-fixture_z")
            && hints[0].matches("rocket-fixture_z").count() == 1,
        "unstable/duplicate project hint: {hints:?}"
    );
}

#[test]
fn volume_hint_targets_project_and_selected_environment() {
    let p = Project {
        name: "fixture".into(),
        default_env: "dev".into(),
        setup: setup(&[
            ("migrate", step_run("migrate")),
            ("assets", step_task("assets")),
        ]),
        ..Project::default()
    };
    for env in ["dev", "smoke", "stage 'name"] {
        let hints = compose_volume_hints(&p, env, &BTreeSet::from(["volume".to_string()]));
        let want = match env {
            "dev" => [
                "rocket migrate -p fixture",
                "rocket setup assets -p fixture",
            ]
            .map(String::from),
            "smoke" => [
                "rocket migrate -p fixture --env smoke",
                "rocket setup assets -p fixture --env smoke",
            ]
            .map(String::from),
            _ => [
                r#"rocket migrate -p fixture --env 'stage '"'"'name'"#,
                r#"rocket setup assets -p fixture --env 'stage '"'"'name'"#,
            ]
            .map(String::from),
        };
        for command in &want {
            assert!(
                hints.len() == 1 && hints[0].contains(command.as_str()),
                "{env}: hint {hints:?} missing usable command {command:?}"
            );
        }
        if env == "dev" {
            assert!(
                !hints[0].contains("--env"),
                "default environment unnecessarily specified: {hints:?}"
            );
        }
    }
}
