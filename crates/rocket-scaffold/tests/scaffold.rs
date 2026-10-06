//! Ports of Go's `scaffold` tests, plus golden comparisons against
//! output captured from the Go implementation (`rocket init --print [--json]`).
//! Goldens live in `tests/golden` and `tests/cases`; regenerate them with the
//! Go binary only.

use rocket_scaffold::{Detected, Port, SetupStep, detect, parse_short_port};
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_tree(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// Copies `src` to `<tmp>/<name>` so the project name is derived from `name`.
fn fixture_copy(src: &Path, name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join(name);
    copy_tree(src, &dst);
    (tmp, dst)
}

fn autodropshipping() -> (tempfile::TempDir, PathBuf) {
    let src = manifest_dir().join("../../testdata/init/autodropshipping");
    fixture_copy(&src, "autodropshipping")
}

#[test]
fn parse_port_mapping() {
    // (input, ok, default, env, target)
    let cases: [(&str, bool, i64, &str, i64); 9] = [
        ("${PORT:-3000}:3000", true, 3000, "PORT", 3000),
        ("127.0.0.1:${PG:-5435}:5432", true, 5435, "PG", 5432),
        ("${WEB-8081}:80/tcp", true, 8081, "WEB", 80),
        ("${API_PORT}:8080", true, 8080, "API_PORT", 8080),
        ("$API_PORT:8080", true, 8080, "API_PORT", 8080),
        ("6379:6379", true, 6379, "", 6379),
        ("8080", false, 0, "", 0), // container-only port is not published
        ("3000-3005:3000-3005", false, 0, "", 0), // ranges are skipped
        ("127.0.0.1::5432", false, 0, "", 0), // ephemeral host port
    ];
    for (input, ok, default, env, target) in cases {
        let got = parse_short_port(input);
        assert_eq!(got.is_some(), ok, "{input}");
        if let Some(p) = got {
            assert_eq!(
                (p.default, p.env.as_str(), p.target),
                (default, env, target),
                "{input}"
            );
        }
    }
    // Edge cases of the Go parser.
    assert!(parse_short_port("${PORT:-abc}:80").is_none());
    assert_eq!(parse_short_port("${PORT:-}:80").unwrap().default, 80);
    assert!(parse_short_port("0:80").is_none());
    assert!(parse_short_port("${A}:x").is_none());
    assert_eq!(parse_short_port(" 81:80 ").unwrap().default, 81);
}

#[test]
fn detect_autodropshipping_shape() {
    let (_tmp, root) = autodropshipping();
    let r = detect(&root).unwrap();
    assert_eq!(r.name, "autodropshipping");
    assert_eq!(r.dotenv, [".env"]);
    assert_eq!(r.taskfile, "Taskfile.yml");
    assert_eq!(r.envs.len(), 2);
    assert_eq!(
        (r.envs[0].name.as_str(), r.envs[0].files[0].as_str()),
        ("dev", "compose.yaml")
    );
    assert_eq!(
        (r.envs[1].name.as_str(), r.envs[1].files[0].as_str()),
        ("smoke", "docker-compose.yaml")
    );
    let svc = |n: &str| {
        r.services
            .iter()
            .find(|s| s.name == n)
            .unwrap_or_else(|| panic!("no service {n}"))
    };
    let pg = svc("postgres");
    assert_eq!(pg.compose, "postgres");
    assert_eq!(pg.profiles, ["deps"]);
    assert_eq!(
        pg.ports,
        [Port {
            name: "main".into(),
            default: 5432,
            env: "POSTGRES_PORT".into(),
            target: 5432
        }]
    );
    let rd = svc("redis");
    assert!(rd.ports.len() == 1 && rd.ports[0].env.is_empty() && rd.ports[0].default == 6379);
    let mn = svc("minio");
    assert!(
        mn.ports.len() == 2
            && mn.ports[0].env == "MINIO_PORT"
            && mn.ports[1].env == "MINIO_CONSOLE_PORT"
    );
    let w = svc("worker");
    assert!(w.compose == "worker" && w.ports.is_empty());
    assert!(svc("dev").task == "dev" && svc("cp-dev").task == "cp:dev");
    assert!(
        r.services.iter().all(|s| s.name != "control-plane"),
        "prod-only compose service must not become a dev service"
    );
    assert_eq!(
        r.pipelines,
        ["cp:test", "infra:down", "infra:up", "lint", "test"]
    );
    assert_eq!(r.deploy_tasks, ["deploy:prod", "release:ci"]);
    assert_eq!(
        r.setup,
        [
            SetupStep {
                name: "install".into(),
                task: "install".into()
            },
            SetupStep {
                name: "migrate".into(),
                task: "db:migrate".into()
            },
        ]
    );
    let names: Vec<&str> = r.tasks.iter().map(|t| t.name.as_str()).collect();
    for hidden in ["_helper", "default"] {
        assert!(
            !names.contains(&hidden),
            "tasks list contains {hidden}: {names:?}"
        );
    }
    assert!(
        names.contains(&"cp:dev") && names.contains(&"infra:up"),
        "{names:?}"
    );
}

#[test]
fn render_is_a_valid_manifest() {
    let (_tmp, root) = autodropshipping();
    let r = detect(&root).unwrap();
    let out = r.render();
    let p = rocket_manifest::parse(out.as_bytes(), &root.to_string_lossy())
        .unwrap_or_else(|e| panic!("generated rocket.yaml is invalid: {e}\n{out}"));
    assert_eq!(
        p.services["postgres"].kind,
        rocket_domain::ServiceKind::Compose
    );
    assert_eq!(p.services["cp-dev"].task, "cp:dev");
    assert_eq!(p.pipelines["cp:test"][0].task, "cp:test");
    assert_eq!(p.setup["migrate"].task, "db:migrate");
    assert_eq!(p.envs["smoke"].compose, ["docker-compose.yaml"]);
    for want in [
        "#   - deploy:prod",
        "#   - cp:dev \u{2014} Control plane dev",
        "control-plane",
        "xean-spectrai",
    ] {
        assert!(out.contains(want), "output lacks {want:?}:\n{out}");
    }
    assert!(
        !p.pipelines.contains_key("deploy:prod"),
        "deploy task must not become a pipeline"
    );
}

#[test]
fn render_without_detections_is_still_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Empty Project");
    std::fs::create_dir_all(&root).unwrap();
    let r = detect(&root).unwrap();
    assert_eq!(r.name, "empty-project");
    let out = r.render();
    rocket_manifest::parse(out.as_bytes(), &root.to_string_lossy())
        .unwrap_or_else(|e| panic!("placeholder manifest invalid: {e}\n{out}"));
}

#[test]
fn detect_setup_install_task_precedence() {
    struct Case {
        name: &'static str,
        tasks: &'static str,
        want_task: &'static str,
    }
    let cases = [
        Case {
            name: "setup fallback",
            tasks: "  setup: echo setup\n",
            want_task: "setup",
        },
        Case {
            name: "explicit install",
            tasks: "  install: echo install\n",
            want_task: "install",
        },
        Case {
            name: "setup before install",
            tasks: "  setup: echo setup\n  install: echo install\n",
            want_task: "install",
        },
        Case {
            name: "install before setup",
            tasks: "  install: echo install\n  setup: echo setup\n",
            want_task: "install",
        },
        Case {
            name: "namespaced only",
            tasks: "",
            want_task: "",
        },
        Case {
            name: "internal setup",
            tasks: "  setup: {internal: true, cmds: [echo setup]}\n",
            want_task: "",
        },
    ];
    for c in cases {
        let root = tempfile::tempdir().unwrap();
        let root_taskfile = format!(
            "version: '3'\nincludes: {{infra: ./infra.yml}}\ntasks:\n  dev: echo dev\n{}",
            c.tasks
        );
        std::fs::write(root.path().join("Taskfile.yml"), root_taskfile).unwrap();
        std::fs::write(
            root.path().join("infra.yml"),
            "version: '3'\ntasks:\n  setup: echo included setup\n  install: echo included install\n",
        )
        .unwrap();
        let r = detect(root.path()).unwrap();
        let task = r
            .setup
            .iter()
            .find(|s| s.name == "install")
            .map_or("", |s| s.task.as_str());
        assert_eq!(task, c.want_task, "{}", c.name);
        assert!(
            !r.pipelines.contains(&"setup".to_owned()),
            "{}: root setup exposed: {:?}",
            c.name,
            r.pipelines
        );
        assert_eq!(r.pipelines, ["infra:install", "infra:setup"], "{}", c.name);
        let out = r.render();
        let p = rocket_manifest::parse(out.as_bytes(), &root.path().to_string_lossy())
            .unwrap_or_else(|e| panic!("{}: rendered manifest invalid: {e}\n{out}", c.name));
        let got = p.setup.get("install").map_or("", |s| s.task.as_str());
        assert_eq!(got, c.want_task, "{}: rendered setup.install", c.name);
        assert!(
            !p.pipelines.contains_key("setup"),
            "{}: rendered root setup pipeline",
            c.name
        );
    }
}

#[test]
fn malformed_inputs_are_reported() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("compose.yaml"), "services: [a, b]\n").unwrap();
    let err = detect(root.path()).unwrap_err().to_string();
    assert!(err.starts_with("compose.yaml: "), "{err}");

    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Taskfile.yml"), "tasks: [\n").unwrap();
    let err = detect(root.path()).unwrap_err().to_string();
    assert!(err.starts_with("Taskfile.yml: "), "{err}");

    // An unreadable include is ignored, an empty compose file is fine.
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("compose.yaml"), "").unwrap();
    std::fs::write(
        root.path().join("Taskfile.yml"),
        "includes: {x: ./missing.yml}\ntasks: {build: echo}\n",
    )
    .unwrap();
    let r = detect(root.path()).unwrap();
    assert_eq!(r.pipelines, ["build"]);
    assert!(r.services.is_empty());
}

// ---- goldens captured from the Go implementation ---------------------------

fn check_golden(detected: &mut Detected, name: &str, golden_yaml: &Path, golden_json: &Path) {
    let want_yaml = std::fs::read_to_string(golden_yaml).unwrap();
    assert_eq!(
        detected.render(),
        want_yaml,
        "{name}: rendered rocket.yaml differs from Go"
    );
    detected.root = "<ROOT>".into();
    let want_json = std::fs::read_to_string(golden_json).unwrap();
    let got_json = serde_json::to_string_pretty(&detected).unwrap();
    assert_eq!(
        got_json,
        want_json.trim_end(),
        "{name}: detected JSON differs from Go"
    );
}

#[test]
fn golden_autodropshipping() {
    let (_tmp, root) = autodropshipping();
    let mut r = detect(&root).unwrap();
    let g = manifest_dir().join("tests/golden");
    check_golden(
        &mut r,
        "autodropshipping",
        &g.join("autodropshipping.yaml"),
        &g.join("autodropshipping.detected.json"),
    );
}

#[test]
fn golden_empty_project() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Empty Project");
    std::fs::create_dir_all(&root).unwrap();
    let mut r = detect(&root).unwrap();
    let g = manifest_dir().join("tests/golden");
    check_golden(
        &mut r,
        "empty-project",
        &g.join("empty-project.yaml"),
        &g.join("empty-project.detected.json"),
    );
}

#[test]
fn golden_cases() {
    let cases = manifest_dir().join("tests/cases");
    for name in [
        "only-suffixed",
        "anchors-long-ports",
        "nested-tasks",
        "Weird_Name.v2",
    ] {
        let (_tmp, root) = fixture_copy(&cases.join(name), name);
        let mut r = detect(&root).unwrap();
        check_golden(
            &mut r,
            name,
            &cases.join(format!("{name}.yaml")),
            &cases.join(format!("{name}.detected.json")),
        );
    }
}
