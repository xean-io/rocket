//! Shared helpers: short temp dirs (unix socket paths are limited to ~103
//! bytes) and in-process fake daemons built on axum.
#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use bytes::Bytes;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::net::{TcpListener, UnixListener};

/// A temp dir directly under /tmp so `<dir>/rocketd.sock` stays short.
pub fn short_tmp() -> TempDir {
    tempfile::Builder::new()
        .prefix("rkc")
        .tempdir_in("/tmp")
        .expect("tempdir")
}

pub fn socket_in(dir: &Path) -> PathBuf {
    dir.join("rocketd.sock")
}

/// Serve `app` on a unix socket until the test ends.
pub fn serve_unix(app: Router, socket: &Path) -> tokio::task::JoinHandle<()> {
    let listener = UnixListener::bind(socket).expect("bind unix");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    })
}

/// Serve `app` on 127.0.0.1 with an ephemeral port.
pub async fn serve_tcp(app: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind tcp");
    let addr = listener.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, h)
}

pub fn json_response(status: u16, body: &str) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// An SSE response that emits `chunks` in order then closes.
pub fn sse_response(chunks: Vec<&'static str>) -> Response {
    use futures_util::stream;
    let s = stream::iter(
        chunks
            .into_iter()
            .map(|c| Ok::<Bytes, Infallible>(Bytes::from_static(c.as_bytes()))),
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(s))
        .unwrap()
}

/// An SSE response that emits `chunks` and then never closes.
pub fn sse_response_held_open(chunks: Vec<&'static str>) -> Response {
    use futures_util::{StreamExt, stream};
    let s = stream::iter(
        chunks
            .into_iter()
            .map(|c| Ok::<Bytes, Infallible>(Bytes::from_static(c.as_bytes()))),
    )
    .chain(stream::pending());
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(s))
        .unwrap()
}

/// Requires `Authorization: Bearer <token>` where the token can be rotated by
/// the test; counts every request that reaches the middleware.
#[derive(Clone)]
pub struct Auth {
    pub token: Arc<Mutex<String>>,
    pub seen: Arc<Mutex<Vec<Option<String>>>>,
}

impl Auth {
    pub fn new(token: &str) -> Self {
        Self {
            token: Arc::new(Mutex::new(token.to_owned())),
            seen: Arc::default(),
        }
    }

    pub fn layer(self, app: Router) -> Router {
        app.layer(middleware::from_fn_with_state(self, check))
    }
}

async fn check(
    axum::extract::State(auth): axum::extract::State<Auth>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let got = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    auth.seen.lock().unwrap().push(got.clone());
    let want = format!("Bearer {}", auth.token.lock().unwrap());
    if got.as_deref() == Some(want.as_str()) {
        next.run(req).await
    } else {
        let mut r = json_response(
            401,
            r#"{"error":"missing or invalid bearer token (see daemon.json)","code":"unauthorized"}"#,
        );
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            "Bearer realm=\"rocketd\"".parse().unwrap(),
        );
        r
    }
}

pub const HEALTH: &str = r#"{"ok":true,"api":"v1","version":"0.1.0-dev","pid":4242,"started_at":"2026-10-05T00:14:10Z","home":"/h","socket":"/h/rocketd.sock","future_field":{"a":1}}"#;
