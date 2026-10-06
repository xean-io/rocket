//! Output helpers: Go-compatible JSON, durations and the small cell
//! formatters shared by the table printers.

use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::OnceLock;
use time::{OffsetDateTime, UtcOffset};

/// Writes to stdout without panicking on a closed pipe; `false` when the
/// write failed (streaming callers stop then).
pub fn out(text: &str) -> bool {
    let mut o = std::io::stdout().lock();
    o.write_all(text.as_bytes())
        .and_then(|()| o.flush())
        .is_ok()
}

/// `println`, panic-free.
pub fn outln(text: &str) -> bool {
    out(&format!("{text}\n"))
}

/// Writes to stderr and ignores failures.
pub fn err(text: &str) {
    let mut o = std::io::stderr().lock();
    let _ = o.write_all(text.as_bytes());
}

pub fn errln(text: &str) {
    err(&format!("{text}\n"));
}

/// Go always escapes U+2028 and U+2029 in JSON strings.
fn escape_line_separators(s: String) -> String {
    if s.contains(['\u{2028}', '\u{2029}']) {
        s.replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    } else {
        s
    }
}

/// `printJSON`: two-space indent, no HTML escaping, trailing newline.
pub fn json_pretty<T: Serialize + ?Sized>(v: &T) -> String {
    let s = serde_json::to_string_pretty(v).unwrap_or_else(|e| format!("\"{e}\""));
    format!("{}\n", escape_line_separators(s))
}

pub fn print_json<T: Serialize + ?Sized>(v: &T) {
    out(&json_pretty(v));
}

/// `printJSONLine`: Go's `json.Marshal`, compact and HTML-escaped.
pub fn json_line<T: Serialize + ?Sized>(v: &T) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    let s = escape_line_separators(s);
    s.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// The `{"code", "error"}` body printed for a failure under `--json`
/// (Go marshals a `map[string]string`, so keys are sorted).
pub fn json_error(message: &str, code: &str) -> String {
    let mut m = BTreeMap::new();
    m.insert("error", message);
    m.insert("code", code);
    json_pretty(&m)
}

/// `name=port,name=port` sorted by name, `-` when empty.
pub fn format_ports(ports: &BTreeMap<String, u16>) -> String {
    if ports.is_empty() {
        return "-".into();
    }
    ports
        .iter()
        .map(|(n, p)| format!("{n}={p}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn or_dash(s: &str) -> String {
    if s.is_empty() { "-".into() } else { s.into() }
}

pub fn pid_str(pid: i32) -> String {
    if pid == 0 {
        "-".into()
    } else {
        pid.to_string()
    }
}

pub fn exit_str(code: Option<i32>) -> String {
    code.map_or_else(|| "-".into(), |c| c.to_string())
}

const NS_PER_MS: i128 = 1_000_000;
const NS_PER_SEC: i128 = 1_000_000_000;

/// Go's `time.Duration.String()` for a nanosecond count.
pub fn go_duration(ns: i128) -> String {
    if ns == 0 {
        return "0s".into();
    }
    let neg = ns < 0;
    let u = ns.unsigned_abs();
    let body = if u < NS_PER_SEC as u128 {
        if u < 1_000 {
            format!("{u}ns")
        } else if u < 1_000_000 {
            format!("{}\u{b5}s", with_frac(u / 1_000, u % 1_000, 3))
        } else {
            format!("{}ms", with_frac(u / 1_000_000, u % 1_000_000, 6))
        }
    } else {
        let frac = u % NS_PER_SEC as u128;
        let total_secs = u / NS_PER_SEC as u128;
        let secs = with_frac(total_secs % 60, frac, 9);
        let total_mins = total_secs / 60;
        if total_mins == 0 {
            format!("{secs}s")
        } else if total_mins < 60 {
            format!("{total_mins}m{secs}s")
        } else {
            format!("{}h{}m{secs}s", total_mins / 60, total_mins % 60)
        }
    };
    if neg { format!("-{body}") } else { body }
}

/// `v` plus `.frac` (zero-padded to `digits`, trailing zeros trimmed).
fn with_frac(v: u128, frac: u128, digits: usize) -> String {
    if frac == 0 {
        return v.to_string();
    }
    let f = format!("{frac:0digits$}");
    format!("{v}.{}", f.trim_end_matches('0'))
}

/// Go's `Duration.Round(m)`: halfway rounds away from zero.
pub fn round_duration(d: i128, m: i128) -> i128 {
    if m <= 0 {
        return d;
    }
    let mut r = d % m;
    if d < 0 {
        r = -r;
        if r + r < m {
            return d + r;
        }
        return d - m + r;
    }
    if r + r < m {
        return d - r;
    }
    d + m - r
}

/// `durationStr`: milliseconds rounded to 100ms, Go style.
pub fn duration_str(ms: i64) -> String {
    go_duration(round_duration(i128::from(ms) * NS_PER_MS, 100 * NS_PER_MS))
}

/// `expiresStr`: time left rounded to the second, `expired` when past.
pub fn expires_str(t: Option<OffsetDateTime>, now: OffsetDateTime) -> String {
    let Some(t) = t else { return "-".into() };
    let d = round_duration((t - now).whole_nanoseconds(), NS_PER_SEC);
    if d < 0 {
        "expired".into()
    } else {
        go_duration(d)
    }
}

pub fn expires_str_now(t: Option<OffsetDateTime>) -> String {
    expires_str(t, OffsetDateTime::now_utc())
}

static LOCAL_OFFSET: OnceLock<UtcOffset> = OnceLock::new();

/// Captures the local UTC offset. The `time` crate reads it soundly only
/// while the process is single-threaded: call this first thing in `main`.
pub fn init_local_offset() {
    if let Ok(o) = UtcOffset::current_local_offset() {
        let _ = LOCAL_OFFSET.set(o);
    }
}

/// `t.Local().Format("01-02 15:04:05")`.
pub fn local_stamp(t: OffsetDateTime) -> String {
    let fmt = time::macros::format_description!("[month]-[day] [hour]:[minute]:[second]");
    t.to_offset(LOCAL_OFFSET.get().copied().unwrap_or(UtcOffset::UTC))
        .format(&fmt)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_duration_strings() {
        let ms = NS_PER_MS;
        let s = NS_PER_SEC;
        for (ns, want) in [
            (0, "0s"),
            (1, "1ns"),
            (1_500, "1.5\u{b5}s"),
            (250 * ms, "250ms"),
            (1_234_567, "1.234567ms"),
            (s, "1s"),
            (1_500 * ms, "1.5s"),
            (59 * s + 900 * ms, "59.9s"),
            (60 * s, "1m0s"),
            (29 * 60 * s + 59 * s, "29m59s"),
            (3600 * s, "1h0m0s"),
            (3600 * s + 62 * s, "1h1m2s"),
            (-2 * s, "-2s"),
            (-1_500 * ms, "-1.5s"),
        ] {
            assert_eq!(go_duration(ns), want, "{ns}");
        }
    }

    #[test]
    fn rounding_matches_go() {
        let ms = NS_PER_MS;
        let s = NS_PER_SEC;
        assert_eq!(round_duration(1_499 * ms, s), s);
        assert_eq!(round_duration(1_500 * ms, s), 2 * s);
        assert_eq!(round_duration(-1_500 * ms, s), -2 * s);
        assert_eq!(round_duration(-1_499 * ms, s), -s);
        assert_eq!(round_duration(-400 * ms, s), 0);
        assert_eq!(round_duration(7, 0), 7);
    }

    #[test]
    fn duration_str_rounds_to_a_tenth_of_a_second() {
        assert_eq!(duration_str(0), "0s");
        assert_eq!(duration_str(49), "0s");
        assert_eq!(duration_str(50), "100ms");
        assert_eq!(duration_str(1_234), "1.2s");
        assert_eq!(duration_str(125_000), "2m5s");
    }

    #[test]
    fn expires_str_cases() {
        let now = OffsetDateTime::UNIX_EPOCH;
        assert_eq!(expires_str(None, now), "-");
        assert_eq!(
            expires_str(Some(now + time::Duration::seconds(90)), now),
            "1m30s"
        );
        assert_eq!(
            expires_str(Some(now - time::Duration::seconds(3)), now),
            "expired"
        );
        // -400ms rounds to zero, which is not yet "expired" (Go checks after Round).
        assert_eq!(
            expires_str(Some(now - time::Duration::milliseconds(400)), now),
            "0s"
        );
    }

    #[test]
    fn json_helpers_match_go() {
        #[derive(Serialize)]
        struct T {
            a: &'static str,
            b: Vec<i32>,
            c: Vec<i32>,
        }
        let t = T {
            a: "x<y>&z\u{2028}",
            b: vec![],
            c: vec![1],
        };
        assert_eq!(
            json_pretty(&t),
            "{\n  \"a\": \"x<y>&z\\u2028\",\n  \"b\": [],\n  \"c\": [\n    1\n  ]\n}\n"
        );
        assert_eq!(
            json_line(&t),
            "{\"a\":\"x\\u003cy\\u003e\\u0026z\\u2028\",\"b\":[],\"c\":[1]}"
        );
        assert_eq!(
            json_error("boom", "invalid"),
            "{\n  \"code\": \"invalid\",\n  \"error\": \"boom\"\n}\n"
        );
    }

    #[test]
    fn cell_formatters() {
        let ports: BTreeMap<String, u16> =
            [("web".to_owned(), 80), ("api".to_owned(), 8080)].into();
        assert_eq!(format_ports(&ports), "api=8080,web=80");
        assert_eq!(format_ports(&BTreeMap::new()), "-");
        assert_eq!(or_dash(""), "-");
        assert_eq!(pid_str(0), "-");
        assert_eq!(pid_str(42), "42");
        assert_eq!(exit_str(None), "-");
        assert_eq!(exit_str(Some(3)), "3");
    }
}
