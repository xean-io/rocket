//! Taskfile reading and task classification (Go: `readTaskfile`,
//! `classifyTasks`).

use crate::{Detected, Service, SetupStep, Task};
use rocket_adapters::task::Driver;
use rocket_domain::ports::TaskDriver;
use rocket_manifest::yaml::{self, Node, NodeKind};
use std::path::{Path, PathBuf};

/// yaml.v3 decodes these into a `bool` field (the YAML 1.1 forms included).
fn yaml_bool(n: Node<'_>) -> bool {
    n.kind() == NodeKind::Scalar
        && matches!(
            n.value(),
            "true" | "True" | "TRUE" | "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON"
        )
}

fn scalar_string(n: Option<Node<'_>>) -> String {
    match n {
        Some(n) if n.kind() == NodeKind::Scalar && !n.is_null() => n.value().to_owned(),
        _ => String::new(),
    }
}

/// Lists the tasks of a Taskfile and its includes (namespaced, up to three
/// levels). Internal tasks/includes and templated paths are skipped.
pub(crate) fn read(path: &Path, prefix: &str, depth: u32) -> Result<Vec<Task>, String> {
    let data =
        std::fs::read(path).map_err(|e| format!("open {}: {}", path.display(), io_reason(&e)))?;
    let doc = yaml::parse(&String::from_utf8_lossy(&data))?;
    let Some(doc) = doc else {
        return Ok(Vec::new());
    };
    let root = doc.root();
    if root.is_null() {
        return Ok(Vec::new());
    }
    if root.kind() != NodeKind::Mapping {
        return Err(format!(
            "yaml: unmarshal errors:\n  line {}: cannot unmarshal {} into scaffold.taskfileYAML",
            root.line(),
            root.tag()
        ));
    }
    let mut out = Vec::new();
    if let Some(tasks) = root.get("tasks") {
        for (key, body) in tasks.entries() {
            let (mut desc, mut internal) = (String::new(), false);
            if body.kind() == NodeKind::Mapping {
                desc = scalar_string(body.get("desc"));
                internal = body.get("internal").is_some_and(yaml_bool);
            }
            let name = key.value();
            if internal || name == "default" {
                continue;
            }
            out.push(Task {
                name: format!("{prefix}{name}"),
                desc,
            });
        }
    }
    if depth >= 3 {
        return Ok(out);
    }
    if let Some(includes) = root.get("includes") {
        for (ns, body) in includes.entries() {
            let (taskfile, internal) = match body.kind() {
                NodeKind::Scalar => (body.value().to_owned(), false),
                NodeKind::Mapping => (
                    scalar_string(body.get("taskfile")),
                    body.get("internal").is_some_and(yaml_bool),
                ),
                NodeKind::Sequence => (String::new(), false),
            };
            if internal || taskfile.is_empty() || taskfile.contains("{{") {
                continue;
            }
            let mut p = PathBuf::from(&taskfile);
            if !p.is_absolute() {
                p = crate::gopath::clean(&path.parent().unwrap_or(Path::new(".")).join(&p));
            }
            if std::fs::metadata(&p).is_ok_and(|m| m.is_dir()) {
                match Driver::new().taskfile(&p) {
                    Some(tf) => p = tf,
                    None => continue,
                }
            }
            // Optional or missing include: ignore.
            if let Ok(sub) = read(&p, &format!("{prefix}{}:", ns.value()), depth + 1) {
                out.extend(sub);
            }
        }
    }
    Ok(out)
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

/// Guesses whether a task is a dev server.
pub fn is_long_running(name: &str) -> bool {
    name == "dev"
        || name == "serve"
        || name == "start"
        || name.starts_with("dev:")
        || name.ends_with(":dev")
}

fn is_deployish(name: &str) -> bool {
    name.split(':')
        .any(|seg| seg.contains("deploy") || seg.contains("release"))
}

pub(crate) fn classify(r: &mut Detected, mut tasks: Vec<Task>) {
    tasks.sort_by(|a, b| a.name.cmp(&b.name));
    let have = |name: &str| tasks.iter().any(|t| t.name == name);
    let mut setup_task: Vec<(&str, &str)> = Vec::new();
    for (step, task) in [
        ("doctor", "doctor"),
        ("install", "install"),
        ("install", "setup"),
        ("migrate", "migrate"),
        ("migrate", "db:migrate"),
    ] {
        if have(task) && !setup_task.iter().any(|(s, _)| *s == step) {
            setup_task.push((step, task));
            r.setup.push(SetupStep {
                name: step.into(),
                task: task.into(),
            });
        }
    }
    // `setup` stays reserved even when an explicit install takes precedence.
    let mut is_setup: Vec<String> = vec!["setup".into()];
    is_setup.extend(r.setup.iter().map(|s| s.task.clone()));
    let mut taken: Vec<String> = r.services.iter().map(|s| s.name.clone()).collect();
    for t in &tasks {
        if is_setup.contains(&t.name) {
            continue;
        }
        if is_deployish(&t.name) {
            r.deploy_tasks.push(t.name.clone());
        } else if is_long_running(&t.name) {
            let mut name = t.name.replace(':', "-");
            while taken.contains(&name) {
                name.push_str("-task");
            }
            taken.push(name.clone());
            r.services.push(Service {
                name,
                task: t.name.clone(),
                ..Service::default()
            });
        } else {
            r.pipelines.push(t.name.clone());
        }
    }
    r.tasks = tasks;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_running_guesses() {
        for (name, want) in [
            ("dev", true),
            ("serve", true),
            ("start", true),
            ("dev:web", true),
            ("web:dev", true),
            ("dev-server", false),
            ("build", false),
            ("developer", false),
        ] {
            assert_eq!(is_long_running(name), want, "{name}");
        }
    }

    #[test]
    fn deployish_checks_each_segment() {
        assert!(is_deployish("release:notes"));
        assert!(is_deployish("ci:deploy-prod"));
        assert!(!is_deployish("test:unit"));
    }
}
