//! Client against in-process fake daemons (unix socket and TCP).
#![cfg(unix)]

mod common;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, Uri};
use axum::routing::{any, get, post};
use common::*;
use futures_util::StreamExt;
use rocket_client::{
    Client, ClientError, DaemonEvent, DaemonInfo, ErrorCode, EventFilter, JobsQuery, Paths,
    TransportKind,
};
use rocket_domain::api::{DownRequest, JobRequest, UpRequest};
use rocket_domain::{JobKind, JobStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<String>>>;

const JOB: &str = r#"{"id":"j3f9a0c12be","project":"nuvara","name":"ci","kind":"pipeline","owner":"user","steps":[{"run":"x"}],"status":"running","started_at":"2026-10-05T00:14:10Z","duration_ms":5}"#;
const UP: &str = r#"{"project":"nuvara","env":"dev","services":[{"service":"api","action":"started","state":"running","health":"healthy","pid":1,"ports":{"http":3000},"owner":"user"}]}"#;
const RUN: &str = r#"{"project":"nuvara","service":"api","env":"dev","kind":"run","state":"running","health":"healthy"}"#;

#[tokio::test]
async fn health_decodes_and_ignores_unknown_fields() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let app = Router::new().route("/v1/health", get(|| async { json_response(200, HEALTH) }));
    let _srv = serve_unix(app, &sock);

    let h = Client::unix(&sock).health().await.unwrap();
    assert!(h.ok);
    assert_eq!(h.api, "v1");
    assert_eq!(h.pid, 4242);
}

#[tokio::test]
async fn up_sends_go_shaped_json_and_decodes_result() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let seen: Log = Arc::default();
    let app = Router::new()
        .route(
            "/v1/up",
            post(
                |State(log): State<Log>, headers: HeaderMap, body: String| async move {
                    let ct = headers
                        .get("content-type")
                        .map(|v| v.to_str().unwrap().to_owned());
                    log.lock().unwrap().push(format!("{ct:?} {body}"));
                    json_response(200, UP)
                },
            ),
        )
        .with_state(seen.clone());
    let _srv = serve_unix(app, &sock);

    let req = UpRequest {
        project: "nuvara".into(),
        owner: "user".into(),
        ..UpRequest::default()
    };
    let res = Client::unix(&sock).up(&req).await.unwrap();
    assert_eq!(res.services[0].ports["http"], 3000);
    assert_eq!(
        seen.lock().unwrap()[0],
        r#"Some("application/json") {"project":"nuvara","owner":"user"}"#
    );
}

#[tokio::test]
async fn api_errors_map_to_typed_codes() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let app = Router::new()
        .route(
            "/v1/jobs",
            post(|| async {
                json_response(
                    428,
                    r#"{"error":"env prod requires confirmation (yes)","code":"confirmation_required"}"#,
                )
            }),
        )
        .route("/v1/ports", get(|| async { json_response(404, r#"{"error":"nope","code":"not_found"}"#) }))
        .route("/v1/gc", post(|| async { json_response(418, r#"{"error":"tea","code":"teapot"}"#) }))
        .route("/v1/ps", get(|| async { json_response(502, "<html>bad gateway</html>") }))
        .route("/v1/status", get(|| async { json_response(500, r#"{"error":"boom"}"#) }));
    let _srv = serve_unix(app, &sock);
    let c = Client::unix(&sock);

    let req = JobRequest {
        project: "p".into(),
        kind: JobKind::Deploy,
        name: "prod".into(),
        ..JobRequest::default()
    };
    match c.start_job(&req).await.unwrap_err() {
        ClientError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 428);
            assert_eq!(code, ErrorCode::ConfirmationRequired);
            assert_eq!(message, "env prod requires confirmation (yes)");
        }
        e => panic!("{e:?}"),
    }
    match c.ports().await.unwrap_err() {
        ClientError::Api {
            status: 404,
            code: ErrorCode::NotFound,
            ..
        } => {}
        e => panic!("{e:?}"),
    }
    match c.gc().await.unwrap_err() {
        ClientError::Api {
            code: ErrorCode::Other(s),
            ..
        } => assert_eq!(s, "teapot"),
        e => panic!("{e:?}"),
    }
    // Non-JSON error body: message carries status and body; code derived from status.
    match c.ps(None, false).await.unwrap_err() {
        ClientError::Api {
            status: 502,
            code,
            message,
        } => {
            assert!(
                message.starts_with("daemon returned 502: <html>"),
                "{message}"
            );
            assert_eq!(code, ErrorCode::Other(String::new()));
        }
        e => panic!("{e:?}"),
    }
    // JSON error without a code: fall back to the status.
    match c.status(None).await.unwrap_err() {
        ClientError::Api {
            status: 500,
            code: ErrorCode::Internal,
            message,
        } => {
            assert_eq!(message, "boom");
        }
        e => panic!("{e:?}"),
    }
}

#[test]
fn error_codes_roundtrip_wire_strings() {
    for (s, c) in [
        ("invalid", ErrorCode::Invalid),
        ("unauthorized", ErrorCode::Unauthorized),
        ("not_found", ErrorCode::NotFound),
        ("conflict", ErrorCode::Conflict),
        ("confirmation_required", ErrorCode::ConfirmationRequired),
        ("internal", ErrorCode::Internal),
    ] {
        assert_eq!(ErrorCode::from_wire(s), c);
        assert_eq!(c.as_str(), s);
    }
    assert_eq!(ErrorCode::from_wire("x"), ErrorCode::Other("x".into()));
}

#[tokio::test]
async fn connect_failure_is_a_connect_error() {
    let dir = short_tmp();
    let err = Client::unix(socket_in(dir.path()))
        .health()
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Connect { .. }), "{err:?}");
}

/// Records `METHOD uri` for every request and answers with canned JSON.
async fn recorder(State(log): State<Log>, method: Method, uri: Uri) -> axum::response::Response {
    log.lock().unwrap().push(format!("{method} {uri}"));
    let body = match uri.path() {
        "/v1/health" => HEALTH.to_owned(),
        "/v1/projects" if method == Method::GET => {
            r#"{"projects":[{"name":"nuvara","path":"/p","added_at":"2026-10-05T00:14:10Z"}]}"#
                .to_owned()
        }
        "/v1/projects" => {
            r#"{"name":"nuvara","path":"/p","added_at":"2026-10-05T00:14:10Z"}"#.to_owned()
        }
        p if p.starts_with("/v1/projects/") => r#"{"removed":"a b"}"#.to_owned(),
        "/v1/up" | "/v1/restart" => UP.to_owned(),
        "/v1/down" => format!(r#"{{"stopped":[{RUN}]}}"#),
        "/v1/ps" => format!(r#"{{"services":[{RUN}]}}"#),
        "/v1/logs" => r#"{"project":"nuvara","service":"api","lines":["a"]}"#.to_owned(),
        "/v1/ports" => r#"{"ports":[]}"#.to_owned(),
        "/v1/status" => format!(r#"{{"services":[{RUN}],"jobs":[],"conflicts":[]}}"#),
        "/v1/gc" => r#"{"actions":[{"action":"pruned","project":"p","service":"s"}]}"#.to_owned(),
        "/v1/shutdown" => r#"{"ok":true,"pid":9}"#.to_owned(),
        "/v1/jobs" if method == Method::GET => format!(r#"{{"jobs":[{JOB}]}}"#),
        p if p.ends_with("/logs") => r#"{"job":"j 1","project":"nuvara","lines":["x"]}"#.to_owned(),
        p if p.starts_with("/v1/jobs/") || p == "/v1/jobs" => JOB.to_owned(),
        p => panic!("unexpected path {p}"),
    };
    json_response(200, &body)
}

#[tokio::test]
async fn all_nineteen_endpoints_use_the_documented_method_and_path() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let log: Log = Arc::default();
    let app = Router::new()
        .fallback(any(recorder))
        .with_state(log.clone());
    let _srv = serve_unix(app, &sock);
    let c = Client::unix(&sock);

    c.health().await.unwrap();
    assert_eq!(c.projects().await.unwrap().projects[0].name, "nuvara");
    assert_eq!(c.add_project("/p").await.unwrap().path, "/p");
    assert_eq!(c.remove_project("a b").await.unwrap().removed, "a b");
    let up = UpRequest {
        project: "nuvara".into(),
        ..UpRequest::default()
    };
    c.up(&up).await.unwrap();
    c.restart(&up).await.unwrap();
    let down = c
        .down(&DownRequest {
            everywhere: true,
            ..DownRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(down.stopped.len(), 1);
    c.ps(Some("nuvara"), true).await.unwrap();
    assert_eq!(c.logs("nuvara", "api", 50).await.unwrap().lines, ["a"]);
    c.ports().await.unwrap();
    assert_eq!(c.status(Some("nuvara")).await.unwrap().services.len(), 1);
    assert_eq!(c.gc().await.unwrap().actions[0].action, "pruned");
    assert_eq!(c.shutdown().await.unwrap().pid, 9);
    let job = c
        .start_job(&JobRequest {
            project: "p".into(),
            name: "ci".into(),
            ..JobRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(job.status, JobStatus::Running);
    let q = JobsQuery {
        project: Some("nuvara".into()),
        all: true,
        limit: Some(10),
    };
    assert_eq!(c.jobs(&q).await.unwrap().jobs.len(), 1);
    c.job("j 1", true).await.unwrap();
    c.job("j 1", false).await.unwrap();
    c.cancel_job("j 1").await.unwrap();
    assert_eq!(c.job_logs("j 1", 100).await.unwrap().lines, ["x"]);

    assert_eq!(
        *log.lock().unwrap(),
        [
            "GET /v1/health",
            "GET /v1/projects",
            "POST /v1/projects",
            "DELETE /v1/projects/a%20b",
            "POST /v1/up",
            "POST /v1/restart",
            "POST /v1/down",
            "GET /v1/ps?project=nuvara&all=true",
            "GET /v1/logs?project=nuvara&service=api&tail=50",
            "GET /v1/ports",
            "GET /v1/status?project=nuvara",
            "POST /v1/gc",
            "POST /v1/shutdown",
            "POST /v1/jobs",
            "GET /v1/jobs?project=nuvara&all=true&limit=10",
            "GET /v1/jobs/j%201?wait=true",
            "GET /v1/jobs/j%201",
            "POST /v1/jobs/j%201/cancel",
            "GET /v1/jobs/j%201/logs?tail=100",
        ]
    );
}

#[tokio::test]
async fn query_values_are_percent_encoded() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let app = Router::new().route(
        "/v1/ps",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q["project"], "/Users/me/my proj&co");
            json_response(200, r#"{"services":[]}"#)
        }),
    );
    let _srv = serve_unix(app, &sock);
    Client::unix(&sock)
        .ps(Some("/Users/me/my proj&co"), false)
        .await
        .unwrap();
}

fn write_daemon_json(path: &std::path::Path, addr: std::net::SocketAddr, token: &str) {
    let info = format!(
        r#"{{"version":"0.1.0-dev","api":"v1","pid":4242,"socket":"/x","http":"http://{addr}","token":"{token}","rocket_bin":"/usr/local/bin/rocket","started_at":"2026-10-05T00:14:10.938611Z"}}"#
    );
    std::fs::write(path, info).unwrap();
}

#[tokio::test]
async fn tcp_sends_bearer_token_and_retries_once_after_rereading_daemon_json() {
    let dir = short_tmp();
    let djson = dir.path().join("daemon.json");
    let auth = Auth::new("fresh-token");
    let app = auth
        .clone()
        .layer(Router::new().route("/v1/health", get(|| async { json_response(200, HEALTH) })));
    let (addr, _srv) = serve_tcp(app).await;

    // The daemon restarted: the file the client loaded is stale...
    write_daemon_json(&djson, addr, "stale-token");
    let client = Client::tcp_from_file(&djson).unwrap();
    // ...and gets rotated on disk before the first request.
    write_daemon_json(&djson, addr, "fresh-token");

    let h = client.health().await.unwrap();
    assert!(h.ok);
    assert_eq!(
        *auth.seen.lock().unwrap(),
        [
            Some("Bearer stale-token".to_owned()),
            Some("Bearer fresh-token".to_owned())
        ]
    );
    // The refreshed token sticks: no second 401 round trip.
    client.health().await.unwrap();
    assert_eq!(auth.seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn tcp_unauthorized_when_token_stays_wrong() {
    let dir = short_tmp();
    let djson = dir.path().join("daemon.json");
    let auth = Auth::new("right");
    let app = auth
        .clone()
        .layer(Router::new().route("/v1/health", get(|| async { json_response(200, HEALTH) })));
    let (addr, _srv) = serve_tcp(app).await;
    write_daemon_json(&djson, addr, "wrong");

    let err = Client::tcp_from_file(&djson)
        .unwrap()
        .health()
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            ClientError::Api {
                status: 401,
                code: ErrorCode::Unauthorized,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(auth.seen.lock().unwrap().len(), 2, "exactly one retry");
}

#[tokio::test]
async fn from_paths_selects_transport() {
    let dir = short_tmp();
    let paths = Paths::from_home(dir.path()).unwrap();
    let app = Router::new().route("/v1/health", get(|| async { json_response(200, HEALTH) }));
    let _unix = serve_unix(app.clone(), &paths.socket);
    let auth = Auth::new("t");
    let (addr, _tcp) = serve_tcp(auth.clone().layer(app)).await;
    write_daemon_json(&paths.daemon_json, addr, "t");

    Client::from_paths(&paths, TransportKind::Unix)
        .unwrap()
        .health()
        .await
        .unwrap();
    assert!(auth.seen.lock().unwrap().is_empty());
    Client::from_paths(&paths, TransportKind::Tcp)
        .unwrap()
        .health()
        .await
        .unwrap();
    assert_eq!(auth.seen.lock().unwrap().len(), 1);

    let info = DaemonInfo::load(&paths.daemon_json).unwrap();
    assert_eq!(info.token, "t");
    Client::tcp(&info).unwrap().health().await.unwrap();
}

#[tokio::test]
async fn tcp_without_daemon_json_is_a_daemon_info_error() {
    let dir = short_tmp();
    let err = Client::tcp_from_file(dir.path().join("daemon.json"))
        .err()
        .unwrap();
    assert!(matches!(err, ClientError::DaemonInfo { .. }), "{err:?}");
}

const LOG_SSE: [&str; 4] = [
    ": ok\n\n",
    "event: log.line\ndata: {\"type\":\"log.line\",\"time\":\"2026-10-05T00:14:11Z\",\"project\":\"nuvara\",\"service\":\"api\",\"li",
    "ne\":\"first\"}\n\n: ping\n\nevent: future.thing\ndata: whatever\n\n",
    "event: log.line\r\ndata: {\"type\":\"log.line\",\"time\":\"2026-10-05T00:14:12Z\",\"project\":\"nuvara\",\"service\":\"api\",\"line\":\"second\"}\r\n\r\n",
];

#[tokio::test]
async fn follow_logs_streams_typed_events_until_server_closes() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let seen: Log = Arc::default();
    let app = Router::new()
        .route(
            "/v1/logs",
            get(|State(log): State<Log>, uri: Uri| async move {
                log.lock().unwrap().push(uri.to_string());
                sse_response(LOG_SSE.to_vec())
            }),
        )
        .with_state(seen.clone());
    let _srv = serve_unix(app, &sock);

    let stream = Client::unix(&sock)
        .follow_logs("nuvara", "api", Some(20))
        .await
        .unwrap();
    let items: Vec<_> = stream.collect().await;
    assert_eq!(items.len(), 3, "{items:?}");
    let lines: Vec<_> = items
        .iter()
        .filter_map(|i| match i.as_ref().unwrap() {
            DaemonEvent::LogLine(e) => Some(e.line.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, ["first", "second"]);
    assert!(matches!(items[1].as_ref().unwrap(), DaemonEvent::Raw(m) if m.event == "future.thing"));
    assert_eq!(
        seen.lock().unwrap()[0],
        "/v1/logs?project=nuvara&service=api&tail=20&follow=true"
    );
}

#[tokio::test]
async fn follow_surfaces_http_errors_before_streaming() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let app = Router::new().route(
        "/v1/logs",
        get(|| async { json_response(404, r#"{"error":"unknown service","code":"not_found"}"#) }),
    );
    let _srv = serve_unix(app, &sock);
    let err = Client::unix(&sock)
        .follow_logs("p", "nope", None)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            ClientError::Api {
                status: 404,
                code: ErrorCode::NotFound,
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn events_sends_filters_and_stops_when_dropped() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let q: Log = Arc::default();
    let app = Router::new()
        .route(
            "/v1/events",
            get(|State(q): State<Log>, Query(m): Query<HashMap<String, String>>| async move {
                let mut keys: Vec<_> = m.iter().map(|(k, v)| format!("{k}={v}")).collect();
                keys.sort();
                q.lock().unwrap().push(keys.join("&"));
                sse_response_held_open(vec![": ok\n\n", "event: port.released\ndata: {\"type\":\"port.released\",\"time\":\"2026-10-05T00:14:11Z\",\"project\":\"p\"}\n\n"])
            }),
        )
        .with_state(q.clone());
    let _srv = serve_unix(app, &sock);

    let filter = EventFilter {
        project: Some("p".into()),
        types: vec!["service.state".into(), "port.released".into()],
        ..EventFilter::default()
    };
    let mut s = Client::unix(&sock).events(&filter).await.unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), s.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(first, DaemonEvent::PortReleased(_)));
    assert_eq!(
        q.lock().unwrap()[0],
        "project=p&types=service.state,port.released"
    );
    drop(s); // must not hang or panic
}

const JOB_SSE_DONE: [&str; 3] = [
    ": ok\n\n",
    "event: job.log\ndata: {\"type\":\"job.log\",\"time\":\"2026-10-05T00:14:11Z\",\"project\":\"nuvara\",\"job_id\":\"j1\",\"line\":\"step 1\"}\n\n",
    "event: job.state\ndata: {\"type\":\"job.state\",\"time\":\"2026-10-05T00:14:12Z\",\"project\":\"nuvara\",\"job_id\":\"j1\",\"status\":\"failed\"}\n\n",
];

#[tokio::test]
async fn follow_job_logs_ends_after_terminal_job_state_even_if_server_keeps_open() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let q: Log = Arc::default();
    let app = Router::new()
        .route(
            "/v1/jobs/{id}/logs",
            get(
                |State(q): State<Log>, Path(id): Path<String>, uri: Uri| async move {
                    q.lock()
                        .unwrap()
                        .push(format!("{id} {}", uri.query().unwrap_or("")));
                    sse_response_held_open(JOB_SSE_DONE.to_vec())
                },
            ),
        )
        .with_state(q.clone());
    let _srv = serve_unix(app, &sock);

    let stream = Client::unix(&sock)
        .follow_job_logs("j1", None)
        .await
        .unwrap();
    let items: Vec<_> = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.collect::<Vec<_>>(),
    )
    .await
    .expect("stream must end after the terminal job.state");
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(matches!(items[0].as_ref().unwrap(), DaemonEvent::JobLog(e) if e.line == "step 1"));
    assert!(items[1].as_ref().unwrap().is_terminal_job_state());
    assert_eq!(q.lock().unwrap()[0], "j1 follow=true");

    let _ = Client::unix(&sock)
        .follow_job_logs("j1", Some(0))
        .await
        .unwrap();
    assert_eq!(q.lock().unwrap()[1], "j1 follow=true&tail=0");
}

#[tokio::test]
async fn follow_job_logs_reports_a_stream_closed_before_the_terminal_state() {
    let dir = short_tmp();
    let sock = socket_in(dir.path());
    let app = Router::new().route(
        "/v1/jobs/{id}/logs",
        get(|| async { sse_response(vec![JOB_SSE_DONE[0], JOB_SSE_DONE[1]]) }),
    );
    let _srv = serve_unix(app, &sock);
    let items: Vec<_> = Client::unix(&sock)
        .follow_job_logs("j1", None)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(items.len(), 2);
    assert!(items[0].is_ok());
    assert!(
        matches!(items[1], Err(ClientError::Protocol(_))),
        "{items:?}"
    );
}

#[tokio::test]
async fn sse_over_tcp_carries_the_bearer_token() {
    let auth = Auth::new("tok");
    let app = auth
        .clone()
        .layer(Router::new().route("/v1/logs", get(|| async { sse_response(LOG_SSE.to_vec()) })));
    let (addr, _srv) = serve_tcp(app).await;
    let c = Client::tcp_with(&format!("http://{addr}"), "tok", None).unwrap();
    let n = c.follow_logs("p", "s", None).await.unwrap().count().await;
    assert_eq!(n, 3);
    assert_eq!(*auth.seen.lock().unwrap(), [Some("Bearer tok".to_owned())]);
}
