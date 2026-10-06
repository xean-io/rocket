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
mod menu;
mod state;
mod supervisor;
#[cfg(desktop)]
mod tray;
#[cfg(desktop)]
mod update;
mod window;

use state::AppState;
use tauri::{Manager, RunEvent, WindowEvent};

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
        // The smoke hook must neither hand off to a running Rocket nor touch
        // its saved window state.
        if !update::smoke_requested() {
            builder = builder
                .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                    window::show_main(app);
                }))
                .plugin(tauri_plugin_window_state::Builder::default().build());
        }
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let paths = rocket_client::Paths::resolve()?;
            app.manage(AppState::new(paths)?);
            #[cfg(desktop)]
            {
                app.manage(update::UpdateState::default());
                // Test-only: check + download + verify the signature, then exit.
                if update::smoke_requested() {
                    update::smoke(app.handle().clone());
                }
                app.set_menu(menu::build(app.handle())?)?;
                tray::init(app.handle())?;
            }
            tauri::async_runtime::spawn(supervisor::run(app.handle().clone()));
            Ok(())
        })
        .on_menu_event(|app, event| menu::handle_event(app, event.id().as_ref()))
        .on_window_event(|window, event| match event {
            // Closing the window keeps the app alive in the tray.
            WindowEvent::CloseRequested { api, .. } if window.label() == window::MAIN => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Destroyed => window.state::<AppState>().stop_all_follows(),
            _ => {}
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
            commands::initial_route,
            #[cfg(desktop)]
            update::updater_env,
            #[cfg(desktop)]
            update::check_update,
            #[cfg(desktop)]
            update::install_update,
            #[cfg(desktop)]
            update::daemon_stale,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Rocket desktop app")
        .run(|app, event| {
            // Clicking the Dock icon with every window hidden brings it back.
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                window::show_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
