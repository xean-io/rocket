//! Rocket desktop shell: a thin Tauri bridge over the rocket daemon API.
//!
//! The backend keeps one `/v1/events` stream open ([`supervisor`]), forwards
//! its events to the frontend and exposes the daemon endpoints as typed
//! commands ([`commands`]). TypeScript types are generated from the Rust DTOs
//! ([`bindings`]).

mod backoff;
#[cfg(test)]
mod bindings;
mod commands;
mod dto;
mod error;
mod follow;
mod state;
mod supervisor;

use state::AppState;
use tauri::{Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rocket_desktop_lib=info".into()),
        )
        .init();

    let mut builder = tauri::Builder::default();
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }))
            .plugin(tauri_plugin_window_state::Builder::default().build());
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let paths = rocket_client::Paths::resolve()?;
            app.manage(AppState::new(paths)?);
            tauri::async_runtime::spawn(supervisor::run(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Destroyed) {
                window.state::<AppState>().stop_all_follows();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::connection_status,
            commands::health,
            commands::projects,
            commands::add_project,
            commands::remove_project,
            commands::up,
            commands::restart,
            commands::down,
            commands::ps,
            commands::status,
            commands::ports,
            commands::logs,
            commands::gc,
            commands::jobs,
            commands::job,
            commands::start_job,
            commands::cancel_job,
            commands::job_logs,
            commands::daemon_info,
            commands::restart_daemon,
            commands::reconnect,
            commands::follow_job_logs,
            commands::follow_service_logs,
            commands::stop_follow,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Rocket desktop app");
}
