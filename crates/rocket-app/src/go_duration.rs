//! Go `time.ParseDuration` and `Duration.String`, for TTL requests and
//! error messages that must read like the Go daemon's.

use std::time::Duration;

const MAX_I64_PLUS_1: u64 = 1 << 63;

fn leading_int(s: &[u8]) -> Option<(u64, &[u8])> {
    let mut x: u64 = 0;
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        if x > MAX_I64_PLUS_1 / 10 {
            return None;
        }
        x = x * 10 + u64::from(s[i] - b'0');
        if x > MAX_I64_PLUS_1 {
            return None;
        }
        i += 1;
    }
    Some((x, &s[i..]))
}

fn leading_fraction(s: &[u8]) -> (u64, f64, &[u8]) {
    let mut x: u64 = 0;
    let mut scale = 1.0;
    let mut overflow = false;
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        if !overflow {
            if x > (u64::MAX >> 1) / 10 {
                overflow = true;
            } else {
                let y = x * 10 + u64::from(s[i] - b'0');
                if y > MAX_I64_PLUS_1 {
                    overflow = true;
                } else {
                    x = y;
                    scale *= 10.0;
                }
            }
        }
        i += 1;
    }
    (x, scale, &s[i..])
}

fn unit_nanos(unit: &str) -> Option<u64> {
    Some(match unit {
        "ns" => 1,
        "us" | "\u{00b5}s" | "\u{03bc}s" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" => 60 * 1_000_000_000,
        "h" => 3600 * 1_000_000_000,
        _ => return None,
    })
}

/// `time.ParseDuration`, returning nanoseconds (negative allowed) or `None`
/// where Go returns an error.
pub(crate) fn parse_duration(input: &str) -> Option<i128> {
    let mut s = input.as_bytes();
    let mut neg = false;
    if let Some(&c) = s.first()
        && (c == b'-' || c == b'+')
    {
        neg = c == b'-';
        s = &s[1..];
    }
    if s == b"0" {
        return Some(0);
    }
    if s.is_empty() {
        return None;
    }
    let mut d: u64 = 0;
    while !s.is_empty() {
        if !(s[0] == b'.' || s[0].is_ascii_digit()) {
            return None;
        }
        let before = s.len();
        let (mut v, rest) = leading_int(s)?;
        s = rest;
        let pre = before != s.len();
        let mut f: u64 = 0;
        let mut scale = 1.0;
        let mut post = false;
        if !s.is_empty() && s[0] == b'.' {
            s = &s[1..];
            let before = s.len();
            let (ff, sc, rest) = leading_fraction(s);
            f = ff;
            scale = sc;
            s = rest;
            post = before != s.len();
        }
        if !pre && !post {
            return None;
        }
        let end = s
            .iter()
            .position(|&c| c == b'.' || c.is_ascii_digit())
            .unwrap_or(s.len());
        if end == 0 {
            return None;
        }
        let unit = unit_nanos(std::str::from_utf8(&s[..end]).ok()?)?;
        s = &s[end..];
        if v > MAX_I64_PLUS_1 / unit {
            return None;
        }
        v *= unit;
        if f > 0 {
            v += (f as f64 * (unit as f64 / scale)) as u64;
            if v > MAX_I64_PLUS_1 {
                return None;
            }
        }
        d = d.checked_add(v)?;
        if d > MAX_I64_PLUS_1 {
            return None;
        }
    }
    if neg {
        return Some(-i128::from(d));
    }
    if d > MAX_I64_PLUS_1 - 1 {
        return None;
    }
    Some(i128::from(d))
}

/// `value / 10^digits` with the fraction trimmed of trailing zeros.
fn fmt_frac(value: u128, digits: u32) -> String {
    let scale = 10u128.pow(digits);
    let (int, frac) = (value / scale, value % scale);
    if frac == 0 {
        return int.to_string();
    }
    let frac = format!("{frac:0width$}", width = digits as usize);
    format!("{int}.{}", frac.trim_end_matches('0'))
}

/// Go's `Duration.String` for a non-negative duration: `10ms`, `1.5s`,
/// `1m0s`, `1h2m3s`.
pub(crate) fn format_duration(d: Duration) -> String {
    let ns = d.as_nanos();
    if ns == 0 {
        return "0s".into();
    }
    if ns < 1_000 {
        return format!("{ns}ns");
    }
    if ns < 1_000_000 {
        return format!("{}\u{b5}s", fmt_frac(ns, 3));
    }
    if ns < 1_000_000_000 {
        return format!("{}ms", fmt_frac(ns, 6));
    }
    let secs = fmt_frac(ns % 60_000_000_000, 9);
    let total_minutes = ns / 60_000_000_000;
    let (hours, minutes) = (total_minutes / 60, total_minutes % 60);
    if hours > 0 {
        format!("{hours}h{minutes}m{secs}s")
    } else if minutes > 0 {
        format!("{minutes}m{secs}s")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_go() {
        assert_eq!(parse_duration("30m"), Some(1_800_000_000_000));
        assert_eq!(parse_duration("1h30m"), Some(5_400_000_000_000));
        assert_eq!(parse_duration("-1m"), Some(-60_000_000_000));
        assert_eq!(parse_duration("0s"), Some(0));
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration("90"), None);
    }

    #[test]
    fn formats_like_go() {
        let cases = [
            (Duration::from_millis(10), "10ms"),
            (Duration::from_secs(60), "1m0s"),
            (Duration::from_secs(5), "5s"),
            (Duration::from_millis(1500), "1.5s"),
            (Duration::from_secs(3723), "1h2m3s"),
            (Duration::from_micros(1500), "1.5ms"),
            (Duration::ZERO, "0s"),
        ];
        for (d, want) in cases {
            assert_eq!(format_duration(d), want);
        }
    }
}
