//! Log-follow streams pushed to the frontend over `tauri::ipc::Channel`s.

use crate::dto::FollowMessage;
use crate::state::AppState;
use futures_util::StreamExt;
use rocket_client::{ClientError, DaemonEvent, EventStream};
use tauri::ipc::Channel;

/// Pumps `stream` into `channel` on a background task and returns its follow
/// id. The task unregisters itself when the stream ends, the channel closes
/// or `stop_follow` aborts it.
pub fn spawn(state: &AppState, mut stream: EventStream, channel: Channel<FollowMessage>) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let follows = state.follows.clone();
    let task_id = id.clone();
    // Hold the lock across spawn + insert so a task that finishes at once
    // cannot unregister itself before it is registered.
    let mut map = state.follows.lock().expect("follows lock");
    let handle = tauri::async_runtime::spawn(async move {
        let end = loop {
            match stream.next().await {
                None => break FollowMessage::End,
                Some(Ok(DaemonEvent::Raw(_))) => {}
                Some(Ok(ev)) => {
                    if let Some(event) = ev.event()
                        && channel
                            .send(FollowMessage::Event {
                                event: Box::new(event.clone()),
                            })
                            .is_err()
                    {
                        break FollowMessage::End;
                    }
                }
                Some(Err(e @ ClientError::Decode(_))) => tracing::warn!("skipping log event: {e}"),
                Some(Err(e)) => {
                    break FollowMessage::Error {
                        message: e.to_string(),
                    };
                }
            }
        };
        let _ = channel.send(end);
        follows.lock().expect("follows lock").remove(&task_id);
    });
    map.insert(id.clone(), handle.inner().abort_handle());
    id
}
