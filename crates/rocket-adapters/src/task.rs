//! Builds go-task invocations (Go: `adapters/task`). Long-running tasks are
//! executed by the process adapter so they get the same group supervision.

use rocket_domain::ports::TaskDriver;
use std::path::{Path, PathBuf};

/// The files go-task looks for, in its own order.
pub const TASKFILE_NAMES: [&str; 8] = [
    "Taskfile.yml",
    "taskfile.yml",
    "Taskfile.yaml",
    "taskfile.yaml",
    "Taskfile.dist.yml",
    "taskfile.dist.yml",
    "Taskfile.dist.yaml",
    "taskfile.dist.yaml",
];

/// [`TaskDriver`] for the `task` binary.
#[derive(Debug, Clone, Default)]
pub struct Driver {
    /// Defaults to `"task"` when empty.
    pub bin: String,
}

impl Driver {
    /// Uses `task` from `PATH`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses an explicit binary.
    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }
}

impl TaskDriver for Driver {
    /// `task <name> [-- args...]`.
    fn argv(&self, task: &str, args: &[String]) -> Vec<String> {
        let bin = if self.bin.is_empty() {
            "task"
        } else {
            &self.bin
        };
        let mut argv = vec![bin.to_string(), task.to_string()];
        if !args.is_empty() {
            argv.push("--".into());
            argv.extend(args.iter().cloned());
        }
        argv
    }

    /// The first Taskfile in `dir` (regular file, not a directory).
    fn taskfile(&self, dir: &Path) -> Option<PathBuf> {
        TASKFILE_NAMES.iter().map(|n| dir.join(n)).find(|p| {
            // `metadata` follows symlinks, like Go's `os.Stat`.
            std::fs::metadata(p).is_ok_and(|m| !m.is_dir())
        })
    }
}
