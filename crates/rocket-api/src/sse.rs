//! Server-Sent Events plumbing: frame encoding, the streaming response and
//! the heartbeat. Every stream is one spawned task that owns the only writer
//! of its response body, so frames can never interleave (README: "Heartbeat,
//! log and state frames are serialized"). Backpressure is a small bounded
//! channel: a slow client slows its own task, never the daemon.

use crate::json;
use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::Response;
use bytes::Bytes;
use rocket_domain::Event;
use std::convert::Infallible;
use std::future::Future;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Frames buffered between the stream task and the HTTP body.
const BUFFER: usize = 16;

/// `event: <type>\ndata: <json>\n\n`. The payload is Go `json.Marshal`
/// output, so it is HTML-escaped.
pub(crate) fn event_frame(e: &Event) -> Bytes {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(b"event: ");
    out.extend_from_slice(e.r#type.as_bytes());
    out.extend_from_slice(b"\ndata: ");
    out.extend_from_slice(&json::compact_html(e));
    out.extend_from_slice(b"\n\n");
    Bytes::from(out)
}

pub(crate) fn comment_frame(text: &str) -> Bytes {
    Bytes::from(format!(": {text}\n\n"))
}

/// The write half handed to a stream task.
pub(crate) struct Writer {
    tx: mpsc::Sender<Bytes>,
}

impl Writer {
    /// Sends one `event:` frame; `false` when the client is gone.
    pub async fn event(&self, e: &Event) -> bool {
        self.tx.send(event_frame(e)).await.is_ok()
    }

    /// Sends one `: text` comment frame; `false` when the client is gone.
    pub async fn comment(&self, text: &str) -> bool {
        self.tx.send(comment_frame(text)).await.is_ok()
    }
}

/// Starts `work` as the stream's task and returns the `200 text/event-stream`
/// response immediately, already carrying the opening `: ok` comment. The
/// task ends (dropping everything it owns, e.g. a bus subscription) as soon
/// as it finishes, the client disconnects, or `closing` fires; when it ends
/// the response body ends too.
pub(crate) fn response<F, Fut>(closing: CancellationToken, work: F) -> Response
where
    F: FnOnce(Writer) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Bytes>(BUFFER);
    // Some clients (URLSession) hold the response until body bytes arrive.
    let _ = tx.try_send(comment_frame("ok"));
    let probe = tx.clone();
    let task = work(Writer { tx });
    tokio::spawn(async move {
        tokio::select! {
            () = task => {}
            () = probe.closed() => {}
            () = closing.cancelled() => {}
        }
    });
    let stream = futures_util::stream::poll_fn(move |cx| {
        rx.poll_recv(cx).map(|b| b.map(Ok::<Bytes, Infallible>))
    });
    let mut resp = Response::new(Body::from_stream(stream));
    *resp.status_mut() = StatusCode::OK;
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
    h.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    h.insert(header::CONNECTION, "keep-alive".parse().unwrap());
    resp
}
