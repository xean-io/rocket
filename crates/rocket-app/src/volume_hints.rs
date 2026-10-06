//! "New Compose volume" hints (Go: `volume_hints.go`). Evidence is advisory:
//! any Docker error means no hint, never a failed startup.

use crate::app::App;
use crate::up::cancelable;
use rocket_domain::Project;
use rocket_domain::ports::ComposeTarget;
use std::collections::BTreeSet;
use tokio_util::sync::CancellationToken;

impl App {
    /// Named volumes of `service` that Docker confirms do not exist yet,
    /// sorted and deduplicated. Empty on any evidence failure.
    pub(crate) async fn missing_compose_volumes(
        &self,
        target: &ComposeTarget,
        service: &str,
        cancel: &CancellationToken,
    ) -> Vec<String> {
        let Ok(Ok(names)) =
            cancelable(cancel, self.d().compose.named_volumes(target, service)).await
        else {
            return Vec::new();
        };
        let names: BTreeSet<String> = names.into_iter().collect();
        let mut missing = Vec::new();
        for name in names {
            if let Ok(Ok(false)) =
                cancelable(cancel, self.d().compose.volume_exists(target, &name)).await
            {
                missing.push(name);
            }
        }
        missing
    }

    /// Records the `missing` volumes that exist now, i.e. were created by
    /// this startup.
    pub(crate) async fn confirm_compose_volumes(
        &self,
        target: &ComposeTarget,
        missing: &[String],
        created: &mut BTreeSet<String>,
        cancel: &CancellationToken,
    ) {
        for name in missing {
            if let Ok(Ok(true)) =
                cancelable(cancel, self.d().compose.volume_exists(target, name)).await
            {
                created.insert(name.clone());
            }
        }
    }
}

/// The single hint line for newly created volumes, or none when the project
/// has no `migrate`/`assets` setup action to suggest.
pub(crate) fn compose_volume_hints(
    p: &Project,
    env: &str,
    created: &BTreeSet<String>,
) -> Vec<String> {
    if created.is_empty() {
        return Vec::new();
    }
    let mut selection = format!(" -p {}", hint_shell_arg(&p.name));
    if env != p.default_env {
        selection.push_str(&format!(" --env {}", hint_shell_arg(env)));
    }
    let mut actions = Vec::new();
    if p.setup.contains_key("migrate") {
        actions.push(format!("rocket migrate{selection}"));
    }
    if p.setup.contains_key("assets") {
        actions.push(format!("rocket setup assets{selection}"));
    }
    if actions.is_empty() {
        return Vec::new();
    }
    let names: Vec<&str> = created.iter().map(String::as_str).collect();
    vec![format!(
        "New Compose volumes ({}/{env}): {}. Run {} when prerequisites are ready.",
        p.name,
        names.join(", "),
        actions.join(" and ")
    )]
}

/// Quotes `value` for a POSIX shell unless it is plainly safe.
fn hint_shell_arg(value: &str) -> String {
    let safe = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c));
    if safe {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r#"'"'"'"#))
    }
}
