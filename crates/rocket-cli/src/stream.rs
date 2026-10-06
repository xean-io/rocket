//! Following SSE streams until they end or the user interrupts.

use crate::error::Result;
use futures_util::StreamExt;
use rocket_client::{ClientError, DaemonEvent, EventStream};
use rocket_domain::Event;

/// Resolves when the process receives SIGINT or SIGTERM (Ctrl-C elsewhere).
pub struct Interrupt {
    #[cfg(unix)]
    sigint: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
}

impl Interrupt {
    /// Must be created inside the tokio runtime.
    #[cfg(unix)]
    pub fn new() -> Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            sigint: signal(SignalKind::interrupt())?,
            sigterm: signal(SignalKind::terminate())?,
        })
    }

    #[cfg(not(unix))]
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    #[cfg(unix)]
    pub async fn wait(&mut self) {
        tokio::select! {
            _ = self.sigint.recv() => {}
            _ = self.sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    pub async fn wait(&mut self) {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// How [`follow`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// The server closed the stream (or the job reached its final state).
    Ended,
    Interrupted,
    /// The callback asked to stop (stdout closed).
    Stopped,
}

/// The decoded event of a stream item. Events with a name this client does
/// not know are decoded from their payload anyway, like the Go client does.
fn event_of(e: &DaemonEvent) -> Option<Event> {
    match e {
        DaemonEvent::Raw(m) => serde_json::from_str(&m.data).ok(),
        other => other.event().cloned(),
    }
}

/// Feeds every event to `f` until the stream ends, `f` returns `false` or a
/// signal arrives. Undecodable payloads are skipped; a stream that closes
/// before its final event counts as ended (Go returns nil there).
pub async fn follow(
    mut stream: EventStream,
    interrupt: &mut Interrupt,
    mut f: impl FnMut(&Event) -> bool,
) -> Result<Followed> {
    loop {
        tokio::select! {
            () = interrupt.wait() => return Ok(Followed::Interrupted),
            item = stream.next() => match item {
                None | Some(Err(ClientError::Protocol(_))) => return Ok(Followed::Ended),
                Some(Err(ClientError::Decode(_))) => {}
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(ev)) => {
                    if let Some(e) = event_of(&ev) && !f(&e) {
                        return Ok(Followed::Stopped);
                    }
                }
            },
        }
    }
}
