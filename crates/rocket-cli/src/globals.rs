//! Persistent flags, project resolution and the daemon connection (Go:
//! `globals`, `projectRef`, `owner`, `daemonClient` in `main.go`).

use crate::error::{CliError, Result};
use rocket_client::{Client, EnsureOptions, Paths, ensure_daemon};
use rocket_domain::DEFAULT_OWNER;
use std::path::{MAIN_SEPARATOR, Path};

/// The persistent flags (`--json`, `-p/--project`).
#[derive(Debug, Clone, Default)]
pub struct Globals {
    pub json: bool,
    pub project: String,
}

/// `.`, `..` or anything containing a path separator is a path, not a name.
pub fn looks_like_path(s: &str) -> bool {
    s == "." || s == ".." || s.contains(MAIN_SEPARATOR) || s.contains('/')
}

impl Globals {
    /// Resolves `-p` (name or path) or walks up from the working directory
    /// to the nearest rocket.yaml.
    pub fn project_ref(&self) -> Result<String> {
        let cwd = std::env::current_dir()?;
        self.project_ref_in(&cwd)
    }

    /// [`Globals::project_ref`] with an explicit working directory.
    pub fn project_ref_in(&self, cwd: &Path) -> Result<String> {
        if !self.project.is_empty() {
            if looks_like_path(&self.project) {
                let abs = rocket_scaffold::abs(&cwd.join(&self.project)).map_err(CliError::from)?;
                return Ok(rocket_manifest::find(&abs)?.display().to_string());
            }
            return Ok(self.project.clone());
        }
        match rocket_manifest::find(cwd) {
            Ok(dir) => Ok(dir.display().to_string()),
            Err(e) => {
                let text = format!("{e} (pass -p <project>)");
                Err(if e.is_no_manifest() {
                    CliError::NoManifest(text)
                } else {
                    CliError::Message(text)
                })
            }
        }
    }
}

/// `--owner`, then `$ROCKET_OWNER`, then `user`.
pub fn owner(flag: &str) -> String {
    owner_from(flag, std::env::var("ROCKET_OWNER").ok().as_deref())
}

pub fn owner_from(flag: &str, env: Option<&str>) -> String {
    if !flag.is_empty() {
        return flag.to_owned();
    }
    match env {
        Some(e) if !e.is_empty() => e.to_owned(),
        _ => DEFAULT_OWNER.to_owned(),
    }
}

/// A client for the running daemon, starting one (this very binary with
/// `daemon run`) when none answers.
pub async fn daemon_client() -> Result<Client> {
    let paths = Paths::resolve()?;
    ensure(paths).await
}

pub async fn ensure(paths: Paths) -> Result<Client> {
    let mut opts = EnsureOptions::new(paths);
    opts.rocket_bin = Some(std::env::current_exe()?);
    Ok(ensure_daemon(&opts).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_precedence() {
        assert_eq!(owner_from("flag", Some("env")), "flag");
        assert_eq!(owner_from("", Some("agent:x")), "agent:x");
        assert_eq!(owner_from("", Some("")), "user");
        assert_eq!(owner_from("", None), "user");
    }

    #[test]
    fn paths_versus_names() {
        for p in [".", "..", "./x", "a/b", "/abs"] {
            assert!(looks_like_path(p), "{p}");
        }
        for n in ["fixture", "rocket-fixture", ""] {
            assert!(!looks_like_path(n), "{n:?}");
        }
    }

    #[test]
    fn project_ref_walks_up_from_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rocket.yaml"), "version: 1\n").unwrap();
        let sub = dir.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        let g = Globals::default();
        let got = g.project_ref_in(&sub).unwrap();
        assert_eq!(
            Path::new(&got).canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );

        let named = Globals {
            json: false,
            project: "my-app".into(),
        };
        assert_eq!(named.project_ref_in(&sub).unwrap(), "my-app");

        let relative = Globals {
            json: false,
            project: "../..".into(),
        };
        let got = relative.project_ref_in(&sub).unwrap();
        assert_eq!(
            Path::new(&got).canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn missing_manifest_adds_the_flag_hint_only_when_discovering() {
        let dir = tempfile::tempdir().unwrap();
        let err = Globals::default().project_ref_in(dir.path()).unwrap_err();
        assert!(err.is_no_manifest());
        assert!(
            err.to_string()
                .ends_with("any parent directory (pass -p <project>)"),
            "{err}"
        );
        let g = Globals {
            json: false,
            project: dir.path().join("missing").display().to_string(),
        };
        let err = g.project_ref_in(dir.path()).unwrap_err();
        assert!(err.is_no_manifest());
        assert!(!err.to_string().contains("pass -p"), "{err}");
    }
}
