//! Typed daemon events and the stream adapter over an SSE response body.

use crate::error::ClientError;
use crate::sse::{SseFrame, SseMessage, SseParser};
use crate::transport::AbortOnDrop;
use futures_util::Stream;
use futures_util::stream;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use rocket_domain::{Event, event_type};
use std::collections::VecDeque;
use std::pin::Pin;

/// A stream of daemon events. It ends when the server closes the connection
/// (or, for job-log follow, right after the terminal `job.state`). Dropping
/// it closes the connection. An `Err` item is either a payload that failed
/// to decode (the stream continues) or a transport failure (it ends).
pub type EventStream = Pin<Box<dyn Stream<Item = Result<DaemonEvent, ClientError>> + Send>>;

/// One event of `GET /v1/events` and the log-follow streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonEvent {
    ServiceState(Event),
    LogLine(Event),
    PortLeased(Event),
    PortReleased(Event),
    JobState(Event),
    JobLog(Event),
    /// An event name this client does not know, kept verbatim.
    Raw(SseMessage),
}

impl DaemonEvent {
    /// Decodes a dispatched SSE message by its `event:` name. A bare
    /// `message` event is dispatched on the payload's `type` instead.
    pub fn from_message(msg: &SseMessage) -> Result<Self, ClientError> {
        let mut name = msg.event.as_str();
        let mut pre_parsed = None;
        if name == "message" {
            if let Ok(e) = serde_json::from_str::<Event>(&msg.data) {
                pre_parsed = Some(e);
            } else {
                return Ok(Self::Raw(msg.clone()));
            }
            name = pre_parsed.as_ref().map_or("message", |e| e.r#type.as_str());
        }
        let ctor: fn(Event) -> Self = match name {
            event_type::SERVICE_STATE => Self::ServiceState,
            event_type::LOG_LINE => Self::LogLine,
            event_type::PORT_LEASED => Self::PortLeased,
            event_type::PORT_RELEASED => Self::PortReleased,
            event_type::JOB_STATE => Self::JobState,
            event_type::JOB_LOG => Self::JobLog,
            _ => return Ok(Self::Raw(msg.clone())),
        };
        let event = match pre_parsed {
            Some(e) => e,
            None => serde_json::from_str(&msg.data)?,
        };
        Ok(ctor(event))
    }

    /// The decoded payload; `None` for [`DaemonEvent::Raw`].
    pub fn event(&self) -> Option<&Event> {
        match self {
            Self::ServiceState(e)
            | Self::LogLine(e)
            | Self::PortLeased(e)
            | Self::PortReleased(e)
            | Self::JobState(e)
            | Self::JobLog(e) => Some(e),
            Self::Raw(_) => None,
        }
    }

    /// A `job.state` whose job finished (status taken from `status`, else
    /// from the embedded job).
    pub fn is_terminal_job_state(&self) -> bool {
        match self {
            Self::JobState(e) => e
                .status
                .or_else(|| e.job.as_ref().map(|j| j.status))
                .is_some_and(|s| s.terminal()),
            _ => false,
        }
    }
}

struct State {
    body: Incoming,
    parser: SseParser,
    pending: VecDeque<Result<DaemonEvent, ClientError>>,
    done: bool,
    until_terminal_job_state: bool,
    _guard: AbortOnDrop,
}

/// Turns an SSE response into an [`EventStream`]. With
/// `until_terminal_job_state` the stream ends after the first terminal
/// `job.state` and reports a [`ClientError::Protocol`] if the server closes
/// before sending one.
pub(crate) fn event_stream(
    body: Incoming,
    guard: AbortOnDrop,
    until_terminal_job_state: bool,
) -> EventStream {
    let state = State {
        body,
        parser: SseParser::new(),
        pending: VecDeque::new(),
        done: false,
        until_terminal_job_state,
        _guard: guard,
    };
    Box::pin(stream::unfold(state, |mut st| async move {
        loop {
            if let Some(item) = st.pending.pop_front() {
                return Some((item, st));
            }
            if st.done {
                return None;
            }
            match st.body.frame().await {
                Some(Ok(frame)) => {
                    let Ok(data) = frame.into_data() else {
                        continue;
                    };
                    for f in st.parser.feed(&data) {
                        let SseFrame::Message(m) = f else { continue };
                        let ev = DaemonEvent::from_message(&m);
                        let terminal = st.until_terminal_job_state
                            && ev.as_ref().is_ok_and(DaemonEvent::is_terminal_job_state);
                        st.pending.push_back(ev);
                        if terminal {
                            st.done = true;
                            break;
                        }
                    }
                }
                Some(Err(e)) => {
                    st.pending.push_back(Err(ClientError::Http(e.to_string())));
                    st.done = true;
                }
                None => {
                    if st.until_terminal_job_state {
                        st.pending.push_back(Err(ClientError::Protocol(
                            "job log stream closed before its final state".into(),
                        )));
                    }
                    st.done = true;
                }
            }
        }
    }))
}
