//! Ports of the `internal/adapters/compose/*_test.go` files. Fixture-script
//! tests run everywhere with sh; the Docker tests are `#[ignore]`d (run them
//! with `cargo test -p rocket-adapters -- --ignored`) and skip themselves when
//! `docker info` fails.

use rocket_adapters::compose::{Driver, args, named_volumes_from_config};
use rocket_domain::build_env;
use rocket_domain::ports::{ComposeDriver, ComposeTarget};
use std::collections::BTreeMap;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

#[test]
fn args_force_project_name_files_and_profiles() {
    let target = ComposeTarget {
        project_name: "rocket-nuvara-smoke".into(),
        files: strs(&["/code/nuvara/docker-compose.prod.yml"]),
        profiles: strs(&["release", "deps"]),
        ..Default::default()
    };
    assert_eq!(
        args(&target, &["up", "-d", "--wait", "postgres"]),
        strs(&[
            "compose",
            "-p",
            "rocket-nuvara-smoke",
            "-f",
            "/code/nuvara/docker-compose.prod.yml",
            "--profile",
            "release",
            "--profile",
            "deps",
            "up",
            "-d",
            "--wait",
            "postgres"
        ])
    );
}

#[test]
fn named_volumes_from_config_table() {
    struct Case {
        name: &'static str,
        config: &'static str,
        want: Option<Vec<&'static str>>,
    }
    let cases = [
        Case {
            name: "mounted named only",
            config: r#"{"services":{"db":{"volumes":[{"type":"volume","source":"data","target":"/data"},{"type":"volume","source":"custom","target":"/custom"},{"type":"volume","source":"external","target":"/external"},{"type":"bind","source":"data","target":"/bind"},{"type":"volume","target":"/anonymous"},{"type":"volume","source":"data","target":"/again"}]},"other":{"volumes":[{"type":"volume","source":"unused","target":"/unused"}]}},"volumes":{"data":{"name":"rocket-test-dev_data"},"custom":{"name":"custom_data"},"external":{"name":"shared_data","external":true},"unused":{"name":"rocket-test-dev_unused"}}}"#,
            want: Some(vec!["custom_data", "rocket-test-dev_data"]),
        },
        Case {
            name: "no volumes",
            config: r#"{"services":{"db":{}}}"#,
            want: Some(vec![]),
        },
        Case {
            name: "unknown selected service",
            config: r#"{"services":{"other":{}}}"#,
            want: None,
        },
        Case {
            name: "undefined named source",
            config: r#"{"services":{"db":{"volumes":[{"type":"volume","source":"missing"}]}}}"#,
            want: None,
        },
        Case {
            name: "missing normalized name",
            config: r#"{"services":{"db":{"volumes":[{"type":"volume","source":"data"}]}},"volumes":{"data":{}}}"#,
            want: None,
        },
        Case {
            name: "bad JSON",
            config: "not JSON",
            want: None,
        },
    ];
    for c in cases {
        let got = named_volumes_from_config(c.config.as_bytes(), "db");
        match (&c.want, got) {
            (Some(want), Ok(got)) => assert_eq!(&got, want, "{}", c.name),
            (None, Err(_)) => {}
            (want, got) => panic!("{}: want {want:?}, got {got:?}", c.name),
        }
    }
}

#[test]
fn named_volumes_error_messages_match_go() {
    let msg = |config: &str| {
        named_volumes_from_config(config.as_bytes(), "db")
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        msg(r#"{"services":{"other":{}}}"#),
        "compose config has no service \"db\""
    );
    assert_eq!(
        msg(r#"{"services":{"db":{"volumes":[{"type":"volume","source":"m"}]}}}"#),
        "compose config has no volume \"m\""
    );
    assert_eq!(
        msg(
            r#"{"services":{"db":{"volumes":[{"type":"volume","source":"d"}]}},"volumes":{"d":{}}}"#
        ),
        "compose volume \"d\" has no normalized name"
    );
    assert!(msg("not JSON").starts_with("decode compose config: "));
}

// ---- fixture-script tests (unix) -------------------------------------------

#[cfg(unix)]
mod fixture {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn write_script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn env_with(extra: &[(&str, &str)]) -> Vec<String> {
        let base: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
        let layer: BTreeMap<String, String> = extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        build_env(&base, &[&layer])
    }

    #[tokio::test]
    async fn up_retains_compose_wait() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("argv");
        let bin = write_script(
            dir.path(),
            "docker-fixture",
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$TRACE_FILE\"\nprintf '%s\\n' --next-- >> \"$TRACE_FILE\"\ncase \"$4\" in ps) printf '%s\\n' fixture-container ;; esac\n",
        );
        let target = ComposeTarget {
            project_name: "rocket-probe-fixture-dev".into(),
            dir: dir.path().to_path_buf(),
            env: vec![
                format!("TRACE_FILE={}", trace.display()),
                "PATH=/usr/bin:/bin".into(),
            ],
            ..Default::default()
        };
        let cid = Driver::with_bin(bin)
            .up(&target, "backend", &mut std::io::sink())
            .await
            .unwrap();
        assert_eq!(cid, "fixture-container");
        let got = std::fs::read_to_string(&trace).unwrap();
        let want = [
            "compose",
            "-p",
            &target.project_name,
            "up",
            "-d",
            "--wait",
            "backend",
            "--next--",
            "compose",
            "-p",
            &target.project_name,
            "ps",
            "-q",
            "backend",
            "--next--",
            "",
        ]
        .join("\n");
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn volume_commands_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("argv");
        let script = r#"#!/bin/sh
case "$1" in
  compose)
    printf '%s\n' "$@" > "$TRACE_FILE"
    if [ "$FAIL_CONFIG" = 1 ]; then exit 1; fi
    printf '%s\n' '{"services":{"db":{"volumes":[{"type":"volume","source":"data"}]}},"volumes":{"data":{"name":"fixture_data"}}}'
    ;;
  volume)
    if [ "$INVENTORY_MARKER" != from-target ] || [ ! . -ef "$EXPECTED_DIR" ]; then
      printf '%s\n' wrong_endpoint
      exit 0
    fi
    if [ "$FAIL_VOLUMES" = 1 ]; then exit 1; fi
    printf '%s\n' fixture_data fixture_data_old
    ;;
esac
"#;
        let bin = write_script(dir.path(), "docker-fixture", script);
        let d = Driver::with_bin(bin);
        let mut target = ComposeTarget {
            project_name: "rocket-volumes-fixture-dev".into(),
            dir: dir.path().to_path_buf(),
            files: strs(&["compose.yaml"]),
            profiles: strs(&["fixture"]),
            env: env_with(&[
                ("TRACE_FILE", trace.to_str().unwrap()),
                ("INVENTORY_MARKER", "from-target"),
                ("EXPECTED_DIR", dir.path().to_str().unwrap()),
            ]),
        };
        let names = d.named_volumes(&target, "db").await.unwrap();
        assert_eq!(names, ["fixture_data"]);
        let got = std::fs::read_to_string(&trace).unwrap();
        let want = [
            "compose",
            "-p",
            &target.project_name,
            "-f",
            "compose.yaml",
            "--profile",
            "fixture",
            "config",
            "--format",
            "json",
        ];
        assert_eq!(got.split_whitespace().collect::<Vec<_>>(), want);

        for (name, exists) in [
            ("fixture_data", true),
            ("fixture", false),
            ("missing", false),
        ] {
            assert_eq!(
                d.volume_exists(&target, name).await.unwrap(),
                exists,
                "exact name {name:?}"
            );
        }
        target.env = env_with(&[
            ("TRACE_FILE", trace.to_str().unwrap()),
            ("INVENTORY_MARKER", "from-target"),
            ("EXPECTED_DIR", dir.path().to_str().unwrap()),
            ("FAIL_CONFIG", "1"),
        ]);
        assert!(
            d.named_volumes(&target, "db").await.is_err(),
            "failed config command provided evidence"
        );
        target.env.push("FAIL_VOLUMES=1".into());
        assert!(
            d.volume_exists(&target, "fixture_data").await.is_err(),
            "failed volume inventory provided evidence"
        );
    }

    #[tokio::test]
    async fn failed_commands_report_exit_status_and_stderr_tail() {
        let dir = tempfile::tempdir().unwrap();
        let bin = write_script(
            dir.path(),
            "docker-fixture",
            "#!/bin/sh\necho out-line\necho 'something broke' >&2\nexit 3\n",
        );
        let target = ComposeTarget {
            project_name: "p".into(),
            dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        let mut out = Vec::new();
        let err = Driver::with_bin(bin)
            .down(&target, &mut out)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "exit status 3: something broke");
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("out-line") && text.contains("something broke"),
            "{text:?}"
        );
    }

    #[tokio::test]
    async fn long_stderr_is_truncated_to_the_last_400_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bin = write_script(
            dir.path(),
            "docker-fixture",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 100 ]; do echo \"0123456789 line $i\" >&2; i=$((i+1)); done\nexit 1\n",
        );
        let err = Driver::with_bin(bin)
            .stop(&ComposeTarget::default(), "svc", &mut std::io::sink())
            .await
            .unwrap_err()
            .to_string();
        let msg = err.strip_prefix("exit status 1: ").unwrap();
        assert!(msg.starts_with('…') && msg.ends_with("line 99"), "{msg}");
        assert!(msg.len() <= 400 + '…'.len_utf8() + 2, "{}", msg.len());
    }

    #[tokio::test]
    async fn running_uses_docker_ps_with_compose_label_filters() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("argv");
        let bin = write_script(
            dir.path(),
            "docker-fixture",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nprintf '%s\\n' abc123\n",
                trace.display()
            ),
        );
        let d = Driver::with_bin(bin);
        assert!(d.running("rocket-p-dev", "api").await.unwrap());
        assert_eq!(
            std::fs::read_to_string(&trace)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                "ps",
                "-q",
                "--filter",
                "label=com.docker.compose.project=rocket-p-dev",
                "--filter",
                "label=com.docker.compose.service=api",
                "--filter",
                "status=running"
            ]
        );
        let none = write_script(dir.path(), "docker-none", "#!/bin/sh\nexit 0\n");
        assert!(!Driver::with_bin(none).running("p", "s").await.unwrap());
    }

    #[tokio::test]
    async fn logs_merge_stdout_and_stderr_and_split_lines() {
        let dir = tempfile::tempdir().unwrap();
        let trace = dir.path().join("argv");
        let bin = write_script(
            dir.path(),
            "docker-fixture",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\necho line1\necho line2 >&2\n",
                trace.display()
            ),
        );
        let target = ComposeTarget {
            project_name: "rocket-p-dev".into(),
            dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        let mut lines = Driver::with_bin(bin)
            .logs(&target, "api", 50)
            .await
            .unwrap();
        lines.sort();
        assert_eq!(lines, ["line1", "line2"]);
        assert_eq!(
            std::fs::read_to_string(&trace)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                "compose",
                "-p",
                "rocket-p-dev",
                "logs",
                "--no-color",
                "--tail",
                "50",
                "api"
            ]
        );
        let empty = write_script(dir.path(), "docker-empty", "#!/bin/sh\nexit 0\n");
        assert!(
            Driver::with_bin(empty)
                .logs(&target, "api", 5)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

// ---- Docker tests (ignored by default) --------------------------------------

mod docker {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn docker_available() -> bool {
        Command::new("docker")
            .arg("info")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn unique(prefix: &str) -> String {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{prefix}-{n:x}")
    }

    fn owned_containers(project: &str) -> String {
        let out = Command::new("docker")
            .args([
                "ps",
                "-aq",
                "--filter",
                &format!("label=com.docker.compose.project={project}"),
            ])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon"]
    async fn compose_lifecycle() {
        if !docker_available() {
            eprintln!("skipped: docker not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("compose.yaml");
        std::fs::write(
            &file,
            "services:\n  box:\n    image: busybox\n    command: ['sh', '-c', 'sleep 300']\n    healthcheck:\n      test: ['CMD', 'true']\n      interval: 1s\n",
        )
        .unwrap();
        let d = Driver::new();
        let target = ComposeTarget {
            project_name: unique("rocket-itest"),
            dir: dir.path().to_path_buf(),
            files: vec![file.to_str().unwrap().into()],
            env: std::env::vars().map(|(k, v)| format!("{k}={v}")).collect(),
            ..Default::default()
        };
        let up = d.up(&target, "box", &mut std::io::stderr()).await;
        let result = async {
            let cid = up.map_err(|e| format!("up: {e}"))?;
            assert!(!cid.is_empty());
            assert!(
                d.running(&target.project_name, "box")
                    .await
                    .map_err(|e| e.to_string())?
            );
            d.stop(&target, "box", &mut std::io::stderr())
                .await
                .map_err(|e| e.to_string())?;
            assert!(
                !d.running(&target.project_name, "box").await.unwrap_or(true),
                "still running after stop"
            );
            Ok::<_, String>(())
        }
        .await;
        let down = d.down(&target, &mut std::io::stderr()).await;
        assert_eq!(
            owned_containers(&target.project_name),
            "",
            "owned containers remain"
        );
        result.unwrap();
        down.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a Docker daemon"]
    async fn named_volumes_fresh_existing_and_external() {
        if !docker_available() {
            eprintln!("skipped: docker not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let project = unique("rocket-volume-itest");
        let external = format!("{project}_external");
        let created = Command::new("docker")
            .args([
                "volume",
                "create",
                "--label",
                &format!("rocket.test.project={project}"),
                &external,
            ])
            .output()
            .unwrap();
        assert!(created.status.success(), "create fixture volume");
        let spec = format!(
            "name: ignored-manual-name\nservices:\n  box:\n    image: busybox\n    profiles: [fixture]\n    command: ['sh', '-c', 'sleep 300']\n    healthcheck: {{test: ['CMD', 'true'], interval: 1s}}\n    volumes:\n      - data:/data\n      - custom:/custom\n      - external:/external\n      - ./bind:/bind\nvolumes:\n  data: {{}}\n  custom: {{name: \"${{COMPOSE_PROJECT_NAME}}_${{VOLUME_SUFFIX}}\"}}\n  unused: {{}}\n  external: {{external: true, name: \"{external}\"}}\n"
        );
        std::fs::create_dir(dir.path().join("bind")).unwrap();
        let file = dir.path().join("compose.yaml");
        std::fs::write(&file, spec).unwrap();
        let base: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
        let layer = BTreeMap::from([
            ("COMPOSE_PROJECT_NAME".to_string(), project.clone()),
            ("VOLUME_SUFFIX".to_string(), "custom".to_string()),
        ]);
        let target = ComposeTarget {
            project_name: project.clone(),
            dir: dir.path().to_path_buf(),
            files: vec![file.to_str().unwrap().into()],
            profiles: strs(&["fixture"]),
            env: build_env(&base, &[&layer]),
        };
        let d = Driver::new();
        let sink = || std::io::sink();

        let names = d.named_volumes(&target, "box").await.unwrap();
        assert_eq!(
            names,
            [format!("{project}_custom"), format!("{project}_data")]
        );
        for n in &names {
            assert!(
                !d.volume_exists(&target, n).await.unwrap(),
                "fresh volume {n} exists"
            );
        }
        assert!(d.volume_exists(&target, &external).await.unwrap());
        let up = d.up(&target, "box", &mut sink()).await;
        let after_up = async {
            for n in &names {
                assert!(
                    d.volume_exists(&target, n).await.unwrap(),
                    "new volume {n} not confirmed after up"
                );
            }
        };
        if up.is_ok() {
            after_up.await;
            d.down(&target, &mut sink()).await.unwrap();
            for n in &names {
                assert!(
                    d.volume_exists(&target, n).await.unwrap(),
                    "down failed to retain {n}"
                );
            }
            assert!(!d.up(&target, "box", &mut sink()).await.unwrap().is_empty());
        }
        let down = d.down(&target, &mut sink()).await;
        assert_eq!(owned_containers(&project), "");
        // Remove only the uniquely named volumes this test created.
        let mut rm = Command::new("docker");
        rm.args(["volume", "rm", &external]).args(&names);
        let _ = rm.output();
        up.unwrap();
        down.unwrap();
    }
}
