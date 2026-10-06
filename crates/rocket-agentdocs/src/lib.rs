//! Renders the instructions that teach AI agents to use rocket (a Claude Code
//! skill and an AGENTS.md/CLAUDE.md block) and installs them idempotently
//! (Go: `agentdocs`).
//!
//! The texts are byte-identical to the Go output; `tests/fixtures` holds the
//! content captured from `rocket agent install --print --json`.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Markers delimiting the managed block in AGENTS.md / CLAUDE.md.
pub const BEGIN_MARKER: &str = "<!-- rocket:begin -->";
pub const END_MARKER: &str = "<!-- rocket:end -->";

/// Install targets.
/// Claude Code skill + CLAUDE.md block.
pub const TARGET_CLAUDE: &str = "claude";
/// AGENTS.md block.
pub const TARGET_AGENTS: &str = "agents";
pub const TARGET_BOTH: &str = "both";

const RULES: &str = r#"- Use rocket for ALL process, service, test and CI operations. Never start processes yourself:
  no `&`, `nohup`, `docker compose up`, `task dev`, watchers or dev servers outside rocket.
- Identify yourself on every command: `export ROCKET_OWNER=agent:<session-id>` (or `--owner agent:<session-id>`).
- Always pass `--ttl` for exploratory runs: `rocket up api --ttl 30m`.
- Before finishing, clean up everything you started: `rocket down --owner agent:<session-id>`.
- Run targeted checks, not full suites: `rocket run <targeted pipeline|task> [-- args]`
  (e.g. `rocket run test:api -- -run TestLogin`).
- Always add `--json` and check exit codes: 0 ok, 1 error, 2 partial or job failure, 3 confirmation required.
- Look before acting: `rocket status --json` shows services, ports, running jobs and port conflicts.
- Deploys need a human: `rocket deploy <env>` is refused for agents unless a human passes `--yes`
  and sets `ROCKET_ALLOW_DEPLOY=1`.
"#;

const CHEATSHEET: &str = r#"```sh
export ROCKET_OWNER=agent:<session-id>
rocket status --json                      # project, services, ports, running jobs, conflicts
rocket up <svc|group> --ttl 30m --json    # idempotent; dependencies first; busy ports remapped
rocket ps --json | rocket ports --json
rocket logs <svc> --tail 100 --json
rocket run <pipeline|task> --json [-- args]   # blocks; {job,status,exit_code,duration_ms,log_path,tail}
rocket run <pipeline> --detach --json     # returns the job; then rocket job <id> logs -f
rocket jobs --json | rocket job <id> --json | rocket job cancel <id>
rocket setup --json                       # doctor -> install -> migrate
rocket down --owner agent:<session-id> --json
```
"#;

const SKILL_HEAD: &str = r#"---
name: rocket
description: "Trigger: starting or stopping dev servers, docker compose, Taskfile tasks, tests, CI, migrations, deploys, busy ports, logs. Use the rocket CLI for every process/service/test/CI operation."
---

# rocket: the single owner of dev processes

This machine runs rocket, a supervisor that owns every dev process across projects
(services, compose containers, test and CI jobs). Processes you start yourself become
orphans that hold ports and break the next run; rocket tracks, cleans and reports them.

## Rules

"#;

/// The Claude Code skill (SKILL.md).
pub fn skill() -> String {
    format!("{SKILL_HEAD}{RULES}\n## Commands\n\n{CHEATSHEET}")
}

/// The managed AGENTS.md / CLAUDE.md block, ending in a newline.
pub fn block() -> String {
    format!(
        "{BEGIN_MARKER}\n## Processes, services, tests and CI: use rocket\n\n{RULES}\n{CHEATSHEET}{END_MARKER}\n"
    )
}

/// Inserts or refreshes the managed block in existing content. The flag is
/// whether the content changed.
pub fn upsert_block(existing: &str) -> (String, bool) {
    let block = block();
    let begin = existing.find(BEGIN_MARKER);
    let end = existing.find(END_MARKER);
    let out = match (begin, end) {
        (Some(begin), Some(end)) if end > begin => {
            let rest = &existing[end + END_MARKER.len()..];
            let rest = rest.strip_prefix('\n').unwrap_or(rest);
            format!("{}{block}{rest}", &existing[..begin])
        }
        _ if existing.is_empty() => block,
        _ => {
            let nl = if existing.ends_with('\n') { "" } else { "\n" };
            format!("{existing}{nl}\n{block}")
        }
    };
    let changed = out != existing;
    (out, changed)
}

/// What [`install`] writes.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `claude` | `agents` | `both` (empty means both).
    pub target: String,
    /// Skill under `home/.claude` instead of the project.
    pub global: bool,
    /// User home (tests pass a temp dir).
    pub home: PathBuf,
    /// Where AGENTS.md/CLAUDE.md live; empty skips the blocks.
    pub project_root: PathBuf,
}

/// One file [`install`] touched (or [`plan`] would touch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Action {
    pub path: String,
    /// `skill` | `block`.
    pub kind: String,
    /// `created` | `updated` | `unchanged` (`planned` from [`plan`]).
    pub action: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown target \"{0}\" (claude, agents or both)")]
    UnknownTarget(String),
    #[error("no directory for the skill (not in a project and no home directory)")]
    NoSkillDir,
    #[error("{0}")]
    Io(String),
}

struct Planned {
    path: PathBuf,
    kind: &'static str,
}

fn plan_files(o: &Options) -> Result<Vec<Planned>, Error> {
    let target = if o.target.is_empty() {
        TARGET_BOTH
    } else {
        o.target.as_str()
    };
    let claude = target == TARGET_CLAUDE || target == TARGET_BOTH;
    let agents = target == TARGET_AGENTS || target == TARGET_BOTH;
    if !claude && !agents {
        return Err(Error::UnknownTarget(target.to_owned()));
    }
    let mut out = Vec::new();
    if claude {
        let base = if o.global { &o.home } else { &o.project_root };
        if base.as_os_str().is_empty() {
            return Err(Error::NoSkillDir);
        }
        out.push(Planned {
            path: base
                .join(".claude")
                .join("skills")
                .join("rocket")
                .join("SKILL.md"),
            kind: "skill",
        });
    }
    if !o.project_root.as_os_str().is_empty() {
        if agents {
            out.push(Planned {
                path: o.project_root.join("AGENTS.md"),
                kind: "block",
            });
        }
        if claude {
            out.push(Planned {
                path: o.project_root.join("CLAUDE.md"),
                kind: "block",
            });
        }
    }
    Ok(out)
}

/// Lists the files [`install`] would write.
pub fn plan(o: &Options) -> Result<Vec<Action>, Error> {
    Ok(plan_files(o)?
        .into_iter()
        .map(|f| Action {
            path: f.path.display().to_string(),
            kind: f.kind.into(),
            action: "planned".into(),
        })
        .collect())
}

/// Writes the skill and/or blocks; running it twice changes nothing.
pub fn install(o: &Options) -> Result<Vec<Action>, Error> {
    let mut out = Vec::new();
    for f in plan_files(o)? {
        let old = read_optional(&f.path)?;
        let next = if f.kind == "block" {
            upsert_block(old.as_deref().unwrap_or("")).0
        } else {
            skill()
        };
        let mut action = "unchanged";
        if old.as_deref() != Some(next.as_str()) {
            write_file(&f.path, &next)?;
            action = if old.is_some() { "updated" } else { "created" };
        }
        out.push(Action {
            path: f.path.display().to_string(),
            kind: f.kind.into(),
            action: action.into(),
        });
    }
    Ok(out)
}

fn read_optional(path: &Path) -> Result<Option<String>, Error> {
    match std::fs::read(path) {
        Ok(data) => Ok(Some(String::from_utf8_lossy(&data).into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(go_path_error("open", path, &e))),
    }
}

fn write_file(path: &Path, content: &str) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        create_dir_all(dir).map_err(|e| Error::Io(go_path_error("mkdir", dir, &e)))?;
    }
    std::fs::write(path, content).map_err(|e| Error::Io(go_path_error("open", path, &e)))
}

#[cfg(unix)]
fn create_dir_all(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(dir)
}

#[cfg(not(unix))]
fn create_dir_all(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// `<op> <path>: <lowercased reason>`, like Go's `*PathError`.
fn go_path_error(op: &str, path: &Path, err: &std::io::Error) -> String {
    let text = err.to_string();
    let text = text
        .rfind(" (os error")
        .map_or(text.as_str(), |i| &text[..i]);
    let mut chars = text.chars();
    let text = chars
        .next()
        .map(|c| c.to_lowercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    format!("{op} {}: {text}", path.display())
}
