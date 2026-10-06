//! Native application menu (accelerators follow the original macOS app's command set).
//!
//! Custom items carry a stable action id (`services.up`, `view.ports`, ...).
//! Choosing one emits [`EVENT`] with that id as payload; the frontend routes it
//! through the same handlers the old webview shortcuts used. Predefined items
//! (copy, paste, quit, ...) are handled natively and never reach the frontend.

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Wry};

/// Payload: the action id (a string, see [`MenuAction::id`]).
pub const EVENT: &str = "rocket://menu";

/// Everything a custom menu item can ask the frontend to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    AddProject,
    Settings,
    CheckUpdates,
    Up,
    Restart,
    Stop,
    Logs,
    Refresh,
    CollectGarbage,
    ShowProjects,
    ShowPorts,
    ShowJobs,
    ShowOwners,
}

impl MenuAction {
    pub const ALL: [Self; 13] = [
        Self::AddProject,
        Self::Settings,
        Self::CheckUpdates,
        Self::Up,
        Self::Restart,
        Self::Stop,
        Self::Logs,
        Self::Refresh,
        Self::CollectGarbage,
        Self::ShowProjects,
        Self::ShowPorts,
        Self::ShowJobs,
        Self::ShowOwners,
    ];

    /// The id used as menu item id and as the `rocket://menu` payload.
    pub fn id(self) -> &'static str {
        match self {
            Self::AddProject => "app.add_project",
            Self::Settings => "app.settings",
            Self::CheckUpdates => "app.check_updates",
            Self::Up => "services.up",
            Self::Restart => "services.restart",
            Self::Stop => "services.stop",
            Self::Logs => "services.logs",
            Self::Refresh => "services.refresh",
            Self::CollectGarbage => "services.gc",
            Self::ShowProjects => "view.projects",
            Self::ShowPorts => "view.ports",
            Self::ShowJobs => "view.jobs",
            Self::ShowOwners => "view.owners",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AddProject => "Add Project\u{2026}",
            Self::Settings => "Settings\u{2026}",
            Self::CheckUpdates => "Check for Updates\u{2026}",
            Self::Up => "Up",
            Self::Restart => "Restart",
            Self::Stop => "Stop",
            Self::Logs => "Toggle Logs",
            Self::Refresh => "Refresh",
            Self::CollectGarbage => "Collect Garbage",
            Self::ShowProjects => "Projects",
            Self::ShowPorts => "Ports",
            Self::ShowJobs => "Jobs",
            Self::ShowOwners => "Owners",
        }
    }

    /// Accelerator in muda syntax (`CmdOrCtrl` is Cmd on macOS).
    pub fn accelerator(self) -> Option<&'static str> {
        match self {
            Self::AddProject => Some("CmdOrCtrl+O"),
            Self::Settings => Some("CmdOrCtrl+,"),
            Self::Up => Some("CmdOrCtrl+Shift+U"),
            Self::Restart => Some("CmdOrCtrl+R"),
            Self::Stop => Some("CmdOrCtrl+."),
            Self::Logs => Some("CmdOrCtrl+L"),
            Self::Refresh => Some("CmdOrCtrl+Shift+R"),
            Self::CollectGarbage | Self::CheckUpdates => None,
            Self::ShowProjects => Some("CmdOrCtrl+1"),
            Self::ShowPorts => Some("CmdOrCtrl+2"),
            Self::ShowJobs => Some("CmdOrCtrl+3"),
            Self::ShowOwners => Some("CmdOrCtrl+4"),
        }
    }
}

fn item(app: &AppHandle, action: MenuAction) -> tauri::Result<tauri::menu::MenuItem<Wry>> {
    let mut b = MenuItemBuilder::with_id(action.id(), action.label());
    if let Some(acc) = action.accelerator() {
        b = b.accelerator(acc);
    }
    b.build(app)
}

/// Builds the whole menu bar: App (macOS), File, Edit, Services, View, Window.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let add_project = item(app, MenuAction::AddProject)?;
    let settings = item(app, MenuAction::Settings)?;
    let check_updates = item(app, MenuAction::CheckUpdates)?;

    let mut bar = MenuBuilder::new(app);

    #[cfg(target_os = "macos")]
    {
        let name = app.package_info().name.clone();
        let app_menu = SubmenuBuilder::new(app, name)
            .about(None)
            .item(&check_updates)
            .separator()
            .item(&settings)
            .separator()
            .services()
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;
        bar = bar.item(&app_menu);
    }

    // Settings live in the app menu on macOS and in File elsewhere.
    let file = SubmenuBuilder::new(app, "File").item(&add_project);
    let file = if cfg!(target_os = "macos") {
        file
    } else {
        file.item(&settings).item(&check_updates)
    };
    let file = file.separator().close_window().build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let services = SubmenuBuilder::new(app, "Services")
        .item(&item(app, MenuAction::Up)?)
        .item(&item(app, MenuAction::Restart)?)
        .item(&item(app, MenuAction::Stop)?)
        .separator()
        .item(&item(app, MenuAction::Logs)?)
        .item(&item(app, MenuAction::Refresh)?)
        .separator()
        .item(&item(app, MenuAction::CollectGarbage)?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item(app, MenuAction::ShowProjects)?)
        .item(&item(app, MenuAction::ShowPorts)?)
        .item(&item(app, MenuAction::ShowJobs)?)
        .item(&item(app, MenuAction::ShowOwners)?)
        .separator()
        .fullscreen()
        .build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;

    bar.item(&file)
        .item(&edit)
        .item(&services)
        .item(&view)
        .item(&window)
        .build()
}

/// Forwards a custom menu item to the frontend; unknown ids (predefined items,
/// tray items) are ignored.
pub fn handle_event(app: &AppHandle, id: &str) {
    if let Some(action) = MenuAction::from_id(id) {
        crate::window::show_main(app);
        let _ = app.emit(EVENT, action.id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_round_trip() {
        for a in MenuAction::ALL {
            assert_eq!(MenuAction::from_id(a.id()), Some(a), "{a:?}");
        }
    }

    #[test]
    fn check_updates_has_a_stable_id_and_label() {
        assert_eq!(MenuAction::CheckUpdates.id(), "app.check_updates");
        assert_eq!(
            MenuAction::CheckUpdates.label(),
            "Check for Updates\u{2026}"
        );
    }

    #[test]
    fn unknown_and_predefined_ids_are_ignored() {
        assert_eq!(MenuAction::from_id("copy"), None);
        assert_eq!(MenuAction::from_id("tray.open"), None);
        assert_eq!(MenuAction::from_id(""), None);
    }

    #[test]
    fn ids_and_accelerators_are_unique() {
        let ids: HashSet<_> = MenuAction::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids.len(), MenuAction::ALL.len());
        let accels: Vec<_> = MenuAction::ALL
            .iter()
            .filter_map(|a| a.accelerator())
            .collect();
        let unique: HashSet<_> = accels.iter().collect();
        assert_eq!(unique.len(), accels.len());
    }

    #[test]
    fn accelerators_mirror_the_original_commands() {
        assert_eq!(MenuAction::Up.accelerator(), Some("CmdOrCtrl+Shift+U"));
        assert_eq!(MenuAction::Restart.accelerator(), Some("CmdOrCtrl+R"));
        assert_eq!(MenuAction::Stop.accelerator(), Some("CmdOrCtrl+."));
        assert_eq!(MenuAction::Logs.accelerator(), Some("CmdOrCtrl+L"));
        assert_eq!(MenuAction::Refresh.accelerator(), Some("CmdOrCtrl+Shift+R"));
        assert_eq!(MenuAction::ShowOwners.accelerator(), Some("CmdOrCtrl+4"));
        assert_eq!(MenuAction::CollectGarbage.accelerator(), None);
        assert_eq!(MenuAction::CheckUpdates.accelerator(), None);
    }
}
