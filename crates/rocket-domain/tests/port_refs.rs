//! Tests for port reference scanning (domain/port_refs.go).

use rocket_domain::{PortReference, Service, port_references};

fn refs(value: &str) -> Vec<(String, String, String)> {
    port_references(value)
        .into_iter()
        .map(|r| (r.token, r.service, r.port))
        .collect()
}

fn t(token: &str, service: &str, port: &str) -> (String, String, String) {
    (token.into(), service.into(), port.into())
}

#[test]
fn finds_unique_sorted_references() {
    assert_eq!(
        refs("http://{web.http}/{api.grpc} {web.http}"),
        vec![
            t("{api.grpc}", "api", "grpc"),
            t("{web.http}", "web", "http")
        ]
    );
}

#[test]
fn final_dot_splits_service_from_port() {
    assert_eq!(refs("{a.b.c}"), vec![t("{a.b.c}", "a.b", "c")]);
}

#[test]
fn ignores_shell_expressions_and_malformed_tokens() {
    assert!(refs("${HOME.x}").is_empty());
    assert!(refs("{nodot}").is_empty());
    assert!(refs("{.lead}").is_empty());
    assert!(refs("{trail.}").is_empty());
    assert!(refs("{has space.x}").is_empty());
    assert!(refs("{}").is_empty());
    assert!(
        refs("{a{b.c}").len() == 1,
        "scan resumes after a stray brace"
    );
    assert_eq!(refs("{a{b.c}"), vec![t("{b.c}", "b", "c")]);
}

#[test]
fn whitespace_is_ascii_only_like_go_re2() {
    // U+00A0 is not matched by Go's \s, so it stays inside the token.
    assert_eq!(
        refs("{a\u{a0}b.c}"),
        vec![t("{a\u{a0}b.c}", "a\u{a0}b", "c")]
    );
}

#[test]
fn replace_preserves_shell_expansions() {
    let r = PortReference {
        token: "{web.http}".into(),
        service: "web".into(),
        port: "http".into(),
    };
    assert_eq!(
        r.replace("a={web.http} b=${web.http} c={web.http}", "8080"),
        "a=8080 b=${web.http} c=8080"
    );
    assert_eq!(r.replace("{api.http}", "1"), "{api.http}");
}

#[test]
fn service_port_references_scan_env_values() {
    let mut svc = Service::default();
    svc.env.insert("A".into(), "{web.http}".into());
    svc.env.insert("B".into(), "x {api.grpc} {web.http}".into());
    let got: Vec<_> = svc.port_references().into_iter().map(|r| r.token).collect();
    assert_eq!(got, vec!["{api.grpc}", "{web.http}"]);
}
