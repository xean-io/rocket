//! Project fixtures shared by the tests (Go: `nuvara()`, `otherProject()`,
//! `jobsProject()`).

use rocket_domain::{
    Deploy, Environment, HealthSpec, PortSpec, Project, Service, ServiceKind, Step,
};
use std::collections::BTreeMap;

fn port(name: &str, default: u16, env: &str) -> PortSpec {
    PortSpec {
        name: name.into(),
        default,
        env: env.into(),
        ..PortSpec::default()
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

pub fn nuvara() -> Project {
    let envs = BTreeMap::from([(
        "dev".to_string(),
        Environment {
            name: "dev".into(),
            compose: strings(&["docker-compose.yml"]),
            ..Environment::default()
        },
    )]);
    let services = [
        Service {
            name: "postgres".into(),
            kind: ServiceKind::Compose,
            compose: "postgres".into(),
            profiles: strings(&["deps"]),
            ports: vec![port("main", 5435, "POSTGRES_PORT")],
            ..Service::default()
        },
        Service {
            name: "api".into(),
            kind: ServiceKind::Run,
            run: "bun run dev".into(),
            cwd: "apps/api".into(),
            depends_on: strings(&["postgres"]),
            ports: vec![port("http", 3002, "PORT")],
            health: Some(HealthSpec {
                http: "/health".into(),
                ..HealthSpec::default()
            }),
            ..Service::default()
        },
        Service {
            name: "web".into(),
            kind: ServiceKind::Run,
            run: "bun run dev".into(),
            cwd: "apps/web".into(),
            depends_on: strings(&["api"]),
            ports: vec![port("http", 3000, "PORT")],
            ..Service::default()
        },
        Service {
            name: "causation".into(),
            kind: ServiceKind::Task,
            task: "dev:causation".into(),
            ports: vec![port("http", 3003, "")],
            ..Service::default()
        },
        Service {
            name: "worker".into(),
            kind: ServiceKind::Run,
            run: "sleep 1000".into(),
            depends_on: strings(&["causation"]),
            ..Service::default()
        },
    ];
    Project {
        name: "nuvara".into(),
        root: "/code/nuvara".into(),
        default_env: "dev".into(),
        dotenv: strings(&[".env"]),
        envs,
        services: services.into_iter().map(|s| (s.name.clone(), s)).collect(),
        groups: BTreeMap::from([
            ("core".to_string(), strings(&["api", "web"])),
            ("all".to_string(), strings(&["*"])),
        ]),
        ..Project::default()
    }
}

pub fn other_project() -> Project {
    Project {
        name: "shop".into(),
        root: "/code/shop".into(),
        default_env: "dev".into(),
        envs: BTreeMap::from([(
            "dev".to_string(),
            Environment {
                name: "dev".into(),
                ..Environment::default()
            },
        )]),
        services: BTreeMap::from([(
            "control-plane".to_string(),
            Service {
                name: "control-plane".into(),
                kind: ServiceKind::Run,
                run: "pnpm dev".into(),
                ports: vec![port("http", 3000, "PORT")],
                ..Service::default()
            },
        )]),
        ..Project::default()
    }
}

fn env_named(name: &str, deploy: Option<Deploy>) -> (String, Environment) {
    (
        name.to_string(),
        Environment {
            name: name.into(),
            deploy,
            ..Environment::default()
        },
    )
}

fn run_step(run: &str) -> Step {
    Step {
        run: run.into(),
        ..Step::default()
    }
}

fn task_step(task: &str) -> Step {
    Step {
        task: task.into(),
        ..Step::default()
    }
}

pub fn jobs_project() -> Project {
    Project {
        name: "jobs".into(),
        root: "/code/jobs".into(),
        default_env: "dev".into(),
        dotenv: strings(&[".env"]),
        envs: BTreeMap::from([
            env_named("dev", None),
            env_named(
                "stage",
                Some(Deploy {
                    run: "deploy-stage".into(),
                    ..Deploy::default()
                }),
            ),
            env_named(
                "prod",
                Some(Deploy {
                    task: "release:prod".into(),
                    confirm: true,
                    ..Deploy::default()
                }),
            ),
        ]),
        setup: BTreeMap::from([
            ("doctor".to_string(), run_step("doctor-ok")),
            ("migrate".to_string(), task_step("db:migrate")),
            ("seed".to_string(), run_step("seed-it")),
        ]),
        services: BTreeMap::from([(
            "api".to_string(),
            Service {
                name: "api".into(),
                kind: ServiceKind::Run,
                run: "api-serve".into(),
                ports: vec![port("http", 4000, "PORT")],
                ..Service::default()
            },
        )]),
        pipelines: BTreeMap::from([
            (
                "ci".to_string(),
                vec![
                    run_step("step-lint"),
                    run_step("step-unit"),
                    task_step("e2e"),
                ],
            ),
            (
                "broken".to_string(),
                vec![
                    run_step("step-ok"),
                    run_step("step-boom"),
                    run_step("step-never"),
                ],
            ),
            ("slow".to_string(), vec![run_step("step-slow")]),
        ]),
        ..Project::default()
    }
}
