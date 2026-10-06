//! projects, daemon and schema (Go: `admin.go`).

use crate::error::{CliError, EXIT_ERROR, Result};
use crate::format::{go_duration, out, outln, print_json, round_duration};
use crate::globals::{Globals, ensure, looks_like_path};
use crate::tabwriter::Table;
use rocket_client::{Client, Paths};
use rocket_domain::api::HealthInfo;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use time::OffsetDateTime;

pub async fn projects_add(mut g: Globals, path: Option<String>) -> Result<()> {
    if let Some(p) = path {
        g.project = if looks_like_path(&p) {
            p
        } else {
            format!("./{p}")
        };
    }
    let reference = g.project_ref()?;
    let client = crate::globals::daemon_client().await?;
    let res = client.add_project(&reference).await?;
    if g.json {
        print_json(&res);
    } else {
        outln(&format!("registered {} -> {}", res.name, res.path));
    }
    Ok(())
}

pub async fn projects_ls(g: &Globals) -> Result<()> {
    let client = crate::globals::daemon_client().await?;
    let res = client.projects().await?;
    if g.json {
        print_json(&res);
        return Ok(());
    }
    let mut t = Table::new();
    t.row(["NAME", "PATH"]);
    for p in &res.projects {
        t.row([p.name.clone(), p.path.clone()]);
    }
    out(&t.render());
    Ok(())
}

pub async fn projects_rm(g: &Globals, name: &str) -> Result<()> {
    let client = crate::globals::daemon_client().await?;
    client.remove_project(name).await?;
    if g.json {
        print_json(&BTreeMap::from([("removed", name)]));
    } else {
        outln(&format!("removed {name}"));
    }
    Ok(())
}

/// The layout section of [`DaemonStatus`] (Go: `paths.Paths`).
#[derive(Debug, Serialize)]
pub struct PathsOut {
    pub home: String,
    pub socket: String,
    pub db: String,
    pub logs: String,
    pub pid_file: String,
    pub lock_file: String,
    pub daemon_log: String,
    pub daemon_json: String,
}

impl From<&Paths> for PathsOut {
    fn from(p: &Paths) -> Self {
        let s = |x: &std::path::Path| x.display().to_string();
        Self {
            home: s(&p.home),
            socket: s(&p.socket),
            db: s(&p.db),
            logs: s(&p.logs),
            pid_file: s(&p.pid_file),
            lock_file: s(&p.lock_file),
            daemon_log: s(&p.daemon_log),
            daemon_json: s(&p.daemon_json),
        }
    }
}

/// The JSON shape of `rocket daemon status|start|stop`.
#[derive(Debug, Serialize)]
pub struct DaemonStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<HealthInfo>,
    pub paths: PathsOut,
}

async fn running(paths: &Paths) -> Option<HealthInfo> {
    Client::unix(&paths.socket).is_running().await
}

async fn report(g: &Globals, paths: &Paths) -> Result<()> {
    let info = running(paths).await;
    let ok = info.is_some();
    if g.json {
        print_json(&DaemonStatus {
            running: ok,
            info: info.clone(),
            paths: paths.into(),
        });
    } else if let Some(info) = &info {
        let up = round_duration(
            (OffsetDateTime::now_utc() - info.started_at).whole_nanoseconds(),
            1_000_000_000,
        );
        outln(&format!(
            "rocketd running (pid {}, {}, up {})\nsocket {}",
            info.pid,
            info.version,
            go_duration(up),
            info.socket
        ));
    } else {
        outln("rocketd not running");
    }
    if ok {
        Ok(())
    } else {
        Err(CliError::Exit(EXIT_ERROR))
    }
}

/// Runs the daemon in the foreground; it handles SIGINT/SIGTERM itself.
pub async fn daemon_run() -> Result<()> {
    let paths = Paths::resolve()?;
    rocket_daemon::run(rocket_daemon::RunOptions::for_process(
        paths,
        crate::version::version(),
    ))
    .await
    .map_err(|e| CliError::Message(format!("{e:#}")))
}

pub async fn daemon_start(g: &Globals) -> Result<()> {
    let paths = Paths::resolve()?;
    ensure(paths.clone()).await?;
    report(g, &paths).await
}

pub async fn daemon_stop(g: &Globals) -> Result<()> {
    let paths = Paths::resolve()?;
    if running(&paths).await.is_some() {
        Client::unix(&paths.socket).shutdown().await?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if running(&paths).await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    if running(&paths).await.is_some() {
        return Err(CliError::msg("rocketd still running after 10s"));
    }
    if g.json {
        print_json(&DaemonStatus {
            running: false,
            info: None,
            paths: (&paths).into(),
        });
    } else {
        outln("rocketd stopped");
    }
    Ok(())
}

pub async fn daemon_status(g: &Globals) -> Result<()> {
    let paths = Paths::resolve()?;
    report(g, &paths).await
}

/// Prints the JSON Schema of rocket.yaml.
pub fn schema() -> Result<()> {
    out(&format!("{}\n", rocket_manifest::schema()));
    Ok(())
}
