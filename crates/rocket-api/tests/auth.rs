//! Port of `TestRequireTokenGuardsEveryRequest` (internal/daemon/tcp_test.go).

mod common;

use axum::Router;
use axum::routing::{get, post};
use common::{Harness, call_with, open_with, serve};
use rocket_api::auth::{ct_eq, require_token};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn guarded() -> Router {
    let ok = Router::new()
        .route("/v1/health", get(|| async { "ok" }))
        .route("/v1/shutdown", post(|| async { "bye" }));
    require_token(ok, TOKEN)
}

#[tokio::test]
async fn every_request_needs_the_bearer_token() {
    let wrong = format!("Bearer {}x", &TOKEN[..63]);
    let valid = format!("Bearer {TOKEN}");
    let basic = format!("Basic {TOKEN}");
    let lower = format!("bearer {TOKEN}");
    let trailing = format!("Bearer {TOKEN} ");
    let cases: Vec<(&str, Option<&str>, u16)> = vec![
        ("no header", None, 401),
        ("wrong token", Some(&wrong), 401),
        ("wrong scheme", Some(&basic), 401),
        ("lowercase scheme", Some(&lower), 401),
        ("trailing space", Some(&trailing), 401),
        ("short token", Some("Bearer abc"), 401),
        ("empty bearer", Some("Bearer "), 401),
        ("valid token", Some(&valid), 200),
    ];
    for (name, header, want) in cases {
        let headers: Vec<(&str, &str)> = header.map(|h| ("authorization", h)).into_iter().collect();
        let resp = call_with(&guarded(), "GET", "/v1/health", None, &headers).await;
        assert_eq!(resp.status.as_u16(), want, "{name}");
        if want == 401 {
            // Go: compact json.Encoder output (newline-terminated).
            assert_eq!(
                resp.text(),
                "{\"error\":\"missing or invalid bearer token (see daemon.json)\",\"code\":\"unauthorized\"}\n",
                "{name}"
            );
            assert_eq!(
                resp.header("www-authenticate"),
                "Bearer realm=\"rocketd\"",
                "{name}"
            );
            assert_eq!(resp.header("content-type"), "application/json", "{name}");
        } else {
            assert_eq!(resp.text(), "ok");
        }
    }
}

#[tokio::test]
async fn the_guard_covers_all_methods_and_unknown_paths() {
    let r = guarded();
    for (method, uri) in [
        ("POST", "/v1/shutdown"),
        ("DELETE", "/v1/health"),
        ("GET", "/nope"),
    ] {
        let resp = call_with(&r, method, uri, None, &[]).await;
        assert_eq!(resp.status, 401, "{method} {uri}");
    }
}

#[test]
fn constant_time_compare() {
    assert!(ct_eq(b"abc", b"abc"));
    assert!(ct_eq(b"", b""));
    assert!(!ct_eq(b"abc", b"abd"));
    assert!(!ct_eq(b"abc", b"ab"));
    assert!(!ct_eq(b"ab", b"abc"));
    assert!(!ct_eq(b"", b"a"));
}

#[tokio::test]
async fn token_guard_protects_the_whole_api_including_streams_and_side_effects() {
    let h = Harness::new();
    let (addr, _srv) = serve(require_token(h.router(), TOKEN)).await;

    // No token: JSON 401, never an event stream, and nothing is executed.
    let mut s = open_with(addr, "/v1/events", &[]).await;
    assert_eq!(s.status, 401);
    let body = s.read_to_end().await.to_string();
    assert!(body.contains("\"code\":\"unauthorized\""), "{body}");
    let resp = call_with(
        &require_token(h.router(), TOKEN),
        "POST",
        "/v1/shutdown",
        None,
        &[],
    )
    .await;
    assert_eq!(resp.status, 401);
    assert_eq!(h.shutdowns.load(std::sync::atomic::Ordering::SeqCst), 0);

    // With the token the same stream opens.
    let auth = format!("Bearer {TOKEN}");
    let mut s = open_with(addr, "/v1/events", &[("authorization", &auth)]).await;
    assert_eq!(s.status, 200);
    assert_eq!(s.header_ct(), "text/event-stream");
    s.read_until(": ok\n\n").await;

    // The unguarded router (the unix socket) needs no token.
    let (open_addr, _srv2) = serve(h.router()).await;
    let mut s = open_with(open_addr, "/v1/events", &[]).await;
    assert_eq!(s.status, 200);
    s.read_until(": ok\n\n").await;
}

trait CtExt {
    fn header_ct(&self) -> String;
}
impl CtExt for common::Stream {
    fn header_ct(&self) -> String {
        self.headers["content-type"].to_str().unwrap().to_string()
    }
}
