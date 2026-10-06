//! In-memory, non-blocking pub/sub for daemon events (Go: `adapters/events`).

use rocket_domain::Event;
use rocket_domain::ports::{EventBus, Subscription};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::mpsc;

/// Buffer used when a subscriber asks for `0` (Go: `buffer <= 0`).
const DEFAULT_BUFFER: usize = 256;

#[derive(Default)]
struct Inner {
    next: usize,
    subs: HashMap<usize, mpsc::Sender<Event>>,
}

/// An [`EventBus`] whose slow subscribers drop events instead of blocking
/// publishers. Cloning shares the same bus.
#[derive(Clone, Default)]
pub struct Bus {
    inner: Arc<Mutex<Inner>>,
}

impl Bus {
    /// An empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live subscriptions.
    pub fn subscriber_count(&self) -> usize {
        lock(&self.inner).subs.len()
    }
}

fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(PoisonError::into_inner)
}

impl EventBus for Bus {
    /// Delivers `event` to every subscriber with buffer space.
    fn publish(&self, event: Event) {
        let inner = lock(&self.inner);
        for tx in inner.subs.values() {
            // Full (or closed) subscriber: drop the event, never block.
            let _ = tx.try_send(event.clone());
        }
    }

    /// Registers a subscriber; dropping the [`Subscription`] unsubscribes.
    fn subscribe(&self, buffer: usize) -> Subscription {
        let buffer = if buffer == 0 { DEFAULT_BUFFER } else { buffer };
        let (tx, rx) = mpsc::channel(buffer);
        let id = {
            let mut inner = lock(&self.inner);
            let id = inner.next;
            inner.next += 1;
            inner.subs.insert(id, tx);
            id
        };
        let bus = Arc::clone(&self.inner);
        Subscription::new(rx, move || {
            lock(&bus).subs.remove(&id);
        })
    }
}
