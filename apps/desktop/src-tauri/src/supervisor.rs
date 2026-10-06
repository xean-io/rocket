//! Keeps one `/v1/events` stream open and mirrors it to the frontend.

use crate::backoff::Backoff;
use crate::dto::ConnectionStatus;
use crate::state::AppState;
use futures_util::StreamExt;
use rocket_client::{ClientError, EnsureOptions, EventFilter, ensure_daemon};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// One event of the daemon stream (payload: `Event`).
pub const EVENT: &str = "rocket://event";
/// Connection changes (payload: `ConnectionStatus`).
pub const CONNECTION: &str = "rocket://connection";
/// Snapshots must be reloaded: the stream (re)opened and events may have been
/// missed in between (payload: none).
pub const RESYNC: &str = "rocket://resync";

pub async fn set_status(app: &AppHandle, state: &AppState, status: ConnectionStatus) {
    if state.set_status(status.clone()).await {
        match &status {
            ConnectionStatus::Online { version, pid } => {
                tracing::info!("online: connected to rocketd {version} (pid {pid})");
            }
            ConnectionStatus::Offline { reason } => tracing::warn!("offline: {reason}"),
            ConnectionStatus::Connecting => tracing::info!("connecting to rocketd"),
        }
        let _ = app.emit(CONNECTION, status);
    }
}

/// Runs for the life of the app.
pub async fn run(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut backoff = Backoff::new();
    loop {
        set_status(&app, &state, ConnectionStatus::Connecting).await;
        let reason = match session(&app, &state, &mut backoff).await {
            Ok(()) => "the event stream closed".to_owned(),
            Err(reason) => reason,
        };
        set_status(&app, &state, ConnectionStatus::Offline { reason }).await;
        let delay: Duration = backoff.next_delay();
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = state.reconnect.notified() => backoff.reset(),
        }
    }
}

/// One connection: ensure the daemon, open the stream, forward events until
/// it ends. `Ok` means the stream ended or a reconnect was requested.
async fn session(app: &AppHandle, state: &AppState, backoff: &mut Backoff) -> Result<(), String> {
    let client = {
        let _launch = state.ensure_lock.lock().await;
        ensure_daemon(&EnsureOptions::new(state.paths.clone()))
            .await
            .map_err(|e| e.to_string())?
    };
    state.set_client(client.clone()).await;
    let health = client.health().await.map_err(|e| e.to_string())?;
    let mut stream = client
        .events(&EventFilter::default())
        .await
        .map_err(|e| e.to_string())?;

    backoff.reset();
    set_status(
        app,
        state,
        ConnectionStatus::Online {
            version: health.version,
            pid: health.pid,
        },
    )
    .await;
    // The stream is open, so nothing after this point is missed: tell the UI
    // to reload its snapshots (events can also be dropped for slow readers).
    let _ = app.emit(RESYNC, ());

    loop {
        tokio::select! {
            item = stream.next() => match item {
                None => return Ok(()),
                Some(Ok(event)) => match event.event() {
                    Some(payload) => { let _ = app.emit(EVENT, payload); }
                    None => tracing::debug!("ignoring an unknown daemon event"),
                },
                Some(Err(e @ ClientError::Decode(_))) => tracing::warn!("skipping event: {e}"),
                Some(Err(e)) => return Err(e.to_string()),
            },
            () = state.reconnect.notified() => return Ok(()),
        }
    }
}
