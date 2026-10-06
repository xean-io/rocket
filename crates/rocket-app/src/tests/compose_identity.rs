//! Ports of the non-job cases of `compose_identity_test.go`. The two job
//! tests (`TestJobsForceComposeProjectIdentityAndPersistEnvironment`,
//! `TestJobsRejectInvalidEnvironmentBeforePersistence`) live in `jobs.rs`.

use crate::testing::*;
use rocket_domain::{
    Environment, PortEnvBinding, PortSpec, Project, RunState, Service, ServiceKind,
};
use std::collections::BTreeMap;

#[tokio::test]
async fn services_force_compose_project_identity() {
    for kind in [ServiceKind::Run, ServiceKind::Task] {
        for env_name in ["dev", "smoke"] {
            let name = format!("{kind}/{env_name}");
            let h = Harness::new();
            let service = Service {
                name: "api".into(),
                kind,
                run: "serve".into(),
                task: "api:dev".into(),
                env: BTreeMap::from([("COMPOSE_PROJECT_NAME".into(), "service-override".into())]),
                ports: vec![PortSpec {
                    name: "http".into(),
                    default: 8080,
                    env: "COMPOSE_PROJECT_NAME".into(),
                    env_bindings: BTreeMap::from([(
                        "COMPOSE_PROJECT_NAME".into(),
                        PortEnvBinding {
                            template: "port-override-{port}".into(),
                            default: false,
                        },
                    )]),
                    probe: None,
                }],
                ..Service::default()
            };
            let p = Project {
                name: "identity".into(),
                root: "/code/identity".into(),
                default_env: "dev".into(),
                envs: ["dev", "smoke"]
                    .into_iter()
                    .map(|e| {
                        (
                            e.to_string(),
                            Environment {
                                name: e.into(),
                                ..Environment::default()
                            },
                        )
                    })
                    .collect(),
                services: BTreeMap::from([("api".into(), service)]),
                ..Project::default()
            };
            h.loader.set(p.clone());
            h.set_base_env(&[
                "COMPOSE_PROJECT_NAME=inherited-override",
                "ROCKET_OWNER=agent:leak",
            ]);
            h.env.set(&[("COMPOSE_PROJECT_NAME", "dotenv-override")]);
            let mut request = req(&p.root, &[]);
            request.env = env_name.into();
            request.owner = "agent:job-owner".into();
            let res = h.up(request).await;
            assert!(!res.failed(), "{name}: up failed: {res:?}");
            let env = env_map(&h.runner.spec_for(&p.root).env);
            assert_eq!(
                env["COMPOSE_PROJECT_NAME"],
                format!("rocket-identity-{env_name}"),
                "{name}"
            );
            assert!(
                !env.contains_key("ROCKET_OWNER"),
                "{name}: Rocket owner leaked into child environment"
            );
            assert_eq!(h.run("identity", "api").state, RunState::Running, "{name}");
        }
    }
}

#[tokio::test]
async fn services_force_compose_project_identity_across_override_layers() {
    for source in ["unset", "inherited", "dotenv", "service"] {
        let h = Harness::new();
        let mut p = jobs_project();
        let mut base = vec!["ROCKET_OWNER=agent:leak"];
        let mut dotenv: Vec<(&str, &str)> = Vec::new();
        match source {
            "inherited" => base.push("COMPOSE_PROJECT_NAME=foreign"),
            "dotenv" => dotenv.push(("COMPOSE_PROJECT_NAME", "foreign")),
            "service" => {
                p.services.get_mut("api").unwrap().env =
                    BTreeMap::from([("COMPOSE_PROJECT_NAME".into(), "foreign".into())]);
            }
            _ => {}
        }
        h.loader.set(p.clone());
        h.set_base_env(&base);
        h.env.set(&dotenv);
        h.up(req(&p.root, &[])).await;
        let got = env_map(&h.runner.spec_for(&p.root).env)["COMPOSE_PROJECT_NAME"].clone();
        assert_eq!(got, "rocket-jobs-dev", "{source} compose identity");
    }
}
