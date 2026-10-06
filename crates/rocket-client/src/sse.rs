//! Incremental Server-Sent Events parser (WHATWG event-stream rules).

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseMessage {
    /// The `event:` name, `message` when absent.
    pub event: String,
    /// `data:` lines joined with `\n`.
    pub data: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseFrame {
    Message(SseMessage),
    /// A `:` comment line (rocketd sends `: ok` first, then `: ping` every 15s).
    Comment(String),
    Retry(u64),
}

/// Feed it raw byte chunks of any size; complete frames are returned.
/// LF, CRLF and lone CR line endings are accepted, and multi-byte UTF-8
/// split across chunks is handled (lines are decoded once complete).
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    last_was_cr: bool,
    event: String,
    data: Vec<String>,
    has_data: bool,
    last_id: Option<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
        let mut frames = Vec::new();
        for &b in bytes {
            match b {
                b'\n' => {
                    if self.last_was_cr {
                        self.last_was_cr = false; // CRLF: the CR already ended the line
                    } else {
                        self.end_line(&mut frames);
                    }
                }
                b'\r' => {
                    self.last_was_cr = true;
                    self.end_line(&mut frames);
                }
                _ => {
                    self.last_was_cr = false;
                    self.buffer.push(b);
                }
            }
        }
        frames
    }

    fn end_line(&mut self, frames: &mut Vec<SseFrame>) {
        let line = String::from_utf8_lossy(&self.buffer).into_owned();
        self.buffer.clear();

        if line.is_empty() {
            if self.has_data {
                frames.push(SseFrame::Message(SseMessage {
                    event: if self.event.is_empty() {
                        "message".to_owned()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    data: self.data.join("\n"),
                    id: self.last_id.clone(),
                }));
            }
            self.event.clear();
            self.data.clear();
            self.has_data = false;
            return;
        }
        if let Some(comment) = line.strip_prefix(':') {
            frames.push(SseFrame::Comment(
                comment.strip_prefix(' ').unwrap_or(comment).to_owned(),
            ));
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_str(), ""),
        };
        match field {
            "event" => self.event = value.to_owned(),
            "data" => {
                self.data.push(value.to_owned());
                self.has_data = true;
            }
            "id" if !value.contains('\0') => self.last_id = Some(value.to_owned()),
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    frames.push(SseFrame::Retry(ms));
                }
            }
            _ => {}
        }
    }
}
