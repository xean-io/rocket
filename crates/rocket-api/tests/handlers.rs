//! Port of `server_test.go` plus route-level behaviour of every endpoint.

mod common;

use axum::response::IntoResponse;
use common::{Harness, call, call_with};
use hyper::StatusCode;
use rocket_api::ApiError;
use rocket_app::AppError;
use std::sync::atomic::Ordering;

async fn error_parts(e: AppError) -> (StatusCode, String, String) {
    let resp = ApiError(e).into_response();
    let (parts, body) = resp.into_parts();
    let body = http_body_util::BodyExt::collect(body)
        .await
        .unwrap()
        .to_bytes();
    let ct = parts.headers["content-type"].to_str().unwrap().to_string();
    (parts.status, ct, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn error_mapping_matches_go_write_err() {
    let cases = [
        (
            AppError::invalid("bad"),
            400,
            "invalid",
            "invalid request: bad",
        ),
        (
            AppError::not_found("gone"),
            404,
            "not_found",
            "not found: gone",
        ),
        (
            AppError::conflict("taken"),
            409,
            "conflict",
            "conflict: taken",
        ),
        (
            AppError::confirmation_required("needs --yes"),
            428,
            "confirmation_required",
            "confirmation required: needs --yes",
        ),
        (AppError::internal("boom"), 500, "internal", "boom"),
        (AppError::Canceled, 500, "internal", "context canceled"),
    ];
    for (err, status, code, text) in cases {
        let (got_status, ct, body) = error_parts(err.clone()).await;
        assert_eq!(got_status.as_u16(), status, "{err:?}");
        assert_eq!(ct, "application/json");
        // Go writeJSON: two-space indent, trailing newline.
        assert_eq!(
            body,
            format!("{{\n  \"error\": \"{text}\",\n  \"code\": \"{code}\"\n}}\n"),
            "{err:?}"
        );
    }
}

#[tokio::test]
async fn error_text_is_json_escaped_like_go_without_html_escaping() {
    let (_, _, body) = error_parts(AppError::invalid("a<b>&\"c\"\u{2028}")).await;
    assert_eq!(
        body,
        "{\n  \"error\": \"invalid request: a<b>&\\\"c\\\"\\u2028\",\n  \"code\": \"invalid\"\n}\n"
    );
}

#[tokio::test]
async fn health_endpoint() {
    let h = Harness::new();
    let resp = call(&h.router(), "GET", "/v1/health", None).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.header("content-type"), "application/json");
    let info: rocket_domain::api::HealthInfo = serde_json::from_slice(&resp.body).unwrap();
    assert!(info.ok);
    assert_eq!(info.api, "v1");
    assert_eq!(info.pid, 7);
    assert_eq!(resp.body, rocket_api::json::pretty(&h.server.info));
}

#[tokio::test]
async fn every_route_is_registered_with_its_method() {
    let h = Harness::new();
    let r = h.router();
    // (method, uri, body, expected status): empty bodies and unknown ids give
    // the deterministic 4xx a registered route answers with.
    let cases: &[(&str, &str, Option<&str>, u16)] = &[
        ("GET", "/v1/health", None, 200),
        ("POST", "/v1/up", None, 400),
        ("POST", "/v1/restart", None, 400),
        ("POST", "/v1/down", None, 400),
        ("GET", "/v1/ps", None, 200),
        ("GET", "/v1/logs", None, 400),
        ("GET", "/v1/ports", None, 200),
        ("POST", "/v1/gc", None, 200),
        ("GET", "/v1/projects", None, 200),
        ("POST", "/v1/projects", None, 400),
        ("DELETE", "/v1/projects/nope", None, 404),
        ("GET", "/v1/status", None, 200),
        ("POST", "/v1/jobs", None, 400),
        ("GET", "/v1/jobs", None, 200),
        ("GET", "/v1/jobs/jx", None, 404),
        ("GET", "/v1/jobs/jx/logs", None, 404),
        ("POST", "/v1/jobs/jx/cancel", None, 404),
        ("GET", "/v1/events", None, 200),
    ];
    for (method, uri, body, want) in cases {
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            call_stream_head(&r, method, uri, *body),
        )
        .await
        .unwrap_or_else(|_| panic!("{method} {uri} timed out"));
        assert_eq!(resp, *want, "{method} {uri}");
    }
}

/// Status only: SSE bodies never end, so the body is not consumed.
async fn call_stream_head(r: &axum::Router, method: &str, uri: &str, body: Option<&str>) -> u16 {
    use tower::ServiceExt;
    let req = hyper::Request::builder()
        .method(method)
        .uri(uri)
        .body(axum::body::Body::from(body.unwrap_or("").to_owned()))
        .unwrap();
    r.clone().oneshot(req).await.unwrap().status().as_u16()
}

#[tokio::test]
async fn unknown_route_and_wrong_method_use_go_mux_texts() {
    let h = Harness::new();
    let r = h.router();
    let nf = call(&r, "GET", "/v1/nope", None).await;
    assert_eq!(nf.status, 404);
    assert_eq!(nf.text(), "404 page not found\n");
    assert_eq!(nf.header("content-type"), "text/plain; charset=utf-8");
    assert_eq!(nf.header("x-content-type-options"), "nosniff");
    for (method, uri) in [
        ("DELETE", "/v1/health"),
        ("POST", "/v1/health"),
        ("GET", "/v1/up"),
        ("PUT", "/v1/jobs"),
    ] {
        let na = call(&r, method, uri, None).await;
        assert_eq!(na.status, 405, "{method} {uri}");
        assert_eq!(na.text(), "Method Not Allowed\n");
        assert_eq!(na.header("content-type"), "text/plain; charset=utf-8");
        assert!(
            !na.header("allow").is_empty(),
            "{method} {uri}: missing Allow header"
        );
    }
}

#[tokio::test]
async fn unknown_request_fields_are_rejected_on_every_body_endpoint() {
    let h = Harness::new();
    let r = h.router();
    for (uri, body) in [
        ("/v1/up", r#"{"project":"x","bogus":1}"#),
        ("/v1/restart", r#"{"project":"x","bogus":1}"#),
        ("/v1/down", r#"{"project":"x","bogus":1}"#),
        ("/v1/projects", r#"{"path":"/x","bogus":1}"#),
        ("/v1/jobs", r#"{"project":"x","kind":"pipeline","bogus":1}"#),
    ] {
        let resp = call(&r, "POST", uri, Some(body)).await;
        assert_eq!(resp.status, 400, "{uri}");
        assert_eq!(
            resp.json(),
            serde_json::json!({
                "error": "invalid request: bad JSON body: json: unknown field \"bogus\"",
                "code": "invalid"
            }),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn bad_bodies_use_go_error_texts() {
    let h = Harness::new();
    let r = h.router();
    for (body, want) in [
        ("", "EOF"),
        ("{\"project\":", "unexpected EOF"),
        (
            "nope",
            "invalid character 'o' in literal null (expecting 'u')",
        ),
        (
            "[]",
            "json: cannot unmarshal array into Go value of type app.UpRequest",
        ),
        (
            r#"{"project":5}"#,
            "json: cannot unmarshal number into Go struct field UpRequest.project of type string",
        ),
    ] {
        let resp = call(&r, "POST", "/v1/up", Some(body)).await;
        assert_eq!(resp.status, 400, "{body:?}");
        assert_eq!(
            resp.json()["error"],
            format!("invalid request: bad JSON body: {want}"),
            "{body:?}"
        );
    }
}

#[tokio::test]
async fn up_with_unknown_service_is_a_400_not_a_partial_failure() {
    let h = Harness::new();
    let dir = h.fixture();
    let body = format!(r#"{{"project":"{}","services":["nope"]}}"#, dir.display());
    let resp = call(&h.router(), "POST", "/v1/up", Some(&body)).await;
    assert_eq!(resp.status, 400);
    assert_eq!(
        resp.json()["error"],
        "invalid request: unknown service or group \"nope\" in project rocket-fixture"
    );
}

#[tokio::test]
async fn project_registry_lifecycle() {
    let h = Harness::new();
    let r = h.router();
    let dir = h.fixture();

    let empty = call(&r, "GET", "/v1/projects", None).await;
    assert_eq!(empty.text(), "{\n  \"projects\": []\n}\n");

    let rel = call(
        &r,
        "POST",
        "/v1/projects",
        Some(r#"{"path":"relative/dir"}"#),
    )
    .await;
    assert_eq!(rel.status, 400);
    assert_eq!(
        rel.json()["error"],
        "invalid request: project path must be absolute"
    );

    let add = call(
        &r,
        "POST",
        "/v1/projects",
        Some(&format!(r#"{{"path":"{}"}}"#, dir.display())),
    )
    .await;
    assert_eq!(add.status, 200, "{}", add.text());
    assert_eq!(add.json()["name"], "rocket-fixture");
    assert_eq!(add.json()["path"], dir.to_str().unwrap());

    let list = call(&r, "GET", "/v1/projects", None).await;
    assert_eq!(list.json()["projects"].as_array().unwrap().len(), 1);

    let ps = call(&r, "GET", "/v1/ps?project=rocket-fixture", None).await;
    assert_eq!(ps.status, 200);
    let services = ps.json()["services"].as_array().unwrap().clone();
    assert_eq!(services.len(), 4);
    assert!(services.iter().all(|s| s["state"] == "stopped"));

    let ps_all = call(&r, "GET", "/v1/ps?all=true&project=ignored", None).await;
    assert_eq!(ps_all.json()["services"].as_array().unwrap().len(), 0);

    let status = call(&r, "GET", "/v1/status?project=rocket-fixture", None).await;
    assert_eq!(status.status, 200);
    assert_eq!(status.json()["project"]["name"], "rocket-fixture");

    let rm = call(&r, "DELETE", "/v1/projects/rocket-fixture", None).await;
    assert_eq!(rm.status, 200);
    assert_eq!(rm.text(), "{\n  \"removed\": \"rocket-fixture\"\n}\n");
    let again = call(&r, "DELETE", "/v1/projects/rocket-fixture", None).await;
    assert_eq!(again.status, 404);
    assert_eq!(
        again.json()["error"],
        "not found: project \"rocket-fixture\" is not registered"
    );
}

#[tokio::test]
async fn project_names_in_paths_are_percent_decoded() {
    let h = Harness::new();
    let resp = call(&h.router(), "DELETE", "/v1/projects/my%20proj", None).await;
    assert_eq!(resp.status, 404);
    assert_eq!(
        resp.json()["error"],
        "not found: project \"my proj\" is not registered"
    );
}

#[tokio::test]
async fn ports_gc_and_empty_listings() {
    let h = Harness::new();
    let r = h.router();
    assert_eq!(
        call(&r, "GET", "/v1/ports", None).await.text(),
        "{\n  \"ports\": []\n}\n"
    );
    assert_eq!(
        call(&r, "POST", "/v1/gc", None).await.text(),
        "{\n  \"actions\": []\n}\n"
    );
    assert_eq!(
        call(&r, "GET", "/v1/jobs?all=true&limit=abc", None)
            .await
            .text(),
        "{\n  \"jobs\": []\n}\n"
    );
    assert_eq!(
        call(&r, "GET", "/v1/ps", None).await.text(),
        "{\n  \"services\": []\n}\n"
    );
}

#[tokio::test]
async fn job_validation_errors() {
    let h = Harness::new();
    let r = h.router();
    let dir = h.fixture();
    let p = dir.display();
    let post = |body: String| {
        let r = r.clone();
        async move { call(&r, "POST", "/v1/jobs", Some(&body)).await }
    };

    // Go resolves the project before judging the kind.
    let resp = post(r#"{"kind":"weird"}"#.into()).await;
    assert_eq!(resp.status, 400);
    assert_eq!(
        resp.json()["error"],
        "invalid request: project is required (run inside a directory with rocket.yaml or pass -p)"
    );
    let resp = post(format!(r#"{{"project":"{p}","kind":"weird"}}"#)).await;
    assert_eq!(resp.status, 400);
    assert_eq!(
        resp.json()["error"],
        "invalid request: unknown job kind \"weird\" (setup, pipeline or deploy)"
    );
    let resp = post(format!(r#"{{"project":"{p}"}}"#)).await;
    assert_eq!(resp.status, 400);
    assert_eq!(
        resp.json()["error"],
        "invalid request: unknown job kind \"\" (setup, pipeline or deploy)"
    );
    // The deploy gate answers 428 before anything runs.
    let resp = post(format!(
        r#"{{"project":"{p}","kind":"deploy","name":"stage"}}"#
    ))
    .await;
    assert_eq!(resp.status, 428, "{}", resp.text());
    assert_eq!(resp.json()["code"], "confirmation_required");
}

#[tokio::test]
async fn pipeline_job_runs_to_completion_through_the_api() {
    let h = Harness::new();
    let r = h.router();
    let dir = h.fixture();
    let start = call(
        &r,
        "POST",
        "/v1/jobs",
        Some(&format!(
            r#"{{"project":"{}","kind":"pipeline","name":"echo-args","args":["<&>\u2028x"]}}"#,
            dir.display()
        )),
    )
    .await;
    assert_eq!(start.status, 200, "{}", start.text());
    assert_eq!(start.json()["status"], "running");
    let id = start.json()["id"].as_str().unwrap().to_string();

    let done = call(&r, "GET", &format!("/v1/jobs/{id}?wait=true"), None).await;
    assert_eq!(done.status, 200);
    assert_eq!(done.json()["status"], "succeeded", "{}", done.text());
    // Go's writeJSON: no HTML escaping, but U+2028 is always escaped.
    assert!(done.text().contains("\"<&>\\u2028x\""), "{}", done.text());

    let get = call(&r, "GET", &format!("/v1/jobs/{id}"), None).await;
    assert_eq!(get.json()["exit_code"], 0);

    let logs = call(&r, "GET", &format!("/v1/jobs/{id}/logs?tail=50"), None).await;
    assert_eq!(logs.status, 200);
    assert_eq!(logs.json()["job"], id);
    let lines = logs.json()["lines"].as_array().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|l| l.as_str().unwrap().starts_with("args: <&>")),
        "{lines:?}"
    );

    let list = call(&r, "GET", "/v1/jobs?project=rocket-fixture&limit=1", None).await;
    assert_eq!(list.json()["jobs"].as_array().unwrap().len(), 1);

    let cancel = call(&r, "POST", &format!("/v1/jobs/{id}/cancel"), None).await;
    assert_eq!(cancel.status, 200);
    assert_eq!(cancel.json()["status"], "succeeded");
}

#[tokio::test]
async fn failing_pipeline_reports_exit_code() {
    let h = Harness::new();
    let r = h.router();
    let dir = h.fixture();
    let start = call(
        &r,
        "POST",
        "/v1/jobs",
        Some(&format!(
            r#"{{"project":"{}","kind":"pipeline","name":"broken"}}"#,
            dir.display()
        )),
    )
    .await;
    let id = start.json()["id"].as_str().unwrap().to_string();
    let done = call(&r, "GET", &format!("/v1/jobs/{id}?wait=true"), None).await;
    assert_eq!(done.json()["status"], "failed");
    assert_eq!(done.json()["exit_code"], 3);
}

#[tokio::test]
async fn query_flags_follow_strconv_parse_bool() {
    let h = Harness::new();
    let r = h.router();
    let dir = h.fixture();
    let add = call(
        &r,
        "POST",
        "/v1/projects",
        Some(&format!(r#"{{"path":"{}"}}"#, dir.display())),
    )
    .await;
    assert_eq!(add.status, 200);
    // all=1 / all=T mean true: the (unknown) project filter is ignored.
    for v in ["1", "T", "true"] {
        let ps = call(&r, "GET", &format!("/v1/ps?all={v}&project=nope"), None).await;
        assert_eq!(ps.status, 200, "all={v}");
    }
    // all=yes is false: the unknown project is an error again.
    let ps = call(&r, "GET", "/v1/ps?all=yes&project=nope", None).await;
    assert_eq!(ps.status, 404);
}

#[tokio::test]
async fn shutdown_replies_then_invokes_the_callback() {
    let h = Harness::new();
    let resp = call(&h.router(), "POST", "/v1/shutdown", None).await;
    assert_eq!(resp.status, 200);
    let v = resp.json();
    assert_eq!(v["ok"], true);
    assert_eq!(v["pid"].as_u64().unwrap(), u64::from(std::process::id()));
    assert_eq!(
        resp.text(),
        format!(
            "{{\n  \"ok\": true,\n  \"pid\": {}\n}}\n",
            std::process::id()
        )
    );
    for _ in 0..100 {
        if h.shutdowns.load(Ordering::SeqCst) == 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("shutdown callback was not invoked");
}

#[tokio::test]
async fn content_type_of_the_request_is_not_checked() {
    let h = Harness::new();
    let resp = call_with(
        &h.router(),
        "POST",
        "/v1/up",
        Some(r#"{"project":"x","bogus":true}"#),
        &[("content-type", "text/plain")],
    )
    .await;
    assert_eq!(resp.status, 400);
    assert!(resp.text().contains("unknown field"));
}
