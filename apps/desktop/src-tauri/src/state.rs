//! Shared application state managed by Tauri.

use crate::dto::ConnectionStatus;
use rocket_client::{Client, Paths, TransportKind};
use std::collections::HashMap;
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
    pub follows: Follows,
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
            follows: Arc::default(),
        })
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
