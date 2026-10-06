//! Shared helpers: short temp homes (unix socket paths are limited to ~103
//! bytes), an in-process daemon handle and a fixture copy.
#![allow(dead_code)]
#![cfg(unix)]

use rocket_client::{Client, Paths};
use rocket_daemon::{RunOptions, run};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const VERSION: &str = "9.9.9-test";

/// A temp dir directly under /tmp so `<dir>/home/rocketd.sock` stays short.
pub fn short_tmp() -> TempDir {
    tempfile::Builder::new()
        .prefix("rkd")
        .tempdir_in("/tmp")
        .expect("tempdir")
}

/// `<tmp>/home` as a ROCKET_HOME layout (never the real ~/.rocket).
pub fn paths_in(tmp: &TempDir) -> Paths {
    let tmp = std::fs::canonicalize(tmp.path()).unwrap();
    Paths::from_home(tmp.join("home")).unwrap()
}

pub struct Running {
    pub paths: Paths,
    pub stop: CancellationToken,
    pub handle: JoinHandle<anyhow::Result<()>>,
}

impl Running {
    pub fn client(&self) -> Client {
        Client::unix(&self.paths.socket)
    }

    /// Cancels the stop token and waits for `run` to return.
    pub async fn stop(self) -> anyhow::Result<()> {
        self.stop.cancel();
        self.join().await
    }

    pub async fn join(self) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(15), self.handle)
            .await
            .expect("daemon did not stop in time")
            .expect("daemon task panicked")
    }
}

/// Starts the daemon in this process and waits until it answers on its
/// socket; panics with the daemon's own error when it dies first.
pub async fn start(paths: &Paths) -> Running {
    start_with(paths, |_| {}).await
}

pub async fn start_with(paths: &Paths, tweak: impl FnOnce(&mut RunOptions)) -> Running {
    let mut opts = RunOptions::new(paths.clone(), VERSION);
    tweak(&mut opts);
    let stop = opts.stop.clone();
    let handle = tokio::spawn(run(opts));
    let running = Running {
        paths: paths.clone(),
        stop,
        handle,
    };
    let client = running.client();
    for _ in 0..400 {
        if running.handle.is_finished() {
            let err = running
                .handle
                .await
                .unwrap()
                .expect_err("daemon exited early");
            panic!("daemon failed to start: {err}");
        }
        if client.is_running().await.is_some() {
            return running;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("daemon did not answer within 10s");
}

/// A private copy of `testdata/fixture` at `dst`.
pub fn copy_fixture(dst: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/fixture");
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
    }
    std::fs::canonicalize(dst).unwrap()
}
