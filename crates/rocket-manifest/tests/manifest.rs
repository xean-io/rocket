//! Ports of the Go tests in `internal/manifest/*_test.go`, plus filesystem
//! behavior (discovery, loading, dotenv files).

use rocket_domain::ports::{EnvSource, ManifestLoader};
use rocket_domain::{PortSpec, Project, ServiceKind, health_check_for};
use rocket_manifest::{
    EnvFiles, Error, FILE_NAME, Loader, find, load, parse, parse_dotenv, schema, schema_value,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn fixture(name: &str) -> Result<Project, Error> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/manifests")
        .join(name);
    let data = fs::read(&path).expect("fixture exists");
    parse(
        &data,
        &format!("/projects/{}", name.trim_end_matches(".yaml")),
    )
}

fn err_text(yaml: &str) -> String {
    parse(yaml.as_bytes(), "/code/x")
        .expect_err("expected an error")
        .to_string()
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

// manifest_test.go -----------------------------------------------------------

#[test]
fn parse_nuvara() {
    let p = fixture("nuvara.yaml").unwrap();
    assert_eq!(
        (p.name.as_str(), p.root.as_str()),
        ("nuvara", "/projects/nuvara")
    );
    assert_eq!(p.default_env, "dev");
    let api = &p.services["api"];
    assert_eq!(api.kind, ServiceKind::Run);
    assert_eq!(api.cwd, "apps/api");
    assert_eq!(
        api.ports,
        vec![PortSpec {
            name: "http".into(),
            default: 3002,
            env: "PORT".into(),
            ..PortSpec::default()
        }]
    );
    let health = api.health.as_ref().unwrap();
    assert_eq!(health.http, "/api/health/live");
    assert_eq!(health.timeout, Duration::from_secs(90));
    assert_eq!(p.services["postgres"].kind, ServiceKind::Compose);
    assert_eq!(p.services["causation"].kind, ServiceKind::Task);
    assert_eq!(p.envs["smoke"].profiles, strs(&["release"]));
    let deploy = p.envs["prod"].deploy.as_ref().unwrap();
    assert_eq!(deploy.task, "release:ci");
    assert!(deploy.confirm);
    assert_eq!(p.pipelines["ci"].len(), 3);
    assert_eq!(p.setup["migrate"].task, "db:migrate");
}

#[test]
fn parse_autodropshipping() {
    let p = fixture("autodropshipping.yaml").unwrap();
    let cp = &p.services["control-plane"];
    assert_eq!(cp.env["NODE_ENV"], "development");
    assert_eq!(cp.ports[0].default, 3000);
    assert_eq!(
        p.envs["full"].compose,
        strs(&["compose.yaml", "docker-compose.yaml"])
    );
}

#[test]
fn validation_reports_all_problems() {
    let msg = fixture("invalid.yaml").unwrap_err().to_string();
    for want in [
        r#"service "a": exactly one of compose, task or run is required (got task, run)"#,
        r#"service "b": exactly one of compose, task or run is required (got none)"#,
        r#"service "a": depends_on references unknown service "ghost""#,
        r#"group "g": unknown service "missing""#,
        r#"group "a": name collides with a service"#,
        "port 70000 out of range",
        r#"invalid env var name "bad-name""#,
        r#"health.port "nope" is not a declared port"#,
        "dependency cycle",
    ] {
        assert!(msg.contains(want), "missing problem {want:?} in:\n{msg}");
    }
    assert!(msg.starts_with("invalid rocket.yaml:\n  - "));
}

#[test]
fn problems_follow_go_ordering() {
    // convert problems first (version, then per service), then validate
    // problems: name, services, default_env, services in name order, groups,
    // pipelines, setup.
    let msg = err_text(
        "version: 0\nname: Bad\ndefault_env: zz\nservices:\n  a: {run: x, depends_on: [a, nope]}\n\
         groups: {a: [nope]}\npipelines: {p: {needs: [nope], steps: [{}]}}\nsetup: {s: {}}\n",
    );
    let lines: Vec<&str> = msg.lines().skip(1).collect();
    assert_eq!(
        lines,
        [
            "  - version must be 1 (got 0)",
            "  - invalid project name \"Bad\" (use lowercase letters, digits, '.', '_' or '-')",
            "  - default_env \"zz\" is not a declared env",
            "  - service \"a\" depends on itself",
            "  - service \"a\": depends_on references unknown service \"nope\"",
            "  - group \"a\": name collides with a service",
            "  - group \"a\": unknown service \"nope\"",
            "  - pipeline \"p\" needs: unknown service or group \"nope\" in project Bad",
            "  - pipeline \"p\" step 1: exactly one of task or run is required",
            "  - setup \"s\": exactly one of task or run is required",
            "  - dependency cycle between services: a",
        ]
    );
}

#[test]
fn unknown_fields_are_rejected_with_line_numbers() {
    let err = fixture("unknown_field.yaml").unwrap_err().to_string();
    assert_eq!(
        err,
        "parse rocket.yaml: yaml: unmarshal errors:\n  line 4: field dependson not found in type manifest.serviceYAML"
    );
}

#[test]
fn validation_cases() {
    let cases: [(&str, &str, &str); 11] = [
        (
            "missing version",
            "name: x\nservices: {a: {run: x}}",
            "version must be 1",
        ),
        (
            "bad name",
            "version: 1\nname: 'Bad Name'\nservices: {a: {run: x}}",
            "invalid project name",
        ),
        (
            "self dependency",
            "version: 1\nname: x\nservices: {a: {run: x, depends_on: [a]}}",
            "depends on itself",
        ),
        (
            "unknown default env",
            "version: 1\nname: x\ndefault_env: qa\nenvs: {dev: {}}\nservices: {a: {run: x}}",
            r#"default_env "qa""#,
        ),
        (
            "health path",
            "version: 1\nname: x\nservices: {a: {run: x, ports: {h: {default: 1}}, health: {http: health}}}",
            "must start with /",
        ),
        (
            "health without ports",
            "version: 1\nname: x\nservices: {a: {run: x, health: {tcp: true}}}",
            "health requires at least one port",
        ),
        (
            "bad timeout",
            "version: 1\nname: x\nservices: {a: {run: x, ports: {h: {default: 1}}, health: {timeout: soon}}}",
            "invalid health.timeout",
        ),
        (
            "pipeline step",
            "version: 1\nname: x\nservices: {a: {run: x}}\npipelines: {t: [{}]}",
            r#"pipeline "t" step 1"#,
        ),
        ("no services", "version: 1\nname: x", "no services"),
        (
            "deploy without action",
            "version: 1\nname: x\nenvs: {dev: {}, prod: {deploy: {confirm: true}}}\nservices: {a: {run: x}}",
            r#"env "prod": deploy needs exactly one of task or run"#,
        ),
        (
            "deploy with both",
            "version: 1\nname: x\nenvs: {dev: {}, prod: {deploy: {task: a, run: b}}}\nservices: {a: {run: x}}",
            r#"env "prod": deploy needs exactly one"#,
        ),
    ];
    for (name, yaml, want) in cases {
        let err = err_text(yaml);
        assert!(err.contains(want), "{name}: {err:?} lacks {want:?}");
    }
}

#[test]
fn name_defaults_to_directory() {
    let p = parse(b"version: 1\nservices: {a: {run: x}}", "/code/my-app").unwrap();
    assert_eq!((p.name.as_str(), p.default_env.as_str()), ("my-app", "dev"));
    assert!(p.envs.contains_key("dev"));
    let p = parse(b"version: 1\nservices: {a: {run: x}}", "/code/My-App/").unwrap();
    assert_eq!(p.name, "my-app");
}

#[test]
fn schema_covers_manifest_fields() {
    let schema: Value = serde_json::from_str(&schema()).expect("schema is valid JSON");
    let props = schema["properties"].as_object().unwrap();
    for f in [
        "version",
        "name",
        "dotenv",
        "default_env",
        "setup",
        "envs",
        "services",
        "groups",
        "pipelines",
    ] {
        assert!(
            props.contains_key(f),
            "schema missing top-level property {f}"
        );
    }
    let svc = schema["$defs"]["service"]["properties"]
        .as_object()
        .unwrap();
    for f in [
        "compose",
        "task",
        "run",
        "cwd",
        "env",
        "dotenv",
        "profiles",
        "depends_on",
        "ports",
        "health",
    ] {
        assert!(svc.contains_key(f), "schema missing service property {f}");
    }
    assert_eq!(schema_value(), schema);
}

#[test]
fn parse_dotenv_cases() {
    let src =
        "# comment\nexport A=1\nB = \"two words\"\nC='x#y'\nD=val # trailing\n\nEMPTY=\nbad line\n";
    let want: BTreeMap<String, String> = [
        ("A", "1"),
        ("B", "two words"),
        ("C", "x#y"),
        ("D", "val"),
        ("EMPTY", ""),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(parse_dotenv(src.as_bytes()), want);
}

// pipeline_needs_test.go -----------------------------------------------------

#[test]
fn pipeline_needs_and_legacy_steps() {
    let p = parse(
        b"version: 1\nname: pipelines\nservices: {db: {run: database}, api: {run: api, depends_on: [db]}}\n\
          groups: {infra: [db]}\npipelines:\n  legacy: [{run: lint}]\n  check: {needs: [infra, api], steps: [{task: test}]}\n",
        "/code/pipelines",
    )
    .unwrap();
    assert_eq!(p.pipelines["legacy"].len(), 1);
    assert_eq!(p.pipelines["legacy"][0].run, "lint");
    assert_eq!(p.pipelines["check"][0].task, "test");
    assert_eq!(p.pipeline_needs["check"], strs(&["infra", "api"]));
    assert!(p.pipeline_needs["legacy"].is_empty());
}

#[test]
fn pipeline_needs_validation() {
    let cases = [
        (
            "unknown prerequisite",
            "{needs: [missing], steps: [{run: test}]}",
            "missing",
        ),
        (
            "unknown object key",
            "{needs: [api], steps: [{run: test}], needz: []}",
            "needz",
        ),
        (
            "unknown object step key",
            "{needs: [api], steps: [{run: test, typo: x}]}",
            "typo",
        ),
        ("unknown legacy step key", "[{run: test, typo: x}]", "typo"),
        ("missing steps", "{needs: [api]}", "steps"),
        ("null steps", "{needs: [api], steps: null}", "steps"),
        ("null needs", "{needs: null, steps: [{run: test}]}", "needs"),
        ("invalid step", "{needs: [api], steps: [{}]}", "exactly one"),
        ("wrong pipeline kind", "test", "array or object"),
    ];
    for (name, pipeline, want) in cases {
        let yaml = format!(
            "version: 1\nname: pipelines\nservices: {{api: {{run: api}}}}\npipelines: {{check: {pipeline}}}\n"
        );
        let err = err_text(&yaml);
        assert!(err.contains(want), "{name}: {err:?} lacks {want:?}");
    }
}

#[test]
fn pipeline_needs_schema() {
    let schema = schema_value();
    let choices = schema["properties"]["pipelines"]["additionalProperties"]["oneOf"]
        .as_array()
        .expect("pipeline accepts legacy array and object");
    assert_eq!(choices.len(), 2);
}

// port_refs_test.go ----------------------------------------------------------

#[test]
fn port_references_add_dependencies() {
    let p = parse(
        b"version: 1\nname: refs\nservices:\n\
          \x20 api: {run: api, ports: {http: {default: 8080, env: PORT}}}\n\
          \x20 commerce: {run: commerce, ports: {http: {default: 9000, env: PORT}}}\n\
          \x20 web:\n    task: web\n    depends_on: [commerce]\n    env:\n\
          \x20     API_URL: \"http://127.0.0.1:{api.http}/{api.http}\"\n\
          \x20     COMMERCE_URL: \"http://127.0.0.1:{commerce.http}\"\n",
        "/code/refs",
    )
    .unwrap();
    assert_eq!(
        p.services["web"].depends_on,
        strs(&["commerce", "api"]),
        "explicit entries are preserved and references are added once"
    );
    assert_eq!(
        p.start_order(&strs(&["web"])).unwrap(),
        strs(&["api", "commerce", "web"])
    );
}

#[test]
fn port_reference_validation() {
    let cases = [
        (
            "unknown service",
            "web: {run: web, env: {API_URL: \"http://localhost:{missing.http}\"}}",
            r#"unknown service "missing""#,
        ),
        (
            "unknown port",
            "api: {run: api, ports: {http: {default: 8080}}}\n  web: {run: web, env: {API_URL: \"http://localhost:{api.admin}\"}}",
            r#"unknown port "admin""#,
        ),
        (
            "self reference",
            "api: {run: api, ports: {http: {default: 8080}}, env: {URL: \"http://localhost:{api.http}\"}}",
            "depends on itself",
        ),
        (
            "reference cycle",
            "api: {run: api, ports: {http: {default: 8080}}, env: {URL: \"http://localhost:{web.http}\"}}\n  web: {run: web, ports: {http: {default: 3000}}, env: {URL: \"http://localhost:{api.http}\"}}",
            "dependency cycle",
        ),
        (
            "mixed explicit cycle",
            "api: {run: api, depends_on: [web], ports: {http: {default: 8080}}}\n  web: {run: web, env: {URL: \"http://localhost:{api.http}\"}}",
            "dependency cycle",
        ),
    ];
    for (name, services, want) in cases {
        let err = parse(
            format!("version: 1\nname: refs\nservices:\n  {services}\n").as_bytes(),
            "/code/refs",
        )
        .expect_err(name)
        .to_string();
        assert!(err.contains(want), "{name}: {err:?} lacks {want:?}");
    }
}

#[test]
fn port_references_support_dotted_service_names_and_skip_shell_expressions() {
    let p = parse(
        b"version: 1\nname: refs\nservices:\n  api.backend: {run: api, ports: {http: {default: 8080}}}\n  \
          web: {run: web, env: {API_URL: \"http://localhost:{api.backend.http}\", SHELL: \"${missing.http}\"}}\n",
        "/code/refs",
    )
    .unwrap();
    assert_eq!(p.services["web"].depends_on, strs(&["api.backend"]));
}

// port_templates_test.go -----------------------------------------------------

fn port_env_yaml(env: &str) -> String {
    format!(
        "version: 1\nname: templates\nservices: {{api: {{run: dev, ports: {{http: {{default: 8080, env: {env}}}}}}}}}"
    )
}

#[test]
fn port_env_templates() {
    let cases = [
        ("legacy scalar", "PORT", "PORT"),
        (
            "host port template",
            r#"{API_BIND: "0.0.0.0:{port}"}"#,
            "API_BIND",
        ),
        (
            "representative ignores defaults",
            r#"{A_PUBLIC_URL: {default: "http://localhost:{port}"}, Z_PORT: "{port}", API_BIND: "0.0.0.0:{port}"}"#,
            "API_BIND",
        ),
        (
            "default only cannot remap",
            r#"{PUBLIC_URL: {default: "http://localhost:{port}"}}"#,
            "",
        ),
    ];
    for (name, env, want) in cases {
        let p = parse(port_env_yaml(env).as_bytes(), "/code/templates")
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(p.services["api"].ports[0].env, want, "{name}");
    }
    let p = parse(
        port_env_yaml(r#"{PUBLIC_URL: {default: "http://localhost:{port}"}}"#).as_bytes(),
        "/code/templates",
    )
    .unwrap();
    let binding = &p.services["api"].ports[0].env_bindings["PUBLIC_URL"];
    assert!(binding.default);
    assert_eq!(binding.template, "http://localhost:{port}");
}

#[test]
fn port_env_template_validation() {
    let cases = [
        (
            "missing placeholder",
            r#"{API_BIND: "0.0.0.0:8080"}"#,
            "{port}",
        ),
        (
            "default missing placeholder",
            r#"{PORT: "{port}", URL: {default: "http://localhost:8080"}}"#,
            "{port}",
        ),
        (
            "bad variable",
            r#"{'bad-name': "{port}"}"#,
            "invalid env var name",
        ),
        (
            "unknown default field",
            r#"{PORT: "{port}", URL: {default: "http://localhost:{port}", fallback: x}}"#,
            "fallback",
        ),
        (
            "wrong default object",
            r#"{PORT: {value: "{port}"}}"#,
            "value",
        ),
        ("numeric binding", "{PORT: 1234}", "string"),
        ("null binding", "{PORT: null}", "string"),
        ("numeric default", "{PORT: {default: 1234}}", "string"),
        ("sequence", "[PORT]", "string or mapping"),
        ("empty mapping", "{}", "empty"),
        (
            "duplicate variable",
            r#"{PORT: "{port}", PORT: "{port}"}"#,
            "already defined",
        ),
    ];
    for (name, env, want) in cases {
        let err = err_text(&port_env_yaml(env));
        assert!(err.contains(want), "{name}: {err:?} lacks {want:?}");
    }
    let err = err_text(
        "version: 1\nname: templates\nservices: {api: {run: dev, ports: {http: {default: 8080, env: {PORT: \"{port}\"}, typo: true}}}}\n",
    );
    assert!(err.contains("typo"), "unknown port field accepted: {err}");
}

#[test]
fn port_env_template_schema() {
    let schema = schema_value();
    let env = &schema["$defs"]["port"]["properties"]["env"];
    let choices = env["oneOf"].as_array().expect("scalar and mapping forms");
    assert_eq!(choices.len(), 2);
    assert_eq!(choices[1]["type"], "object");
    assert_eq!(choices[1]["minProperties"], 1);
    let default_binding = &choices[1]["additionalProperties"]["oneOf"][1];
    assert_eq!(default_binding["additionalProperties"], false);
}

// probe_test.go --------------------------------------------------------------

#[test]
fn port_probe_parsing() {
    let cases = [
        ("omitted defaults on", "", true),
        ("enabled", ", probe: true", true),
        ("disabled", ", probe: false", false),
        ("anchored disabled", ", probe: &disabled false", false),
    ];
    let leased: BTreeMap<String, u16> = [("http".to_string(), 8180)].into();
    for (name, field, enabled) in cases {
        let p = parse(
            format!("version: 1\nname: probes\nservices: {{api: {{run: dev, ports: {{http: {{default: 8080{field}}}}}}}}}")
                .as_bytes(),
            "/code/probes",
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let got = health_check_for(&p.services["api"], &leased);
        assert_eq!(got.is_some(), enabled, "{name}: {got:?}");
    }
    let p = parse(
        b"version: 1\nname: probes\nservices: {api: {run: dev, ports: {debug: {default: 9000, probe: &disabled false}, http: {default: 8080, probe: *disabled}}}}",
        "/code/probes",
    )
    .unwrap();
    let leased: BTreeMap<String, u16> =
        [("debug".to_string(), 9000), ("http".to_string(), 8080)].into();
    assert_eq!(
        health_check_for(&p.services["api"], &leased),
        None,
        "aliased false must disable probing"
    );
}

#[test]
fn port_probe_validation() {
    for value in ["null", r#""false""#, "off", "0", "[]", "{}"] {
        let err = err_text(&format!(
            "version: 1\nname: probes\nservices: {{api: {{run: dev, ports: {{http: {{default: 8080, probe: {value}}}}}}}}}"
        ));
        assert!(err.contains("probe must be a boolean"), "{value}: {err}");
    }
    let cases = [
        (
            "disabled explicit target",
            "debug",
            r#"health.port "debug" has probe disabled"#,
        ),
        (
            "unknown explicit target",
            "unknown",
            r#"health.port "unknown" is not a declared port"#,
        ),
        ("enabled explicit target", "http", ""),
        ("implicit eligible target", "", ""),
    ];
    for (name, target, want) in cases {
        let yaml = format!(
            "version: 1\nname: probes\nservices: {{api: {{run: dev, ports: {{debug: {{default: 9100, probe: false}}, http: {{default: 8080}}}}, health: {{tcp: true, port: '{target}'}}}}}}"
        );
        let result = parse(yaml.as_bytes(), "/code/probes");
        if want.is_empty() {
            result.unwrap_or_else(|e| panic!("{name}: {e}"));
        } else {
            let err = result.expect_err(name).to_string();
            assert!(err.contains(want), "{name}: {err:?} lacks {want:?}");
        }
    }
}

#[test]
fn port_probe_schema() {
    let probe = &schema_value()["$defs"]["port"]["properties"]["probe"];
    assert_eq!(probe["type"], "boolean");
    assert_eq!(probe["default"], true);
}

// Beyond the Go tests: yaml.v3 behaviors the port reproduces -------------------

#[test]
fn merge_keys_and_aliases_decode() {
    let p = parse(
        b"version: 1\nname: x\nbase: &base {run: x, env: {A: '1'}}\nservices:\n  a: {<<: *base, cwd: y}\n  b:\n    <<: *base\n    run: override\n",
        "/code/x",
    );
    // `base` is not a manifest field, so strict decoding rejects it.
    assert!(p.unwrap_err().to_string().contains("field base not found"));
    let p = parse(
        b"version: 1\nname: x\nservices:\n  a: &s {run: x}\n  b: {<<: *s, cwd: y}\n",
        "/code/x",
    )
    .unwrap();
    assert_eq!(p.services["b"].run, "x");
    assert_eq!(p.services["b"].cwd, "y");
}

#[test]
fn empty_documents_are_eof() {
    assert_eq!(err_text(""), "parse rocket.yaml: EOF");
    assert_eq!(err_text("# only a comment\n"), "parse rocket.yaml: EOF");
}

#[test]
fn duplicate_keys_are_reported() {
    assert_eq!(
        err_text("version: 1\nname: x\nname: y\nservices: {a: {run: x}}\n"),
        "parse rocket.yaml: yaml: unmarshal errors:\n  line 3: mapping key \"name\" already defined at line 2"
    );
}

#[test]
fn type_errors_aggregate_in_document_order() {
    assert_eq!(
        err_text("version: [1]\nfoo: 1\nservices: {a: {run: [x]}}\n"),
        "parse rocket.yaml: yaml: unmarshal errors:\n  line 1: cannot unmarshal !!seq into int\n  line 2: field foo not found in type manifest.fileYAML\n  line 3: cannot unmarshal !!seq into string"
    );
}

// Discovery and loading ------------------------------------------------------

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

const MINIMAL: &str = "version: 1\nservices: {a: {run: x}}";

#[test]
fn find_walks_up() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), FILE_NAME, MINIMAL);
    let deep = root.path().join("a/b");
    fs::create_dir_all(&deep).unwrap();
    let got = find(&deep).unwrap();
    assert_eq!(
        got.canonicalize().unwrap(),
        root.path().canonicalize().unwrap()
    );
}

#[test]
fn find_accepts_yml_and_ignores_directories() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "rocket.yml", MINIMAL);
    let nested = root.path().join("sub");
    fs::create_dir_all(nested.join(FILE_NAME)).unwrap(); // a directory, not a manifest
    let got = find(&nested).unwrap();
    assert_eq!(
        got.canonicalize().unwrap(),
        root.path().canonicalize().unwrap()
    );
}

#[test]
fn find_reports_no_manifest() {
    let root = tempfile::tempdir().unwrap();
    // The tempdir lives under the system temp dir, which has no manifest above it.
    let err = find(root.path()).unwrap_err();
    assert!(err.is_no_manifest());
    assert_eq!(
        err.to_string(),
        format!(
            "no rocket.yaml found in {} or any parent directory",
            root.path().display()
        )
    );
}

#[test]
fn load_prefers_yaml_over_yml_and_falls_back() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "rocket.yml",
        "version: 1\nname: from-yml\nservices: {a: {run: x}}",
    );
    assert_eq!(load(root.path()).unwrap().name, "from-yml");
    write(
        root.path(),
        FILE_NAME,
        "version: 1\nname: from-yaml\nservices: {a: {run: x}}",
    );
    assert_eq!(load(root.path()).unwrap().name, "from-yaml");
}

#[test]
fn load_wraps_errors_with_the_file_path() {
    let root = tempfile::tempdir().unwrap();
    let path = write(
        root.path(),
        FILE_NAME,
        "version: 2\nservices: {a: {run: x}}",
    );
    let err = load(root.path()).unwrap_err().to_string();
    assert!(
        err.starts_with(&format!(
            "{}: invalid rocket.yaml:\n  - version must be 1 (got 2)",
            path.display()
        )),
        "{err}"
    );
    let empty = tempfile::tempdir().unwrap();
    let err = load(empty.path()).unwrap_err();
    assert!(matches!(err, Error::NotFound { .. }));
    assert_eq!(
        err.to_string(),
        format!("no rocket.yaml in {}", empty.path().display())
    );
}

#[test]
fn load_sets_root_and_default_name() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("My-Project");
    fs::create_dir(&dir).unwrap();
    write(&dir, FILE_NAME, MINIMAL);
    let p = load(&dir).unwrap();
    assert_eq!(p.name, "my-project");
    assert_eq!(Path::new(&p.root), dir.as_path());
}

#[test]
fn loader_port_returns_domain_projects() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        FILE_NAME,
        "version: 1\nname: x\nservices: {a: {run: x}}",
    );
    let p = Loader.load(root.path()).unwrap();
    assert!(p.services.contains_key("a"));
    let err = Loader
        .load(tempfile::tempdir().unwrap().path())
        .unwrap_err();
    assert!(err.to_string().starts_with("no rocket.yaml in "));
}

#[test]
fn env_files_merge_in_order_and_skip_missing() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), ".env", "A=1\nB=2\n");
    write(root.path(), ".env.local", "B=override\nC=3\n");
    let outside = tempfile::tempdir().unwrap();
    let abs = write(outside.path(), "abs.env", "D=4\n");
    let files = vec![
        ".env".to_string(),
        "missing.env".to_string(),
        ".env.local".to_string(),
        abs.display().to_string(),
    ];
    let got = EnvFiles.dotenv(root.path(), &files).unwrap();
    let want: BTreeMap<String, String> = [("A", "1"), ("B", "override"), ("C", "3"), ("D", "4")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(got, want);
}
