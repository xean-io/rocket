//! status, init, agent install and app (Go: `ai.go`).

use crate::cli::InitArgs;
use crate::commands::services::optional_project;
use crate::error::{CliError, Result};
use crate::format::{duration_str, expires_str_now, format_ports, or_dash, out, outln, print_json};
use crate::globals::{Globals, daemon_client};
use crate::tabwriter::Table;
use rocket_agentdocs::Options;
use rocket_domain::api::Summary;
use rocket_scaffold::Detected;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub async fn status(g: &Globals) -> Result<()> {
    let project = optional_project(g)?;
    let client = daemon_client().await?;
    let sum = client.status(project.as_deref()).await?;
    if g.json {
        print_json(&sum);
    } else {
        out(&render_summary(&sum));
    }
    Ok(())
}

/// The human `rocket status` text (Go: `printSummary`).
pub fn render_summary(sum: &Summary) -> String {
    let mut s = String::new();
    if let Some(p) = &sum.project {
        s.push_str(&format!(
            "project {} ({}, default env {})\n",
            p.name, p.root, p.default_env
        ));
    }
    let mut t = Table::new();
    t.row([
        "PROJECT", "SERVICE", "STATE", "HEALTH", "PORTS", "OWNER", "TTL",
    ]);
    for r in &sum.services {
        t.row([
            r.project.clone(),
            r.service.clone(),
            r.state.to_string(),
            or_dash(r.health.as_str()),
            format_ports(&r.ports),
            or_dash(&r.owner),
            expires_str_now(r.expires_at),
        ]);
    }
    s.push_str(&t.render());
    if !sum.jobs.is_empty() {
        s.push_str("\nrunning jobs:\n");
        for j in &sum.jobs {
            s.push_str(&format!(
                "  {}  {} {} ({}, step {}/{}, owner {}, {})\n",
                j.id,
                j.kind,
                j.name,
                j.project,
                j.step,
                j.steps.len(),
                j.owner,
                duration_str(j.duration_ms)
            ));
        }
    }
    if !sum.conflicts.is_empty() {
        s.push_str("\nconflicts:\n");
        for c in &sum.conflicts {
            s.push_str(&format!(
                "  {} {}/{} {}:{}: {}\n",
                c.kind, c.project, c.service, c.port_name, c.port, c.detail
            ));
        }
    }
    s
}

/// The JSON shape of `rocket init`.
#[derive(Debug, Serialize)]
pub struct InitResult<'a> {
    pub path: String,
    pub written: bool,
    /// With `--print`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    pub detected: &'a Detected,
}

pub fn init(g: &Globals, a: InitArgs) -> Result<()> {
    let dir = if g.project.is_empty() {
        "."
    } else {
        g.project.as_str()
    };
    let dir = rocket_scaffold::abs(Path::new(dir))?;
    if !std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Err(CliError::msg(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    let det = rocket_scaffold::detect(&dir).map_err(|e| CliError::Message(e.to_string()))?;
    let content = det.render();
    if let Err(e) = rocket_manifest::parse(content.as_bytes(), &dir.to_string_lossy()) {
        return Err(CliError::msg(format!(
            "generated rocket.yaml does not validate (please report): {e}"
        )));
    }
    let path = dir.join(rocket_manifest::FILE_NAME);
    let mut res = InitResult {
        path: path.display().to_string(),
        written: false,
        content: String::new(),
        detected: &det,
    };
    if a.print_only {
        if g.json {
            res.content = content;
            print_json(&res);
        } else {
            out(&content);
        }
        return Ok(());
    }
    for existing in ["rocket.yaml", "rocket.yml"] {
        let p = dir.join(existing);
        if std::fs::metadata(&p).is_ok() && !a.force {
            return Err(CliError::msg(format!(
                "{} already exists (use --force to overwrite or --print to preview)",
                p.display()
            )));
        }
    }
    std::fs::write(&path, &content)
        .map_err(|e| CliError::msg(format!("open {}: {}", path.display(), io_reason(&e))))?;
    res.written = true;
    if g.json {
        print_json(&res);
        return Ok(());
    }
    outln(&format!(
        "wrote {}: {} services, {} pipelines, {} envs; review the guesses, then `rocket up`",
        path.display(),
        det.services.len(),
        det.pipelines.len(),
        det.envs.len()
    ));
    Ok(())
}

fn io_reason(err: &std::io::Error) -> String {
    let text = err.to_string();
    let text = text
        .rfind(" (os error")
        .map_or(text.as_str(), |i| &text[..i]);
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// The JSON shape of `rocket agent install`.
#[derive(Debug, Serialize)]
pub struct AgentInstallResult {
    /// `null` when nothing was written (Go's nil slice).
    pub actions: Option<Vec<rocket_agentdocs::Action>>,
    /// With `--print`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub skill: String,
    /// With `--print`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub block: String,
}

pub fn agent_install(g: &Globals, target: &str, global: bool, print_only: bool) -> Result<()> {
    let home = std::env::home_dir().unwrap_or_default();
    let cwd = std::env::current_dir()?;
    let root: PathBuf = match rocket_manifest::find(&cwd) {
        Ok(dir) => dir,
        Err(_) if !global => cwd,
        Err(_) => PathBuf::new(),
    };
    let opts = Options {
        target: target.to_owned(),
        global,
        home,
        project_root: root,
    };
    let fail = |e: rocket_agentdocs::Error| CliError::Message(e.to_string());
    if print_only {
        let plan = rocket_agentdocs::plan(&opts).map_err(fail)?;
        if g.json {
            print_json(&AgentInstallResult {
                actions: Some(plan),
                skill: rocket_agentdocs::skill(),
                block: rocket_agentdocs::block(),
            });
            return Ok(());
        }
        for a in &plan {
            outln(&format!("# would write {} ({})", a.path, a.kind));
        }
        out(&format!(
            "\n===== SKILL.md =====\n{}\n===== AGENTS.md / CLAUDE.md block =====\n{}",
            rocket_agentdocs::skill(),
            rocket_agentdocs::block()
        ));
        return Ok(());
    }
    let actions = rocket_agentdocs::install(&opts).map_err(fail)?;
    if g.json {
        print_json(&AgentInstallResult {
            actions: (!actions.is_empty()).then_some(actions),
            skill: String::new(),
            block: String::new(),
        });
        return Ok(());
    }
    for a in &actions {
        outln(&format!("{:<9} {}", a.action, a.path));
    }
    Ok(())
}

/// Opens Rocket.app (macOS) after making sure the daemon runs.
pub async fn app(g: &Globals) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(CliError::msg(
            "UI not available on this OS yet; use the CLI",
        ));
    }
    daemon_client().await?;
    let app = std::env::var("ROCKET_APP")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Rocket".to_owned());
    let output = tokio::process::Command::new("open")
        .args(["-a", &app])
        .output()
        .await
        .map_err(|e| {
            CliError::msg(format!(
                "open -a {app}: {e}: (build it from macos/ or set ROCKET_APP to its path)"
            ))
        })?;
    if !output.status.success() {
        let combined = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
        return Err(CliError::msg(format!(
            "open -a {app}: {}: {} (build it from macos/ or set ROCKET_APP to its path)",
            output.status,
            String::from_utf8_lossy(&combined).trim()
        )));
    }
    if g.json {
        print_json(&std::collections::BTreeMap::from([
            ("app", serde_json::Value::from(app)),
            ("opened", serde_json::Value::from(true)),
        ]));
    }
    Ok(())
}
