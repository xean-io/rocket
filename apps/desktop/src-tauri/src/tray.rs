//! Menu bar / system tray (menu bar content): a template icon, a
//! running-services headline, quick Up/Down per project, Collect Garbage and
//! Quit. The menu is recomputed from `ps --all` + the project registry, debounced,
//! whenever the supervisor sees a state event or the connection changes.

use crate::dto::ConnectionStatus;
use crate::menu::MenuAction;
use crate::state::AppState;
use rocket_domain::api::{DownRequest, UpRequest};
use rocket_domain::{DEFAULT_OWNER, ProjectRef, Run};
use std::collections::BTreeMap;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

pub const TRAY_ID: &str = "main";
/// Payload: the project name to select in the main window.
pub const OPEN_PROJECT_EVENT: &str = "rocket://open-project";
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Menu bar labels stay short (HIG: at most ~30 characters).
const MAX_LABEL: usize = 30;

const ICON: &[u8] = include_bytes!("../icons/tray.png");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLine {
    pub name: String,
    pub active: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TraySummary {
    pub connected: bool,
    pub running: usize,
    pub projects: Vec<ProjectLine>,
}

/// Registered projects plus any project with known runs, sorted by name.
pub fn summarize(connected: bool, projects: &[ProjectRef], runs: &[Run]) -> TraySummary {
    let mut lines: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for p in projects {
        lines.entry(p.name.as_str()).or_default();
    }
    let mut running = 0;
    for r in runs {
        let e = lines.entry(r.project.as_str()).or_default();
        e.1 += 1;
        if r.state.active() {
            e.0 += 1;
            running += 1;
        }
    }
    TraySummary {
        connected,
        running,
        projects: lines
            .into_iter()
            .map(|(name, (active, total))| ProjectLine {
                name: name.to_owned(),
                active,
                total,
            })
            .collect(),
    }
}

/// First (disabled) menu row.
pub fn headline(s: &TraySummary) -> String {
    match (s.connected, s.running) {
        (false, _) => "rocketd offline".to_owned(),
        (true, 0) => "No services running".to_owned(),
        (true, 1) => "1 service running".to_owned(),
        (true, n) => format!("{n} services running"),
    }
}

/// Status-item text next to the icon: the running count, empty when idle.
pub fn title(s: &TraySummary) -> String {
    if s.connected && s.running > 0 {
        s.running.to_string()
    } else {
        String::new()
    }
}

pub fn tooltip(s: &TraySummary) -> String {
    format!("Rocket: {}", headline(s))
}

/// `name  2/3`, with the name cut so the row stays short.
pub fn project_label(line: &ProjectLine) -> String {
    let name = if line.name.chars().count() <= MAX_LABEL {
        line.name.clone()
    } else {
        let cut: String = line.name.chars().take(MAX_LABEL - 3).collect();
        format!("{cut}\u{2026}")
    };
    format!("{name}  {}/{}", line.active, line.total)
}

/// What a tray menu item asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayCommand {
    OpenRocket,
    OpenProject(String),
    Up(String),
    Down(String),
    CollectGarbage,
    CheckUpdates,
    Reconnect,
    Quit,
}

impl TrayCommand {
    pub fn id(&self) -> String {
        match self {
            Self::OpenRocket => "tray.open".to_owned(),
            Self::OpenProject(p) => format!("tray.project:{p}"),
            Self::Up(p) => format!("tray.up:{p}"),
            Self::Down(p) => format!("tray.down:{p}"),
            Self::CollectGarbage => "tray.gc".to_owned(),
            Self::CheckUpdates => "tray.updates".to_owned(),
            Self::Reconnect => "tray.reconnect".to_owned(),
            Self::Quit => "tray.quit".to_owned(),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "tray.open" => return Some(Self::OpenRocket),
            "tray.gc" => return Some(Self::CollectGarbage),
            "tray.updates" => return Some(Self::CheckUpdates),
            "tray.reconnect" => return Some(Self::Reconnect),
            "tray.quit" => return Some(Self::Quit),
            _ => {}
        }
        let (kind, name) = id.split_once(':')?;
        let name = name.to_owned();
        match kind {
            "tray.project" => Some(Self::OpenProject(name)),
            "tray.up" => Some(Self::Up(name)),
            "tray.down" => Some(Self::Down(name)),
            _ => None,
        }
    }
}

fn build_menu(app: &AppHandle, s: &TraySummary) -> tauri::Result<Menu<Wry>> {
    let head = MenuItemBuilder::new(headline(s))
        .enabled(false)
        .build(app)?;
    let mut menu = MenuBuilder::new(app).item(&head).separator();

    if s.connected {
        if s.projects.is_empty() {
            let none = MenuItemBuilder::new("No projects")
                .enabled(false)
                .build(app)?;
            menu = menu.item(&none);
        }
        for line in &s.projects {
            let open = MenuItemBuilder::with_id(
                TrayCommand::OpenProject(line.name.clone()).id(),
                "Show in Rocket",
            )
            .build(app)?;
            let up = MenuItemBuilder::with_id(TrayCommand::Up(line.name.clone()).id(), "Up")
                .build(app)?;
            let down = MenuItemBuilder::with_id(TrayCommand::Down(line.name.clone()).id(), "Down")
                .enabled(line.active > 0)
                .build(app)?;
            let sub = SubmenuBuilder::new(app, project_label(line))
                .item(&open)
                .separator()
                .item(&up)
                .item(&down)
                .build()?;
            menu = menu.item(&sub);
        }
    } else {
        let start =
            MenuItemBuilder::with_id(TrayCommand::Reconnect.id(), "Start rocketd").build(app)?;
        menu = menu.item(&start);
    }

    let open = MenuItemBuilder::with_id(TrayCommand::OpenRocket.id(), "Open Rocket").build(app)?;
    let gc = MenuItemBuilder::with_id(TrayCommand::CollectGarbage.id(), "Collect Garbage")
        .enabled(s.connected)
        .build(app)?;
    let updates =
        MenuItemBuilder::with_id(TrayCommand::CheckUpdates.id(), "Check for Updates\u{2026}")
            .build(app)?;
    let quit = MenuItemBuilder::with_id(TrayCommand::Quit.id(), "Quit Rocket").build(app)?;
    menu.separator()
        .item(&open)
        .item(&gc)
        .item(&updates)
        .separator()
        .item(&quit)
        .build()
}

fn apply(app: &AppHandle, s: &TraySummary) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app, s) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => tracing::warn!("tray menu: {e}"),
    }
    let _ = tray.set_tooltip(Some(tooltip(s)));
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(Some(title(s)));
}

/// Creates the tray icon and the debounced refresher task.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let initial = TraySummary::default();
    let menu = build_menu(app, &initial)?;
    let icon = Image::from_bytes(ICON)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip(tooltip(&initial))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if let Some(cmd) = TrayCommand::from_id(event.id().as_ref()) {
                run_command(app, cmd);
            }
        })
        .build(app)?;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        loop {
            state.tray.notified().await;
            // Coalesce bursts of events into one recomputation.
            tokio::time::sleep(DEBOUNCE).await;
            refresh(&app, &state).await;
        }
    });
    Ok(())
}

async fn refresh(app: &AppHandle, state: &AppState) {
    let summary = if matches!(state.status().await, ConnectionStatus::Online { .. }) {
        let client = state.client().await;
        match tokio::try_join!(client.ps(None, true), client.projects()) {
            Ok((ps, projects)) => summarize(true, &projects.projects, &ps.services),
            Err(e) => {
                tracing::debug!("tray refresh: {e}");
                return;
            }
        }
    } else {
        TraySummary::default()
    };
    apply(app, &summary);
}

fn run_command(app: &AppHandle, cmd: TrayCommand) {
    match cmd {
        TrayCommand::OpenRocket => crate::window::show_main(app),
        TrayCommand::OpenProject(name) => {
            crate::window::show_main(app);
            let _ = app.emit(OPEN_PROJECT_EVENT, name);
        }
        // Same path as the app-menu item: the frontend runs the manual check.
        TrayCommand::CheckUpdates => {
            crate::window::show_main(app);
            let _ = app.emit(crate::menu::EVENT, MenuAction::CheckUpdates.id());
        }
        TrayCommand::Quit => app.exit(0),
        TrayCommand::Reconnect => app.state::<AppState>().reconnect.notify_one(),
        TrayCommand::Up(_) | TrayCommand::Down(_) | TrayCommand::CollectGarbage => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                let client = state.client().await;
                let result = match cmd {
                    TrayCommand::Up(project) => client
                        .up(&UpRequest {
                            project,
                            owner: DEFAULT_OWNER.to_owned(),
                            ..UpRequest::default()
                        })
                        .await
                        .map(drop),
                    TrayCommand::Down(project) => client
                        .down(&DownRequest {
                            project,
                            ..DownRequest::default()
                        })
                        .await
                        .map(drop),
                    _ => client.gc().await.map(drop),
                };
                if let Err(e) = result {
                    tracing::warn!("tray action failed: {e}");
                }
                state.tray.notify_one();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket_domain::{Health, RunState, ServiceKind};
    use time::OffsetDateTime;

    fn run(project: &str, service: &str, state: RunState) -> Run {
        Run {
            project: project.into(),
            service: service.into(),
            env: "dev".into(),
            kind: ServiceKind::Run,
            state,
            health: Health::Unknown,
            pid: 0,
            pgid: 0,
            compose_project: String::new(),
            profiles: vec![],
            container_id: String::new(),
            owner: String::new(),
            expires_at: None,
            started_at: None,
            stopped_at: None,
            exit_code: None,
            ports: Default::default(),
            log_path: String::new(),
            error: String::new(),
        }
    }

    fn project(name: &str) -> ProjectRef {
        ProjectRef {
            name: name.into(),
            path: format!("/p/{name}"),
            added_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn counts_only_active_runs_per_project() {
        let runs = [
            run("api", "web", RunState::Running),
            run("api", "db", RunState::Starting),
            run("api", "worker", RunState::Stopped),
            run("shop", "web", RunState::Failed),
        ];
        let s = summarize(true, &[project("shop"), project("api")], &runs);
        assert_eq!(s.running, 2);
        assert_eq!(
            s.projects,
            vec![
                ProjectLine {
                    name: "api".into(),
                    active: 2,
                    total: 3
                },
                ProjectLine {
                    name: "shop".into(),
                    active: 0,
                    total: 1
                },
            ]
        );
    }

    #[test]
    fn projects_with_runs_but_not_registered_are_listed() {
        let s = summarize(
            true,
            &[project("a")],
            &[run("ghost", "x", RunState::Running)],
        );
        let names: Vec<_> = s.projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "ghost"]);
    }

    #[test]
    fn headline_pluralizes_and_reports_offline() {
        let mut s = TraySummary {
            connected: true,
            ..TraySummary::default()
        };
        assert_eq!(headline(&s), "No services running");
        s.running = 1;
        assert_eq!(headline(&s), "1 service running");
        s.running = 4;
        assert_eq!(headline(&s), "4 services running");
        s.connected = false;
        assert_eq!(headline(&s), "rocketd offline");
        assert_eq!(tooltip(&s), "Rocket: rocketd offline");
    }

    #[test]
    fn title_is_the_count_only_while_connected_and_busy() {
        let mut s = TraySummary {
            connected: true,
            running: 3,
            projects: vec![],
        };
        assert_eq!(title(&s), "3");
        s.running = 0;
        assert_eq!(title(&s), "");
        s.running = 3;
        s.connected = false;
        assert_eq!(title(&s), "");
    }

    #[test]
    fn long_project_names_are_truncated_like_the_original_menu() {
        let line = ProjectLine {
            name: "x".repeat(40),
            active: 1,
            total: 2,
        };
        let label = project_label(&line);
        let name = label.split("  ").next().unwrap();
        assert_eq!(name.chars().count(), 28); // 27 + ellipsis, like the original menu
        assert!(name.ends_with('\u{2026}'));
        assert!(label.ends_with("  1/2"));
        let short = ProjectLine {
            name: "api".into(),
            active: 0,
            total: 2,
        };
        assert_eq!(project_label(&short), "api  0/2");
    }

    #[test]
    fn command_ids_round_trip_even_with_colons_in_names() {
        let cmds = [
            TrayCommand::OpenRocket,
            TrayCommand::OpenProject("a:b".into()),
            TrayCommand::Up("my app".into()),
            TrayCommand::Down("x".into()),
            TrayCommand::CollectGarbage,
            TrayCommand::CheckUpdates,
            TrayCommand::Reconnect,
            TrayCommand::Quit,
        ];
        for c in cmds {
            assert_eq!(TrayCommand::from_id(&c.id()).as_ref(), Some(&c));
        }
        assert_eq!(TrayCommand::from_id("services.up"), None);
        assert_eq!(TrayCommand::from_id("tray.unknown:x"), None);
    }
}
