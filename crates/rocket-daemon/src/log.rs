//! Daemon logging the way Go's `log` package does it: one line per message on
//! stderr (`rocket daemon start` appends stderr to `rocketd.log`), prefixed
//! with `YYYY/MM/DD HH:MM:SS`.

use std::io::Write;
use std::sync::OnceLock;
use time::{OffsetDateTime, UtcOffset};

static OFFSET: OnceLock<UtcOffset> = OnceLock::new();

/// Captures the local UTC offset. The `time` crate only reads it soundly
/// while the process is single-threaded, so binaries call this first thing
/// in `main`, before starting the async runtime; without it timestamps are
/// UTC.
pub fn init_local_offset() {
    if let Ok(offset) = UtcOffset::current_local_offset() {
        let _ = OFFSET.set(offset);
    }
}

fn stamp(now: OffsetDateTime) -> String {
    let fmt = time::macros::format_description!("[year]/[month]/[day] [hour]:[minute]:[second]");
    now.to_offset(OFFSET.get().copied().unwrap_or(UtcOffset::UTC))
        .format(&fmt)
        .unwrap_or_default()
}

/// Writes one log line to stderr (Go: `log.Printf`).
pub(crate) fn line(msg: &str) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "{} {msg}",
        stamp(OffsetDateTime::now_utc())
    );
}

macro_rules! logf {
    ($($arg:tt)*) => { $crate::log::line(&format!($($arg)*)) };
}
pub(crate) use logf;
