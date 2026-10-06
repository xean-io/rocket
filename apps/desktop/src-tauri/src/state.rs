//! Shared application state managed by Tauri.

use crate::dto::ConnectionStatus;
use rocket_client::{Client, EnsureOptions, Paths, TransportKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, RwLock};
use tokio::task::AbortHandle;

pub type Follows = Arc<Mutex<HashMap<String, AbortHandle>>>;

pub struct AppState {
    pub paths: Paths,
    client: RwLock<Client>,
    status: RwLock<ConnectionStatus>,
    /// Serializes daemon launches (supervisor vs `restart_daemon`).
    pub ensure_lock: tokio::sync::Mutex<()>,
    /// Wakes the supervisor: skip the backoff wait or drop the open stream.
    pub reconnect: Notify,
    /// Wakes the tray refresher (debounced recomputation of the tray menu).
    pub tray: Notify,
    pub follows: Follows,
}

/// The `rocket` CLI shipped as a Tauri sidecar: `externalBin` entries land
/// next to the app executable (`Rocket.app/Contents/MacOS/rocket`) without
/// their target-triple suffix.
pub fn bundled_rocket_bin() -> Option<PathBuf> {
    sidecar_beside(&std::env::current_exe().ok()?)
}

fn sidecar_beside(exe: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "rocket.exe"
    } else {
        "rocket"
    };
    Some(exe.parent()?.join(name))
}

impl AppState {
    pub fn new(paths: Paths) -> Result<Self, rocket_client::ClientError> {
        let client = Client::from_paths(&paths, TransportKind::Unix)?;
        Ok(Self {
            paths,
            client: RwLock::new(client),
            status: RwLock::new(ConnectionStatus::Connecting),
            ensure_lock: tokio::sync::Mutex::new(()),
            reconnect: Notify::new(),
            tray: Notify::new(),
            follows: Arc::default(),
        })
    }

    /// Daemon launch options: the bundled sidecar is the last-resort binary.
    pub fn ensure_options(&self) -> EnsureOptions {
        let mut opts = EnsureOptions::new(self.paths.clone());
        opts.bundled_bin = bundled_rocket_bin();
        opts
    }

    /// A cheap clone of the current client.
    pub async fn client(&self) -> Client {
        self.client.read().await.clone()
    }

    pub async fn set_client(&self, client: Client) {
        *self.client.write().await = client;
    }

    pub async fn status(&self) -> ConnectionStatus {
        self.status.read().await.clone()
    }

    /// Stores `status`; `true` when it differs from the previous one.
    pub async fn set_status(&self, status: ConnectionStatus) -> bool {
        let mut cur = self.status.write().await;
        if *cur == status {
            return false;
        }
        *cur = status;
        true
    }

    /// Aborts one follow task; `true` when it was running.
    pub fn stop_follow(&self, id: &str) -> bool {
        let handle = self.follows.lock().expect("follows lock").remove(id);
        handle.inspect(AbortHandle::abort).is_some()
    }

    pub fn stop_all_follows(&self) {
        for (_, h) in self.follows.lock().expect("follows lock").drain() {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_sits_next_to_the_app_executable() {
        let exe = Path::new("/Applications/Rocket.app/Contents/MacOS/rocket-desktop");
        let want = if cfg!(windows) {
            "rocket.exe"
        } else {
            "rocket"
        };
        assert_eq!(
            sidecar_beside(exe),
            Some(Path::new("/Applications/Rocket.app/Contents/MacOS").join(want))
        );
    }
}
