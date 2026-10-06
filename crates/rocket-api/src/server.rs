//! The router and the plain (non-streaming) handlers (Go: `server.go`).

use crate::decode::{self, Schema};
use crate::error::{ApiError, json_response, text_response};
use crate::query::Query;
use crate::streams;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{delete, get, post};
use rocket_app::{App, AppError, CancellationToken, LogsRequest, StatusRequest, jobs::JobsRequest};
use rocket_domain::api::{
    AddProjectRequest, DownRequest, HealthInfo, JobRequest, ProjectsResult, RemovedProject,
    ShutdownResult, UpRequest,
};
use rocket_domain::ports::EventBus;
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// Version of the API contract, reported by `/v1/health`.
pub const VERSION: &str = "v1";

/// How often idle SSE streams send a `: ping` comment.
pub const HEARTBEAT: Duration = Duration::from_secs(15);

type Shutdown = Arc<dyn Fn() + Send + Sync>;

/// Adapts [`App`] to HTTP. Cheap to clone; the clone shares everything.
#[derive(Clone)]
pub struct Server {
    pub app: App,
    pub bus: Arc<dyn EventBus>,
    pub info: HealthInfo,
    /// Period of the SSE `: ping` comments (Go: 15s; tests shorten it).
    pub heartbeat: Duration,
    shutdown: Option<Shutdown>,
    closing: CancellationToken,
}

impl Server {
    pub fn new(app: App, bus: Arc<dyn EventBus>, info: HealthInfo) -> Self {
        Self {
            app,
            bus,
            info,
            heartbeat: HEARTBEAT,
            shutdown: None,
            closing: CancellationToken::new(),
        }
    }

    /// Overrides the SSE heartbeat period.
    #[must_use]
    pub fn with_heartbeat(mut self, period: Duration) -> Self {
        self.heartbeat = period;
        self
    }

    /// Registers the function `POST /v1/shutdown` invokes asynchronously
    /// after its reply (Go: `Server.Shutdown`).
    #[must_use]
    pub fn on_shutdown(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.shutdown = Some(Arc::new(f));
        self
    }

    /// Cancelling this token ends every open SSE stream; the daemon does it
    /// on shutdown so graceful serving can finish.
    pub fn closing(&self) -> CancellationToken {
        self.closing.clone()
    }

    /// The routed handler, without authentication.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/v1/health", get(health))
            .route("/v1/up", post(up))
            .route("/v1/restart", post(restart))
            .route("/v1/down", post(down))
            .route("/v1/ps", get(ps))
            .route("/v1/logs", get(logs))
            .route("/v1/ports", get(ports))
            .route("/v1/gc", post(gc))
            .route("/v1/projects", get(list_projects).post(add_project))
            .route("/v1/projects/{name}", delete(remove_project))
            .route("/v1/status", get(status))
            .route("/v1/jobs", post(start_job).get(list_jobs))
            .route("/v1/jobs/{id}", get(get_job))
            .route("/v1/jobs/{id}/logs", get(streams::job_logs))
            .route("/v1/jobs/{id}/cancel", post(cancel_job))
            .route("/v1/events", get(streams::events))
            .route("/v1/shutdown", post(shutdown))
            .fallback(|| async { text_response(StatusCode::NOT_FOUND, "404 page not found") })
            .method_not_allowed_fallback(|| async {
                text_response(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
            })
            .with_state(self.clone())
    }
}

type Res = Result<Response, ApiError>;

/// `200` with a Go `writeJSON` body.
pub(crate) fn ok<T: Serialize + ?Sized>(v: &T) -> Response {
    json_response(StatusCode::OK, crate::json::pretty(v))
}

/// Runs a synchronous use case off the async workers: store, manifest and
/// probe calls can block (`PortProbe::holder` waits up to 3s on `lsof`).
pub(crate) async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(AppError::internal(format!("handler task failed: {e}"))))
}

/// Runs a long operation to completion even when the client disconnects and
/// the handler future is dropped (Go: `opCtx` = `context.WithoutCancel`).
pub(crate) async fn detached<T, Fut>(fut: Fut) -> Result<T, AppError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, AppError>> + Send + 'static,
{
    tokio::spawn(fut)
        .await
        .unwrap_or_else(|e| Err(AppError::internal(format!("handler task failed: {e}"))))
}

fn bad_body(msg: String) -> AppError {
    AppError::invalid(format!("bad JSON body: {msg}"))
}

fn decode_body<T: serde::de::DeserializeOwned>(
    body: &[u8],
    schema: &Schema,
) -> Result<T, AppError> {
    decode::decode(body, schema).map_err(bad_body)
}

async fn health(State(s): State<Server>) -> Response {
    ok(&s.info)
}

async fn up(State(s): State<Server>, body: Bytes) -> Res {
    let req: UpRequest = decode_body(&body, &decode::UP)?;
    let app = s.app.clone();
    Ok(ok(&detached(async move { app.up(req).await }).await?))
}

async fn restart(State(s): State<Server>, body: Bytes) -> Res {
    let req: UpRequest = decode_body(&body, &decode::UP)?;
    let app = s.app.clone();
    Ok(ok(&detached(async move { app.restart(req).await }).await?))
}

async fn down(State(s): State<Server>, body: Bytes) -> Res {
    let req: DownRequest = decode_body(&body, &decode::DOWN)?;
    let app = s.app.clone();
    Ok(ok(&detached(async move { app.down(req).await }).await?))
}

async fn ps(State(s): State<Server>, RawQuery(raw): RawQuery) -> Res {
    let q = Query::parse(raw.as_deref());
    let req = StatusRequest {
        project: q.get("project").to_string(),
        all_projects: q.flag("all"),
    };
    let app = s.app.clone();
    Ok(ok(&blocking(move || app.status(&req)).await?))
}

async fn logs(State(s): State<Server>, RawQuery(raw): RawQuery) -> Res {
    let q = Query::parse(raw.as_deref());
    let req = LogsRequest {
        project: q.get("project").to_string(),
        service: q.get("service").to_string(),
        tail: q.int("tail").unwrap_or(0),
    };
    if q.flag("follow") {
        return streams::follow_logs(s, req).await;
    }
    Ok(ok(&s.app.logs(req).await?))
}

async fn ports(State(s): State<Server>) -> Res {
    let app = s.app.clone();
    Ok(ok(&blocking(move || app.ports()).await?))
}

async fn gc(State(s): State<Server>) -> Res {
    let app = s.app.clone();
    Ok(ok(&detached(async move { app.gc().await }).await?))
}

async fn list_projects(State(s): State<Server>) -> Res {
    let app = s.app.clone();
    let projects = blocking(move || app.list_projects()).await?;
    Ok(ok(&ProjectsResult { projects }))
}

async fn add_project(State(s): State<Server>, body: Bytes) -> Res {
    let req: AddProjectRequest = decode_body(&body, &decode::ADD_PROJECT)?;
    let app = s.app.clone();
    Ok(ok(&blocking(move || app.add_project(&req.path)).await?))
}

async fn remove_project(State(s): State<Server>, Path(name): Path<String>) -> Res {
    let app = s.app.clone();
    let n = name.clone();
    blocking(move || app.remove_project(&n)).await?;
    Ok(ok(&RemovedProject { removed: name }))
}

async fn status(State(s): State<Server>, RawQuery(raw): RawQuery) -> Res {
    let q = Query::parse(raw.as_deref());
    let project = q.get("project").to_string();
    let app = s.app.clone();
    Ok(ok(&blocking(move || app.summary(&project)).await?))
}

async fn start_job(State(s): State<Server>, body: Bytes) -> Res {
    let mut fields = decode::decode_object(&body, &decode::JOB).map_err(bad_body)?;
    // Go validates the kind inside StartJob, after resolving the project; the
    // Rust enum cannot hold an unknown kind, so keep it aside and fail at the
    // same point.
    let kind = match fields.get("kind") {
        Some(Value::String(k)) => k.clone(),
        _ => String::new(),
    };
    let bad_kind = !matches!(kind.as_str(), "setup" | "pipeline" | "deploy");
    if bad_kind {
        fields.insert("kind".into(), Value::String("pipeline".into()));
    }
    let req: JobRequest =
        serde_json::from_value(Value::Object(fields)).map_err(|e| bad_body(e.to_string()))?;
    let app = s.app.clone();
    let job = blocking(move || {
        if bad_kind {
            app.resolve_project(&req.project)?;
            return Err(AppError::invalid(format!(
                "unknown job kind {kind:?} (setup, pipeline or deploy)"
            )));
        }
        app.start_job(req)
    })
    .await?;
    Ok(ok(&job))
}

async fn list_jobs(State(s): State<Server>, RawQuery(raw): RawQuery) -> Res {
    let q = Query::parse(raw.as_deref());
    let req = JobsRequest {
        project: q.get("project").to_string(),
        all_projects: q.flag("all"),
        limit: q.int("limit").unwrap_or(0),
    };
    let app = s.app.clone();
    Ok(ok(&blocking(move || app.list_jobs(&req)).await?))
}

async fn get_job(State(s): State<Server>, Path(id): Path<String>, RawQuery(raw): RawQuery) -> Res {
    let q = Query::parse(raw.as_deref());
    let app = s.app.clone();
    let job = if q.flag("wait") {
        // Dropping the handler future abandons the wait.
        app.wait_job(&id, &CancellationToken::new()).await?
    } else {
        blocking(move || app.get_job(&id)).await?
    };
    Ok(ok(&job))
}

async fn cancel_job(State(s): State<Server>, Path(id): Path<String>) -> Res {
    let app = s.app.clone();
    Ok(ok(
        &detached(async move { app.cancel_job(&id).await }).await?
    ))
}

async fn shutdown(State(s): State<Server>) -> Response {
    let reply = ok(&ShutdownResult {
        ok: true,
        pid: i32::try_from(std::process::id()).unwrap_or(0),
    });
    if let Some(f) = s.shutdown.clone() {
        // After the reply: a task, like Go's `go s.Shutdown()`.
        tokio::spawn(async move { f() });
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn detached_work_survives_a_dropped_handler() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let handler = detached(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        // The client disconnects: hyper drops the handler future.
        let _ = tokio::time::timeout(Duration::from_millis(5), handler).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            done.load(Ordering::SeqCst),
            "operation was cancelled with the handler"
        );
    }

    #[tokio::test]
    async fn blocking_reports_panics_as_internal_errors() {
        let r: Result<(), AppError> = blocking(|| panic!("boom")).await;
        assert_eq!(r.unwrap_err().code(), "internal");
    }
}
