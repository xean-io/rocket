//! Differential tests against the Go implementation.
//!
//! `tests/golden/corpus.json` holds inputs plus the exact results observed
//! from Go's `manifest.Parse` / `ParseDotenv`; `tests/golden/schema.json` is
//! the output of `rocket schema`. Regenerate them from the repository root
//! with the Go toolchain (see `tests/golden/README.md`).

use rocket_manifest::{parse, parse_dotenv, schema};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Corpus {
    parse_cases: Vec<ParseCase>,
    dotenv_cases: Vec<DotenvCase>,
}

#[derive(Deserialize)]
struct ParseCase {
    name: String,
    yaml: String,
    root: String,
    ok: bool,
    #[serde(default)]
    error: String,
    #[serde(default)]
    unstable: bool,
    #[serde(default)]
    syntax: bool,
    #[serde(default)]
    project: Value,
    #[serde(default)]
    needs: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    bindings: BTreeMap<String, BTreeMap<String, BTreeMap<String, Binding>>>,
}

#[derive(Deserialize, PartialEq, Debug)]
struct Binding {
    template: String,
    default: bool,
}

#[derive(Deserialize)]
struct DotenvCase {
    name: String,
    input: String,
    want: BTreeMap<String, String>,
}

fn corpus() -> Corpus {
    let raw = include_str!("golden/corpus.json");
    serde_json::from_str(raw).expect("corpus.json is valid")
}

/// Go ranges over maps in random order, so some aggregated problem lists
/// are only stable as a set.
fn line_set(msg: &str) -> BTreeSet<&str> {
    msg.lines().collect()
}

#[test]
fn parse_matches_go_oracle() {
    let corpus = corpus();
    let mut failures = Vec::new();
    for case in &corpus.parse_cases {
        let got = parse(case.yaml.as_bytes(), &case.root);
        if case.syntax {
            // libyaml and saphyr word syntax errors differently; only the
            // accept/reject decision and the error envelope must agree.
            match (&got, case.ok) {
                (Ok(_), true) => {}
                (Err(e), false) if e.to_string().starts_with("parse rocket.yaml: ") => {}
                _ => failures.push(format!(
                    "{}: syntax case disagrees (go ok={}, rust={:?})",
                    case.name,
                    case.ok,
                    got.as_ref().map(|_| ()).map_err(ToString::to_string)
                )),
            }
            continue;
        }
        match (got, case.ok) {
            (Ok(project), true) => {
                let value = serde_json::to_value(&project).expect("project serializes");
                // Go marshals a nil slice as `null`; Rust has no nil slice.
                let mut expected = case.project.clone();
                if let Some(groups) = expected.get_mut("groups").and_then(Value::as_object_mut) {
                    for members in groups.values_mut().filter(|v| v.is_null()) {
                        *members = Value::Array(Vec::new());
                    }
                }
                if value != expected {
                    failures.push(format!(
                        "{}: project differs\n  go:   {}\n  rust: {}",
                        case.name, case.project, value
                    ));
                }
                let needs: BTreeMap<String, Vec<String>> = project.pipeline_needs.clone();
                if needs != case.needs {
                    failures.push(format!(
                        "{}: pipeline needs differ\n  go:   {:?}\n  rust: {:?}",
                        case.name, case.needs, needs
                    ));
                }
                let mut bindings: BTreeMap<String, BTreeMap<String, BTreeMap<String, Binding>>> =
                    BTreeMap::new();
                for (sn, svc) in &project.services {
                    for port in &svc.ports {
                        if port.env_bindings.is_empty() {
                            continue;
                        }
                        bindings.entry(sn.clone()).or_default().insert(
                            port.name.clone(),
                            port.env_bindings
                                .iter()
                                .map(|(k, b)| {
                                    (
                                        k.clone(),
                                        Binding {
                                            template: b.template.clone(),
                                            default: b.default,
                                        },
                                    )
                                })
                                .collect(),
                        );
                    }
                }
                if bindings != case.bindings {
                    failures.push(format!("{}: env bindings differ", case.name));
                }
            }
            (Err(e), false) => {
                let msg = e.to_string();
                let same = if case.unstable {
                    line_set(&msg) == line_set(&case.error)
                } else {
                    msg == case.error
                };
                if !same {
                    failures.push(format!(
                        "{}: error differs\n  go:   {:?}\n  rust: {:?}",
                        case.name, case.error, msg
                    ));
                }
            }
            (Ok(_), false) => failures.push(format!(
                "{}: rust accepted, go said {:?}",
                case.name, case.error
            )),
            (Err(e), true) => failures.push(format!("{}: rust rejected: {e}", case.name)),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus cases differ from Go:\n{}",
        failures.len(),
        corpus.parse_cases.len(),
        failures.join("\n")
    );
}

#[test]
fn dotenv_matches_go_oracle() {
    let mut failures = Vec::new();
    for case in corpus().dotenv_cases {
        let got = parse_dotenv(case.input.as_bytes());
        if got != case.want {
            failures.push(format!("{}: go {:?} rust {:?}", case.name, case.want, got));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn schema_is_byte_identical_to_go() {
    // `rocket schema` prints `Schema()` followed by a newline.
    let want = include_str!("golden/schema.json");
    assert_eq!(format!("{}\n", schema()), want);
}
