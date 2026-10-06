//! Main-window helpers shared by the tray, the menu and the single-instance
//! handler. The window is hidden, not destroyed, so the app lives in the tray.

use tauri::{AppHandle, Manager};

pub const MAIN: &str = "main";

/// Shows, unminimizes and focuses the main window.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
