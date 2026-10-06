//! Thin, typed Tauri commands over the daemon API. Every command resolves
//! the current client, forwards one call and maps errors to [`CommandError`].
//! Actions started from the app are owned by `user`, as in the original macOS app.

use crate::dto::{ConnectionStatus, DaemonDetails, FollowMessage};
use crate::error::CommandError;
use crate::follow;
use crate::state::AppState;
use rocket_client::{DaemonInfo, JobsQuery, ensure_daemon, find_rocket_bin_with};
use rocket_domain::api::{
    DownRequest, DownResult, GcResult, HealthInfo, JobLogsResult, JobRequest, JobsResult,
    LogsResult, PortsResult, ProjectsResult, RemovedProject, StatusResult, Summary, UpRequest,
    UpResult,
};
use rocket_domain::{DEFAULT_OWNER, Job, ProjectRef};
use std::time::Duration;
use tauri::State;
use tauri::ipc::Channel;

type CmdResult<T> = Result<T, CommandError>;

fn owned(owner: &mut String) {
    if owner.is_empty() {
        DEFAULT_OWNER.clone_into(owner);
    }
}

#[tauri::command]
pub async fn connection_status(state: State<'_, AppState>) -> CmdResult<ConnectionStatus> {
    Ok(state.status().await)
}

#[tauri::command]
pub async fn health(state: State<'_, AppState>) -> CmdResult<HealthInfo> {
    Ok(state.client().await.health().await?)
}

#[tauri::command]
pub async fn projects(state: State<'_, AppState>) -> CmdResult<ProjectsResult> {
    Ok(state.client().await.projects().await?)
}

#[tauri::command]
pub async fn add_project(state: State<'_, AppState>, path: String) -> CmdResult<ProjectRef> {
    Ok(state.client().await.add_project(&path).await?)
}

#[tauri::command]
pub async fn remove_project(state: State<'_, AppState>, name: String) -> CmdResult<RemovedProject> {
    Ok(state.client().await.remove_project(&name).await?)
}

#[tauri::command]
pub async fn up(state: State<'_, AppState>, mut request: UpRequest) -> CmdResult<UpResult> {
    owned(&mut request.owner);
    Ok(state.client().await.up(&request).await?)
}

#[tauri::command]
pub async fn restart(state: State<'_, AppState>, mut request: UpRequest) -> CmdResult<UpResult> {
    owned(&mut request.owner);
    Ok(state.client().await.restart(&request).await?)
}

/// `owner` on a down request is a filter ("only runs started by"), so it is
/// forwarded untouched.
#[tauri::command]
pub async fn down(state: State<'_, AppState>, request: DownRequest) -> CmdResult<DownResult> {
    Ok(state.client().await.down(&request).await?)
}

#[tauri::command]
pub async fn ps(
    state: State<'_, AppState>,
    project: Option<String>,
    all: bool,
) -> CmdResult<StatusResult> {
    Ok(state.client().await.ps(project.as_deref(), all).await?)
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>, project: Option<String>) -> CmdResult<Summary> {
    Ok(state.client().await.status(project.as_deref()).await?)
}

#[tauri::command]
pub async fn ports(state: State<'_, AppState>) -> CmdResult<PortsResult> {
    Ok(state.client().await.ports().await?)
}

#[tauri::command]
pub async fn logs(
    state: State<'_, AppState>,
    project: String,
    service: String,
    tail: u32,
) -> CmdResult<LogsResult> {
    Ok(state.client().await.logs(&project, &service, tail).await?)
}

#[tauri::command]
pub async fn gc(state: State<'_, AppState>) -> CmdResult<GcResult> {
    Ok(state.client().await.gc().await?)
}

#[tauri::command]
pub async fn jobs(
    state: State<'_, AppState>,
    project: Option<String>,
    all: bool,
    limit: Option<u32>,
) -> CmdResult<JobsResult> {
    let query = JobsQuery {
        project,
        all,
        limit,
    };
    Ok(state.client().await.jobs(&query).await?)
}

#[tauri::command]
pub async fn job(state: State<'_, AppState>, id: String, wait: bool) -> CmdResult<Job> {
    Ok(state.client().await.job(&id, wait).await?)
}

/// A deploy to a `confirm: true` env fails with `confirmation_required`
/// (status 428) until `request.yes` is set.
#[tauri::command]
pub async fn start_job(state: State<'_, AppState>, mut request: JobRequest) -> CmdResult<Job> {
    owned(&mut request.owner);
    Ok(state.client().await.start_job(&request).await?)
}

#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, id: String) -> CmdResult<Job> {
    Ok(state.client().await.cancel_job(&id).await?)
}

#[tauri::command]
pub async fn job_logs(
    state: State<'_, AppState>,
    id: String,
    tail: u32,
) -> CmdResult<JobLogsResult> {
    Ok(state.client().await.job_logs(&id, tail).await?)
}

/// Problems with `daemon.json` worth showing in Settings: the file holds the
/// TCP token, so anything looser than 0600 is reported (unix only).
fn config_warnings(daemon_json: &std::path::Path) -> Vec<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(daemon_json) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return vec![format!(
                    "{} has mode {mode:04o}; expected 0600 (the token is readable by others)",
                    daemon_json.display()
                )];
            }
        }
    }
    let _ = daemon_json;
    Vec::new()
}

/// Debug builds only: lets visual checks open the app on a given route
/// (`ROCKET_INITIAL_ROUTE=/jobs`) without simulating keystrokes.
#[tauri::command]
pub fn initial_route() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("ROCKET_INITIAL_ROUTE")
            .ok()
            .filter(|r| r.starts_with('/'))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

async fn details(state: &AppState) -> DaemonDetails {
    let health = state.client().await.is_running().await;
    let info = DaemonInfo::load(&state.paths.daemon_json).ok();
    let p = &state.paths;
    let s = |path: &std::path::Path| path.display().to_string();
    DaemonDetails {
        running: health.is_some(),
        home: s(&p.home),
        socket: s(&p.socket),
        db: s(&p.db),
        logs: s(&p.logs),
        daemon_log: s(&p.daemon_log),
        daemon_json: s(&p.daemon_json),
        rocket_bin: find_rocket_bin_with(
            info.as_ref(),
            crate::state::bundled_rocket_bin().as_deref(),
        )
        .map(|b| b.display().to_string()),
        health,
        warnings: config_warnings(&p.daemon_json),
    }
}

#[tauri::command]
pub async fn daemon_info(state: State<'_, AppState>) -> CmdResult<DaemonDetails> {
    Ok(details(&state).await)
}

/// Shuts the daemon down, starts a fresh one and reconnects the stream.
#[tauri::command]
pub async fn restart_daemon(state: State<'_, AppState>) -> CmdResult<DaemonDetails> {
    {
        let _launch = state.ensure_lock.lock().await;
        let old = state.client().await;
        if old.is_running().await.is_some() {
            old.shutdown().await?;
            for _ in 0..50 {
                if old.is_running().await.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        let fresh = ensure_daemon(&state.ensure_options()).await?;
        state.set_client(fresh).await;
    }
    state.reconnect.notify_one();
    Ok(details(&state).await)
}

/// Skips the backoff wait, or drops the open stream and reopens it.
#[tauri::command]
pub async fn reconnect(state: State<'_, AppState>) -> CmdResult<()> {
    state.reconnect.notify_one();
    Ok(())
}

/// Streams a job's `job.log` events, then its final `job.state`, to
/// `channel`. `tail: None` replays the whole log. Returns the follow id.
#[tauri::command]
pub async fn follow_job_logs(
    state: State<'_, AppState>,
    id: String,
    tail: Option<u32>,
    channel: Channel<FollowMessage>,
) -> CmdResult<String> {
    let stream = state.client().await.follow_job_logs(&id, tail).await?;
    Ok(follow::spawn(&state, stream, channel))
}

/// Streams a service's `log.line` events (tail first, then live).
#[tauri::command]
pub async fn follow_service_logs(
    state: State<'_, AppState>,
    project: String,
    service: String,
    tail: Option<u32>,
    channel: Channel<FollowMessage>,
) -> CmdResult<String> {
    let stream = state
        .client()
        .await
        .follow_logs(&project, &service, tail)
        .await?;
    Ok(follow::spawn(&state, stream, channel))
}

#[tauri::command]
pub async fn stop_follow(state: State<'_, AppState>, follow_id: String) -> CmdResult<()> {
    state.stop_follow(&follow_id);
    Ok(())
}
