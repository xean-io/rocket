//! The typed client: one async method per daemon endpoint.

use crate::daemon_info::DaemonInfo;
use crate::error::{ClientError, ErrorCode};
use crate::events::{EventStream, event_stream};
use crate::paths::Paths;
use crate::transport::Transport;
use http_body_util::BodyExt;
use hyper::Method;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rocket_domain::api::{
    DownRequest, DownResult, ErrorBody, GcResult, HealthInfo, JobLogsResult, JobRequest,
    JobsResult, LogsResult, PortsResult, ProjectsResult, RemovedProject, ShutdownResult,
    StatusResult, Summary, UpRequest, UpResult,
};
use rocket_domain::{Job, ProjectRef};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Which listener a client talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportKind {
    /// `$ROCKET_HOME/rocketd.sock`; no token (mode 0600 is the access control).
    #[cfg_attr(unix, default)]
    Unix,
    /// `http://127.0.0.1:<port>` from daemon.json with a bearer token.
    #[cfg_attr(not(unix), default)]
    Tcp,
}

/// Filters of `GET /v1/events` (combined with AND).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventFilter {
    pub project: Option<String>,
    pub service: Option<String>,
    pub job: Option<String>,
    /// Event types, e.g. `service.state`; empty means all.
    pub types: Vec<String>,
}

/// Query of `GET /v1/jobs`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobsQuery {
    pub project: Option<String>,
    pub all: bool,
    /// Server default is 50.
    pub limit: Option<u32>,
}

/// Handle to a daemon. Cheap to clone; every call opens its own connection.
#[derive(Clone)]
pub struct Client {
    transport: Arc<Transport>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

const ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, ENCODE).to_string()
}

/// Builds `?a=b&c=d` (empty string for no pairs), values percent-encoded.
fn query(pairs: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", enc(v))))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

fn some_if(cond: bool, v: &str) -> Option<String> {
    cond.then(|| v.to_owned())
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty()).map(str::to_owned)
}

impl Client {
    /// A client for the unix socket at `socket` (no token).
    pub fn unix(socket: impl AsRef<Path>) -> Self {
        Self::with(Transport::unix(socket.as_ref().to_path_buf()))
    }

    /// A client for the TCP listener described by `info`. The token is not
    /// refreshed on 401; use [`Client::tcp_from_file`] for that.
    pub fn tcp(info: &DaemonInfo) -> Result<Self, ClientError> {
        Self::tcp_with(&info.http, &info.token, None)
    }

    /// A TCP client from explicit parts. With `refresh_from`, a 401 makes the
    /// client re-read that daemon.json once and retry.
    pub fn tcp_with(
        base_url: &str,
        token: &str,
        refresh_from: Option<PathBuf>,
    ) -> Result<Self, ClientError> {
        Ok(Self::with(Transport::tcp(base_url, token, refresh_from)?))
    }

    /// A TCP client from `daemon.json`, re-reading it once after a 401.
    pub fn tcp_from_file(daemon_json: impl AsRef<Path>) -> Result<Self, ClientError> {
        let path = daemon_json.as_ref();
        let info = DaemonInfo::load(path)?;
        Self::tcp_with(&info.http, &info.token, Some(path.to_path_buf()))
    }

    /// The client for `paths` over the chosen transport.
    pub fn from_paths(paths: &Paths, kind: TransportKind) -> Result<Self, ClientError> {
        match kind {
            TransportKind::Unix => Ok(Self::unix(&paths.socket)),
            TransportKind::Tcp => Self::tcp_from_file(&paths.daemon_json),
        }
    }

    fn with(t: Transport) -> Self {
        Self {
            transport: Arc::new(t),
        }
    }

    // ----- plumbing ---------------------------------------------------------

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&(impl Serialize + ?Sized)>,
    ) -> Result<T, ClientError> {
        let payload = body.map(serde_json::to_vec).transpose()?;
        let (resp, _guard) = self
            .transport
            .send(method, path, payload.as_deref())
            .await?;
        let status = resp.status().as_u16();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| ClientError::Http(e.to_string()))?
            .to_bytes();
        if status >= 300 {
            return Err(api_error(status, &bytes));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.request::<T>(Method::GET, path, None::<&()>).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, ClientError> {
        self.request(Method::POST, path, body).await
    }

    async fn stream(
        &self,
        path: &str,
        until_terminal_job_state: bool,
    ) -> Result<EventStream, ClientError> {
        let (resp, guard) = self.transport.send(Method::GET, path, None).await?;
        let status = resp.status().as_u16();
        if status >= 300 {
            let bytes = resp
                .into_body()
                .collect()
                .await
                .map_err(|e| ClientError::Http(e.to_string()))?
                .to_bytes();
            return Err(api_error(status, &bytes));
        }
        Ok(event_stream(
            resp.into_body(),
            guard,
            until_terminal_job_state,
        ))
    }

    // ----- daemon -----------------------------------------------------------

    /// `GET /v1/health`.
    pub async fn health(&self) -> Result<HealthInfo, ClientError> {
        self.get("/v1/health").await
    }

    /// `Some(info)` when a daemon answers within one second.
    pub async fn is_running(&self) -> Option<HealthInfo> {
        tokio::time::timeout(Duration::from_secs(1), self.health())
            .await
            .ok()?
            .ok()
    }

    /// `POST /v1/shutdown`: the daemon exits right after answering.
    pub async fn shutdown(&self) -> Result<ShutdownResult, ClientError> {
        self.post("/v1/shutdown", None::<&()>).await
    }

    /// `POST /v1/gc`.
    pub async fn gc(&self) -> Result<GcResult, ClientError> {
        self.post("/v1/gc", None::<&()>).await
    }

    // ----- projects ---------------------------------------------------------

    /// `GET /v1/projects`.
    pub async fn projects(&self) -> Result<ProjectsResult, ClientError> {
        self.get("/v1/projects").await
    }

    /// `POST /v1/projects` with an absolute directory holding `rocket.yaml`.
    pub async fn add_project(&self, path: &str) -> Result<ProjectRef, ClientError> {
        let body = rocket_domain::api::AddProjectRequest { path: path.into() };
        self.post("/v1/projects", Some(&body)).await
    }

    /// `DELETE /v1/projects/{name}`.
    pub async fn remove_project(&self, name: &str) -> Result<RemovedProject, ClientError> {
        let path = format!("/v1/projects/{}", enc(name));
        self.request(Method::DELETE, &path, None::<&()>).await
    }

    // ----- services ---------------------------------------------------------

    /// `POST /v1/up`; blocks until every service is healthy or failed.
    pub async fn up(&self, req: &UpRequest) -> Result<UpResult, ClientError> {
        self.post("/v1/up", Some(req)).await
    }

    /// `POST /v1/restart` (stop then start).
    pub async fn restart(&self, req: &UpRequest) -> Result<UpResult, ClientError> {
        self.post("/v1/restart", Some(req)).await
    }

    /// `POST /v1/down`.
    pub async fn down(&self, req: &DownRequest) -> Result<DownResult, ClientError> {
        self.post("/v1/down", Some(req)).await
    }

    /// `GET /v1/ps`. Without `project` (or with `all`) every known run.
    pub async fn ps(&self, project: Option<&str>, all: bool) -> Result<StatusResult, ClientError> {
        let q = query(&[
            ("project", non_empty(project)),
            ("all", some_if(all, "true")),
        ]);
        self.get(&format!("/v1/ps{q}")).await
    }

    /// `GET /v1/status`: the one-shot overview (`None` = all projects).
    pub async fn status(&self, project: Option<&str>) -> Result<Summary, ClientError> {
        let q = query(&[("project", non_empty(project))]);
        self.get(&format!("/v1/status{q}")).await
    }

    /// `GET /v1/ports`.
    pub async fn ports(&self) -> Result<PortsResult, ClientError> {
        self.get("/v1/ports").await
    }

    /// `GET /v1/logs`: the last `tail` lines of a service's output.
    pub async fn logs(
        &self,
        project: &str,
        service: &str,
        tail: u32,
    ) -> Result<LogsResult, ClientError> {
        let q = query(&[
            ("project", Some(project.to_owned())),
            ("service", Some(service.to_owned())),
            ("tail", Some(tail.to_string())),
        ]);
        self.get(&format!("/v1/logs{q}")).await
    }

    // ----- jobs -------------------------------------------------------------

    /// `POST /v1/jobs`; returns at once with a `running` job. A refused deploy
    /// is `ErrorCode::ConfirmationRequired` (HTTP 428).
    pub async fn start_job(&self, req: &JobRequest) -> Result<Job, ClientError> {
        self.post("/v1/jobs", Some(req)).await
    }

    /// `GET /v1/jobs`, newest first.
    pub async fn jobs(&self, q: &JobsQuery) -> Result<JobsResult, ClientError> {
        let qs = query(&[
            ("project", non_empty(q.project.as_deref())),
            ("all", some_if(q.all, "true")),
            ("limit", q.limit.filter(|l| *l > 0).map(|l| l.to_string())),
        ]);
        self.get(&format!("/v1/jobs{qs}")).await
    }

    /// `GET /v1/jobs/{id}`; with `wait`, blocks until the job is terminal.
    pub async fn job(&self, id: &str, wait: bool) -> Result<Job, ClientError> {
        let q = query(&[("wait", some_if(wait, "true"))]);
        self.get(&format!("/v1/jobs/{}{q}", enc(id))).await
    }

    /// `POST /v1/jobs/{id}/cancel`; no-op when the job already finished.
    pub async fn cancel_job(&self, id: &str) -> Result<Job, ClientError> {
        self.post(&format!("/v1/jobs/{}/cancel", enc(id)), None::<&()>)
            .await
    }

    /// `GET /v1/jobs/{id}/logs`: the last `tail` lines.
    pub async fn job_logs(&self, id: &str, tail: u32) -> Result<JobLogsResult, ClientError> {
        let q = query(&[("tail", Some(tail.to_string()))]);
        self.get(&format!("/v1/jobs/{}/logs{q}", enc(id))).await
    }

    // ----- streams ----------------------------------------------------------

    /// `GET /v1/events`. The stream ends when the daemon closes it; events can
    /// be dropped for slow consumers, so re-sync snapshots after a reconnect.
    pub async fn events(&self, filter: &EventFilter) -> Result<EventStream, ClientError> {
        let q = query(&[
            ("project", non_empty(filter.project.as_deref())),
            ("service", non_empty(filter.service.as_deref())),
            ("job", non_empty(filter.job.as_deref())),
            (
                "types",
                (!filter.types.is_empty()).then(|| filter.types.join(",")),
            ),
        ]);
        self.stream(&format!("/v1/events{q}"), false).await
    }

    /// `GET /v1/logs?follow=true`: `log.line` events, tail first then live.
    pub async fn follow_logs(
        &self,
        project: &str,
        service: &str,
        tail: Option<u32>,
    ) -> Result<EventStream, ClientError> {
        let q = query(&[
            ("project", Some(project.to_owned())),
            ("service", Some(service.to_owned())),
            ("tail", tail.map(|t| t.to_string())),
            ("follow", Some("true".into())),
        ]);
        self.stream(&format!("/v1/logs{q}"), false).await
    }

    /// `GET /v1/jobs/{id}/logs?follow=true`: `job.log` events (`tail: None`
    /// replays the whole log), ending after the terminal `job.state`. If the
    /// server closes before sending it the last item is a
    /// [`ClientError::Protocol`] error.
    pub async fn follow_job_logs(
        &self,
        id: &str,
        tail: Option<u32>,
    ) -> Result<EventStream, ClientError> {
        let q = query(&[
            ("follow", Some("true".into())),
            ("tail", tail.map(|t| t.to_string())),
        ]);
        self.stream(&format!("/v1/jobs/{}/logs{q}", enc(id)), true)
            .await
    }
}

fn api_error(status: u16, body: &[u8]) -> ClientError {
    match serde_json::from_slice::<ErrorBody>(body) {
        Ok(b) if !b.error.is_empty() => ClientError::Api {
            status,
            code: if b.code.is_empty() {
                ErrorCode::from_status(status)
            } else {
                ErrorCode::from_wire(&b.code)
            },
            message: b.error,
        },
        _ => ClientError::Api {
            status,
            code: ErrorCode::Other(String::new()),
            message: format!(
                "daemon returned {status}: {}",
                String::from_utf8_lossy(body).trim()
            ),
        },
    }
}
