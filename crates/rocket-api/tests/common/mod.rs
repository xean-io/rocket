//! Shared test harness: a real `App` over temp-dir adapters (no processes are
//! started unless a test runs a job), the API router, and raw HTTP helpers.
#![allow(dead_code)]

use axum::Router;
use axum::body::Body;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::header::HeaderMap;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rocket_adapters::{compose, events, logs, probe, process, sqlite, task};
use rocket_api::Server;
use rocket_app::{App, Deps, Options};
use rocket_domain::api::HealthInfo;
use rocket_domain::ports::EventBus;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tower::ServiceExt;

pub struct Harness {
    pub tmp: TempDir,
    pub app: App,
    pub bus: Arc<events::Bus>,
    pub store: Arc<sqlite::Store>,
    pub shutdowns: Arc<AtomicUsize>,
    pub server: Server,
}

pub fn health_info() -> HealthInfo {
    HealthInfo {
        ok: true,
        api: rocket_api::VERSION.to_string(),
        version: "9.9.9-test".into(),
        pid: 7,
        started_at: time::macros::datetime!(2026-10-05 00:14:10 UTC),
        home: "/h".into(),
        socket: "/h/rocketd.sock".into(),
        http: "http://127.0.0.1:1".into(),
    }
}

impl Harness {
    pub fn new() -> Self {
        Self::with_heartbeat(Duration::from_secs(15))
    }

    pub fn with_heartbeat(heartbeat: Duration) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("rka")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let store = Arc::new(sqlite::Store::open(&tmp.path().join("state.db")).unwrap());
        let bus = Arc::new(events::Bus::new());
        let sink = Arc::new(logs::Sink::new(
            tmp.path().join("logs"),
            Some(bus.clone() as Arc<dyn EventBus>),
        ));
        let app = App::new(Deps {
            store: store.clone(),
            manifests: Arc::new(rocket_manifest::Loader),
            env: Arc::new(rocket_manifest::EnvFiles),
            runner: Arc::new(process::Runner),
            compose: Arc::new(compose::Driver::new()),
            tasks: Arc::new(task::Driver::new()),
            probe: Arc::new(probe::Ports::new()),
            health: Arc::new(probe::Health::new()),
            bus: Some(bus.clone() as Arc<dyn EventBus>),
            logs: sink,
            options: Options::default(),
        });
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let counter = shutdowns.clone();
        let server = Server::new(app.clone(), bus.clone(), health_info())
            .with_heartbeat(heartbeat)
            .on_shutdown(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            });
        Self {
            tmp,
            app,
            bus,
            store,
            shutdowns,
            server,
        }
    }

    pub fn router(&self) -> Router {
        self.server.router()
    }

    /// A private copy of `testdata/fixture` under the harness temp dir.
    pub fn fixture(&self) -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/fixture");
        let dst = std::fs::canonicalize(self.tmp.path()).unwrap().join("proj");
        std::fs::create_dir_all(&dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        }
        dst
    }
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Resp {
    pub fn text(&self) -> String {
        String::from_utf8(self.body.to_vec()).unwrap()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.text()))
    }

    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap())
            .unwrap_or("")
    }
}

/// Calls `router` in-process (no socket); streaming bodies are not consumed.
pub async fn call(router: &Router, method: &str, uri: &str, body: Option<&str>) -> Resp {
    call_with(router, method, uri, body, &[]).await
}

pub async fn call_with(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> Resp {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = req.body(Body::from(body.unwrap_or("").to_owned())).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let (parts, body) = resp.into_parts();
    let body = body.collect().await.unwrap().to_bytes();
    Resp {
        status: parts.status,
        headers: parts.headers,
        body,
    }
}

/// Serves `router` on 127.0.0.1:0 until the test ends.
pub async fn serve(router: Router) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (addr, handle)
}

/// A streaming response read as raw text (hyper strips chunked framing).
pub struct Stream {
    pub status: StatusCode,
    pub headers: HeaderMap,
    body: Incoming,
    pub buf: String,
    pub ended: bool,
    _conn: tokio::task::JoinHandle<()>,
}

pub async fn open(addr: SocketAddr, path: &str) -> Stream {
    open_with(addr, path, &[]).await
}

pub async fn open_with(addr: SocketAddr, path: &str, headers: &[(&str, &str)]) -> Stream {
    let io = TokioIo::new(tokio::net::TcpStream::connect(addr).await.unwrap());
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
    let conn = tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut req = Request::builder().uri(path).header("host", "rocketd");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = sender
        .send_request(req.body(http_body_util::Empty::<Bytes>::new()).unwrap())
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    Stream {
        status: parts.status,
        headers: parts.headers,
        body,
        buf: String::new(),
        ended: false,
        _conn: conn,
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self._conn.abort(); // closes the socket, like a client going away
    }
}

impl Stream {
    /// Reads until the accumulated text contains `pat`; panics on timeout or
    /// end of stream. Returns everything accumulated so far.
    pub async fn read_until(&mut self, pat: &str) -> &str {
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            while !self.buf.contains(pat) {
                if !self.pull().await {
                    return false;
                }
            }
            true
        })
        .await;
        match found {
            Ok(true) => &self.buf,
            Ok(false) => panic!("stream ended before {pat:?}; got {:?}", self.buf),
            Err(_) => panic!("timeout waiting for {pat:?}; got {:?}", self.buf),
        }
    }

    /// Reads until the server ends the stream.
    pub async fn read_to_end(&mut self) -> &str {
        let done = tokio::time::timeout(Duration::from_secs(10), async {
            while self.pull().await {}
        })
        .await;
        assert!(done.is_ok(), "stream did not end; got {:?}", self.buf);
        &self.buf
    }

    /// Reads whatever arrives within `quiet` of silence (or until the end).
    pub async fn read_quiet(&mut self, quiet: Duration) -> &str {
        while let Ok(true) = tokio::time::timeout(quiet, self.pull()).await {}
        &self.buf
    }

    async fn pull(&mut self) -> bool {
        if self.ended {
            return false;
        }
        match self.body.frame().await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    self.buf.push_str(&String::from_utf8_lossy(&data));
                }
                true
            }
            _ => {
                self.ended = true;
                false
            }
        }
    }
}

/// The `data:` JSON payloads of the frames accumulated so far.
pub fn data_events(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|d| serde_json::from_str(d).unwrap())
        .collect()
}

pub fn event(kind: &str, project: &str, service: &str) -> rocket_domain::Event {
    rocket_domain::Event {
        r#type: kind.to_string(),
        time: time::macros::datetime!(2026-10-05 00:14:10 UTC),
        project: project.to_string(),
        service: service.to_string(),
        state: None,
        line: String::new(),
        run: None,
        lease: None,
        job_id: String::new(),
        status: None,
        job: None,
    }
}
