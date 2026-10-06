//! Port of domain/graph_test.go and profiles_test.go.

use rocket_domain::{
    Error, Project, Service, ServiceKind, build_env, compose_project_name, merge_profiles,
    remap_candidates,
};
use std::collections::BTreeMap;

type ExpandCase<'a> = (&'a str, Vec<&'a str>, Result<Vec<String>, &'a str>);
type StartupCase<'a> = (&'a str, Vec<&'a str>, Vec<&'a str>, Option<Vec<&'a str>>);

fn svc(name: &str, kind: ServiceKind, deps: &[&str]) -> Service {
    Service {
        name: name.into(),
        kind,
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        ..Service::default()
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn sample_project() -> Project {
    let mut p = Project {
        name: "nuvara".into(),
        ..Project::default()
    };
    for s in [
        svc("postgres", ServiceKind::Compose, &[]),
        svc("redis", ServiceKind::Compose, &[]),
        svc("api", ServiceKind::Run, &["postgres", "redis"]),
        svc("web", ServiceKind::Run, &["api"]),
        svc("causation", ServiceKind::Task, &[]),
    ] {
        p.services.insert(s.name.clone(), s);
    }
    p.groups.insert("deps".into(), strs(&["postgres", "redis"]));
    p.groups.insert("core".into(), strs(&["api", "web"]));
    p.groups.insert("all".into(), strs(&["*"]));
    p
}

#[test]
fn expand_targets() {
    let p = sample_project();
    let all = strs(&["api", "causation", "postgres", "redis", "web"]);
    let cases: Vec<ExpandCase> = vec![
        ("empty means all services", vec![], Ok(all.clone())),
        ("single service", vec!["web"], Ok(strs(&["web"]))),
        (
            "group expands",
            vec!["deps"],
            Ok(strs(&["postgres", "redis"])),
        ),
        ("star group expands to all", vec!["all"], Ok(all.clone())),
        ("literal star", vec!["*"], Ok(all.clone())),
        (
            "duplicates removed",
            vec!["web", "core"],
            Ok(strs(&["api", "web"])),
        ),
        (
            "unknown target",
            vec!["nope"],
            Err(r#"unknown service or group "nope""#),
        ),
    ];
    for (name, input, want) in cases {
        let got = p.expand_targets(&strs(&input));
        match want {
            Ok(w) => assert_eq!(got.unwrap(), w, "{name}"),
            Err(msg) => {
                let err = got.unwrap_err();
                assert!(err.to_string().contains(msg), "{name}: {err}");
                assert!(matches!(err, Error::UnknownTarget { .. }));
            }
        }
    }
}

#[test]
fn start_order_includes_dependencies_first() {
    let got = sample_project().start_order(&strs(&["web"])).unwrap();
    assert_eq!(got, strs(&["postgres", "redis", "api", "web"]));
}

#[test]
fn start_order_detects_cycle() {
    let mut p = Project::default();
    for s in [
        svc("a", ServiceKind::Run, &["b"]),
        svc("b", ServiceKind::Run, &["c"]),
        svc("c", ServiceKind::Run, &["a"]),
    ] {
        p.services.insert(s.name.clone(), s);
    }
    let err = p.start_order(&strs(&["a"])).unwrap_err();
    assert!(err.to_string().contains("cycle"), "{err}");
    assert_eq!(
        err.to_string(),
        "dependency cycle between services: a, b, c"
    );
}

#[test]
fn stop_order_is_reverse_dependency_order() {
    let got = sample_project().stop_order(&strs(&["redis", "web", "api", "postgres"]));
    assert_eq!(got, strs(&["web", "api", "redis", "postgres"]));
}

#[test]
fn stop_order_keeps_unknown_services_last_and_survives_cycles() {
    let p = sample_project();
    let got = p.stop_order(&strs(&["zeta", "web", "alpha", "api"]));
    assert_eq!(got, strs(&["web", "api", "alpha", "zeta"]));

    let mut cyc = Project::default();
    for s in [
        svc("a", ServiceKind::Run, &["b"]),
        svc("b", ServiceKind::Run, &["a"]),
    ] {
        cyc.services.insert(s.name.clone(), s);
    }
    assert_eq!(cyc.stop_order(&strs(&["b", "a"])), strs(&["a", "b"]));
}

#[test]
fn remap_candidates_jump_then_step_and_stay_in_range() {
    let got = remap_candidates(3000);
    assert_eq!(&got[..3], &[3100, 3101, 3102]);
    assert_eq!(got.len(), 200);
    // 65500 + 100 + i exceeds 65535 and wraps to 1024 + (c - 65535).
    let high = remap_candidates(65500);
    assert_eq!(high[0], 1024 + 65);
    assert!(high.iter().all(|&c| c >= 1024));
}

#[test]
fn compose_project_name_sanitizes() {
    assert_eq!(
        compose_project_name("Xean.SpectrAI", "dev"),
        "rocket-xean-spectrai-dev"
    );
    assert_eq!(compose_project_name("..a b..", "--x--"), "rocket--a-b----x");
    // Go lowercases per code point (U+0130 -> "i"), not with full case folding.
    assert_eq!(compose_project_name("\u{130}x", "dev"), "rocket-ix-dev");
}

#[test]
fn build_env_precedence_and_rocket_filter() {
    let base = strs(&[
        "PATH=/bin",
        "PORT=1",
        "ROCKET_OWNER=agent:x",
        "KEEP=base",
        "NOEQ",
    ]);
    let l = |kv: &[(&str, &str)]| -> BTreeMap<String, String> {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let got = build_env(
        &base,
        &[
            &l(&[("PORT", "2"), ("FROM_DOTENV", "yes"), ("KEEP", "dotenv")]),
            &l(&[("KEEP", "svc")]),
            &l(&[("PORT", "3100")]),
        ],
    );
    assert_eq!(
        got,
        strs(&["FROM_DOTENV=yes", "KEEP=svc", "PATH=/bin", "PORT=3100"])
    );
}

#[test]
fn expand_startup_targets_filters_only_wildcards() {
    let mut p = Project {
        name: "profiles".into(),
        ..Project::default()
    };
    let mk = |name: &str, deps: &[&str], profiles: &[&str]| Service {
        name: name.into(),
        depends_on: strs(deps),
        profiles: strs(profiles),
        ..Service::default()
    };
    for s in [
        mk("api", &["internal"], &[]),
        mk("internal", &[], &["hidden"]),
        mk("ordinary", &[], &[]),
        mk("trends", &[], &["trends", "reports"]),
    ] {
        p.services.insert(s.name.clone(), s);
    }
    p.groups.insert("all".into(), strs(&["*"]));
    p.groups.insert("explicit".into(), strs(&["trends"]));
    p.groups.insert("mixed".into(), strs(&["*", "trends"]));

    let cases: Vec<StartupCase> = vec![
        ("empty", vec![], vec![], Some(vec!["api", "ordinary"])),
        ("star", vec!["*"], vec![], Some(vec!["api", "ordinary"])),
        (
            "all group",
            vec!["all"],
            vec![],
            Some(vec!["api", "ordinary"]),
        ),
        (
            "any matching profile",
            vec!["all"],
            vec!["reports"],
            Some(vec!["api", "ordinary", "trends"]),
        ),
        (
            "all active",
            vec!["all"],
            vec!["hidden", "trends"],
            Some(vec!["api", "internal", "ordinary", "trends"]),
        ),
        (
            "unknown profile selects no gated service",
            vec!["all"],
            vec!["unused"],
            Some(vec!["api", "ordinary"]),
        ),
        (
            "explicit service",
            vec!["trends"],
            vec![],
            Some(vec!["trends"]),
        ),
        (
            "explicit group",
            vec!["explicit"],
            vec![],
            Some(vec!["trends"]),
        ),
        (
            "mixed group",
            vec!["mixed"],
            vec![],
            Some(vec!["api", "ordinary", "trends"]),
        ),
        (
            "mixed targets",
            vec!["*", "trends", "trends"],
            vec![],
            Some(vec!["api", "ordinary", "trends"]),
        ),
        ("invalid", vec!["unknown"], vec![], None),
    ];
    for (name, targets, profiles, want) in cases {
        let got = p.expand_startup_targets(&strs(&targets), &strs(&profiles));
        match want {
            Some(w) => assert_eq!(got.unwrap(), strs(&w), "{name}"),
            None => assert!(
                got.unwrap_err()
                    .to_string()
                    .contains("unknown service or group"),
                "{name}"
            ),
        }
    }
    let targets = p.expand_startup_targets(&[], &[]).unwrap();
    // Gated dependency must remain selectable.
    assert_eq!(
        p.start_order(&targets).unwrap(),
        strs(&["internal", "api", "ordinary"])
    );
    // Stop expansion stays unfiltered.
    assert_eq!(
        p.expand_targets(&strs(&["all"])).unwrap(),
        p.service_names()
    );
}

#[test]
fn merge_profiles_sorts_and_dedups() {
    let got = merge_profiles(&[
        strs(&["trends", "base"]),
        strs(&["extra", "base"]),
        strs(&["trends", "reports"]),
    ]);
    assert_eq!(got, strs(&["base", "extra", "reports", "trends"]));
}
