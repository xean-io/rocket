//! Auto-updater: the Rust side owns check, download, signature verification,
//! install and relaunch; the frontend only drives commands and renders state.
//!
//! Updates are signed with the Tauri updater key; the public key lives in
//! `tauri.conf.json` (`plugins.updater.pubkey`) and is always enforced, even
//! when [`ENDPOINT_ENV`] redirects the manifest lookup for testing.

use crate::error::CommandError;
use crate::state::AppState;
use rocket_client::DaemonInfo;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use time::format_description::well_known::Rfc3339;
use ts_rs::TS;

/// Overrides the manifest endpoint (testing only; verification still applies).
pub const ENDPOINT_ENV: &str = "ROCKET_UPDATER_ENDPOINT";

/// Result of one update check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct UpdateStatus {
    pub available: bool,
    /// The announced version; `null` when up to date.
    pub version: Option<String>,
    pub current_version: String,
    pub notes: Option<String>,
    /// RFC3339 publish date, when the manifest carries one.
    pub date: Option<String>,
}

/// Progress of `install_update`, streamed over a channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum InstallEvent {
    Started { content_length: Option<u64> },
    Progress { chunk: usize },
    Finished,
}

/// What the frontend needs before it has checked anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct UpdaterEnv {
    pub current_version: String,
    /// `false` in debug builds: no automatic checks while developing.
    pub auto_check: bool,
}

/// The running daemon is older than the bundled CLI it was started from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct StaleDaemon {
    pub running: String,
    pub bundled: String,
}

/// Parses [`ENDPOINT_ENV`]: unset or blank means "use the configured endpoint".
/// Only `http(s)` URLs are accepted; the updater itself still refuses plain
/// `http` in release builds unless the build config opts in.
pub fn endpoint_override(raw: Option<&str>) -> Result<Option<tauri::Url>, String> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let url = tauri::Url::parse(raw).map_err(|e| format!("{ENDPOINT_ENV} is not a URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(Some(url)),
        other => Err(format!(
            "{ENDPOINT_ENV} must be an http(s) URL, not {other}:"
        )),
    }
}

/// `true` when the daemon runs another version than the bundled CLI AND was
/// started from that bundled binary (a daemon from `brew` or `PATH` is left alone).
pub fn stale_daemon(
    running: &str,
    bundled_version: &str,
    daemon_bin: &str,
    bundled_bin: &Path,
) -> bool {
    let (running, bundled) = (running.trim(), bundled_version.trim());
    if running.is_empty() || bundled.is_empty() || running == bundled || daemon_bin.is_empty() {
        return false;
    }
    let daemon_bin = Path::new(daemon_bin);
    daemon_bin == bundled_bin
        || matches!(
            (daemon_bin.canonicalize(), bundled_bin.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

/// Extracts the version from `rocket --version` output (`rocket version 0.1.1`).
pub fn parse_cli_version(output: &str) -> Option<String> {
    output.split_whitespace().last().map(str::to_owned)
}

/// Allows one check/install at a time.
#[derive(Default)]
pub struct UpdateState {
    busy: AtomicBool,
    pending: Mutex<Option<Update>>,
}

pub struct BusyGuard<'a>(&'a AtomicBool);

impl UpdateState {
    pub fn begin(&self) -> Result<BusyGuard<'_>, CommandError> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(CommandError::local(
                "busy",
                "an update check or install is already running",
            ));
        }
        Ok(BusyGuard(&self.busy))
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Option<Update>> {
        self.pending.lock().expect("pending update lock")
    }
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn updater_error(e: &tauri_plugin_updater::Error) -> CommandError {
    use tauri_plugin_updater::Error;
    let code = match e {
        Error::Reqwest(_) | Error::Network(_) => "network",
        Error::SignatureUtf8(_) | Error::Minisign(_) | Error::Base64(_) => "signature",
        _ => "updater",
    };
    CommandError::local(code, e.to_string())
}

fn build_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, CommandError> {
    let mut builder = app.updater_builder();
    let raw = std::env::var(ENDPOINT_ENV).ok();
    if let Some(url) =
        endpoint_override(raw.as_deref()).map_err(|m| CommandError::local("endpoint", m))?
    {
        builder = builder
            .endpoints(vec![url])
            .map_err(|e| updater_error(&e))?;
    }
    builder.build().map_err(|e| updater_error(&e))
}

fn status_of(update: Option<&Update>, current: &str) -> UpdateStatus {
    match update {
        Some(u) => UpdateStatus {
            available: true,
            version: Some(u.version.clone()),
            current_version: u.current_version.clone(),
            notes: u.body.clone(),
            date: u.date.and_then(|d| d.format(&Rfc3339).ok()),
        },
        None => UpdateStatus {
            available: false,
            version: None,
            current_version: current.to_owned(),
            notes: None,
            date: None,
        },
    }
}

async fn check(app: &AppHandle, state: &UpdateState) -> Result<Option<Update>, CommandError> {
    let _busy = state.begin()?;
    let found = build_updater(app)?
        .check()
        .await
        .map_err(|e| updater_error(&e))?;
    *state.pending() = found.clone();
    Ok(found)
}

#[tauri::command]
pub fn updater_env(app: AppHandle) -> UpdaterEnv {
    UpdaterEnv {
        current_version: app.package_info().version.to_string(),
        auto_check: !cfg!(debug_assertions),
    }
}

/// Looks for a newer signed release and remembers it for [`install_update`].
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateStatus, CommandError> {
    let found = check(&app, &state).await?;
    Ok(status_of(
        found.as_ref(),
        &app.package_info().version.to_string(),
    ))
}

/// Downloads and verifies the update found by the last check, installs it and
/// relaunches. Never returns on success.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
    on_event: Channel<InstallEvent>,
) -> Result<(), CommandError> {
    let _busy = state.begin()?;
    let Some(update) = state.pending().clone() else {
        return Err(CommandError::local(
            "no_update",
            "no update was found; check for updates first",
        ));
    };
    let mut started = false;
    let bytes = update
        .download(
            |chunk, total| {
                if !started {
                    started = true;
                    let _ = on_event.send(InstallEvent::Started {
                        content_length: total,
                    });
                }
                let _ = on_event.send(InstallEvent::Progress { chunk });
            },
            || {
                let _ = on_event.send(InstallEvent::Finished);
            },
        )
        .await
        .map_err(|e| updater_error(&e))?;
    update.install(bytes).map_err(|e| updater_error(&e))?;
    state.pending().take();
    app.restart()
}

/// `true` when the test-only smoke hook is requested (`ROCKET_UPDATER_SMOKE=download`).
pub fn smoke_requested() -> bool {
    std::env::var("ROCKET_UPDATER_SMOKE").is_ok_and(|m| m == "download")
}

/// Test-only hook (`ROCKET_UPDATER_SMOKE=download`): check, download and verify
/// the signature, log the outcome and exit. It never installs.
pub fn smoke(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<UpdateState>();
        let result = async {
            let update = check(&app, &state)
                .await?
                .ok_or_else(|| CommandError::local("no_update", "no update available"))?;
            tracing::info!(
                "smoke: update {} found (current {})",
                update.version,
                update.current_version
            );
            let bytes = update
                .download(|_, _| {}, || {})
                .await
                .map_err(|e| updater_error(&e))?;
            Ok::<_, CommandError>((update.version.clone(), bytes.len()))
        }
        .await;
        match result {
            Ok((version, len)) => {
                tracing::info!("smoke: signature verified for {version} ({len} bytes)");
                app.exit(0);
            }
            Err(e) => {
                tracing::error!("smoke: rejected [{}]: {}", e.code, e.message);
                app.exit(1);
            }
        }
    });
}

/// Reports a daemon that survived an update and still runs the old CLI.
#[tauri::command]
pub async fn daemon_stale(state: State<'_, AppState>) -> Result<Option<StaleDaemon>, CommandError> {
    let Some(bundled_bin) = crate::state::bundled_rocket_bin() else {
        return Ok(None);
    };
    let Some(health) = state.client().await.is_running().await else {
        return Ok(None);
    };
    let Ok(info) = DaemonInfo::load(&state.paths.daemon_json) else {
        return Ok(None);
    };
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(&bundled_bin)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let Some(bundled) = out
        .ok()
        .and_then(Result::ok)
        .filter(|o| o.status.success())
        .and_then(|o| parse_cli_version(&String::from_utf8_lossy(&o.stdout)))
    else {
        return Ok(None);
    };
    if !stale_daemon(&health.version, &bundled, &info.rocket_bin, &bundled_bin) {
        return Ok(None);
    }
    Ok(Some(StaleDaemon {
        running: health.version,
        bundled,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_override_ignores_unset_and_blank() {
        assert_eq!(endpoint_override(None), Ok(None));
        assert_eq!(endpoint_override(Some("")), Ok(None));
        assert_eq!(endpoint_override(Some("  \n")), Ok(None));
    }

    #[test]
    fn endpoint_override_parses_a_url_and_rejects_garbage() {
        let u = endpoint_override(Some(" https://example.test/latest.json ")).unwrap();
        assert_eq!(u.unwrap().as_str(), "https://example.test/latest.json");
        let err = endpoint_override(Some("not a url")).unwrap_err();
        assert!(err.contains(ENDPOINT_ENV), "{err}");
        assert!(endpoint_override(Some("file:///etc/passwd")).is_err());
    }

    #[test]
    fn status_serializes_with_snake_case_and_nulls() {
        let up_to_date = UpdateStatus {
            available: false,
            version: None,
            current_version: "0.1.1".into(),
            notes: None,
            date: None,
        };
        assert_eq!(
            serde_json::to_value(&up_to_date).unwrap(),
            serde_json::json!({
                "available": false, "version": null, "current_version": "0.1.1",
                "notes": null, "date": null
            })
        );
    }

    #[test]
    fn install_events_are_tagged_by_event() {
        let j = |e: InstallEvent| serde_json::to_value(e).unwrap();
        assert_eq!(
            j(InstallEvent::Started {
                content_length: Some(10)
            }),
            serde_json::json!({"event": "started", "content_length": 10})
        );
        assert_eq!(
            j(InstallEvent::Started {
                content_length: None
            }),
            serde_json::json!({"event": "started", "content_length": null})
        );
        assert_eq!(
            j(InstallEvent::Progress { chunk: 4 }),
            serde_json::json!({"event": "progress", "chunk": 4})
        );
        assert_eq!(
            j(InstallEvent::Finished),
            serde_json::json!({"event": "finished"})
        );
    }

    #[test]
    fn only_one_operation_runs_at_a_time() {
        let state = UpdateState::default();
        let first = state.begin().unwrap();
        let err = state.begin().err().unwrap();
        assert_eq!(err.code, "busy");
        drop(first);
        assert!(state.begin().is_ok());
    }

    #[test]
    fn parses_the_cli_version_line() {
        assert_eq!(
            parse_cli_version("rocket version 0.1.99\n").as_deref(),
            Some("0.1.99")
        );
        assert_eq!(parse_cli_version("").as_deref(), None);
    }

    #[test]
    fn stale_only_when_versions_differ_and_the_daemon_runs_the_bundled_binary() {
        let bundled = Path::new("/Applications/Rocket.app/Contents/MacOS/rocket");
        let same = "/Applications/Rocket.app/Contents/MacOS/rocket";
        assert!(stale_daemon("0.1.1", "0.1.2", same, bundled));
        assert!(!stale_daemon("0.1.2", "0.1.2", same, bundled));
        // A daemon started from brew or PATH is not ours to flag.
        assert!(!stale_daemon(
            "0.1.1",
            "0.1.2",
            "/opt/homebrew/bin/rocket",
            bundled
        ));
        assert!(!stale_daemon("0.1.1", "0.1.2", "", bundled));
        // Surrounding whitespace in versions never causes a false alarm.
        assert!(!stale_daemon("0.1.2 ", " 0.1.2", same, bundled));
    }
}
