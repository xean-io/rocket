//! Small re-implementations of Go standard-library behavior that leaks into
//! manifest error messages: `strconv.Quote` (`%q`/`%#v`), `time.ParseDuration`,
//! `filepath.Base` and `filepath.Clean`.

use std::path::{Component, Path, PathBuf};

/// Whether Go's `unicode.IsPrint` would keep `c` unescaped in `strconv.Quote`.
fn is_print(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_control() || c.is_whitespace() {
        return false;
    }
    // Common format (Cf) characters, which Go does not consider printable.
    !matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0xE000..=0xF8FF)
}

/// `strconv.Quote`: the output of Go's `%q` (and `%#v` for strings).
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{07}' => out.push_str("\\a"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0b}' => out.push_str("\\v"),
            c if is_print(c) => out.push(c),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if (c as u32) < 0x10000 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    out.push('"');
    out
}

/// `filepath.Base`.
pub(crate) fn base(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    let trimmed = path.trim_end_matches(sep);
    if trimmed.is_empty() {
        return path.chars().next().map(String::from).unwrap_or_default();
    }
    match trimmed.rfind(sep) {
        Some(i) => trimmed[i + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

/// Lexical `filepath.Clean` (no symlink resolution).
pub(crate) fn clean(path: &Path) -> PathBuf {
    let mut out: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(comp),
            },
            other => out.push(other),
        }
    }
    if out.is_empty() {
        return PathBuf::from(".");
    }
    out.iter().collect()
}

/// `filepath.Abs`.
pub(crate) fn abs(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        return Ok(clean(path));
    }
    Ok(clean(&std::env::current_dir()?.join(path)))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_matches_go() {
        assert_eq!(quote("a\"b\\c\n\t"), r#""a\"b\\c\n\t""#);
        assert_eq!(quote("h\u{e9}llo \u{1F680}"), "\"h\u{e9}llo \u{1F680}\"");
        assert_eq!(quote("\u{0}\u{7f}\u{a0}"), "\"\\x00\\x7f\\u00a0\"");
    }

    #[test]
    fn base_matches_go() {
        assert_eq!(base("/code/my-app"), "my-app");
        assert_eq!(base("/code/my-app///"), "my-app");
        assert_eq!(base(""), ".");
        assert_eq!(base("/"), "/");
    }

    #[test]
    fn durations_match_go() {
        assert_eq!(parse_duration("90s"), Some(90_000_000_000));
        assert_eq!(parse_duration("1h30m"), Some(5_400_000_000_000));
        assert_eq!(parse_duration("1.5s"), Some(1_500_000_000));
        assert_eq!(parse_duration("-5s"), Some(-5_000_000_000));
        assert_eq!(parse_duration("0"), Some(0));
        assert_eq!(parse_duration("90"), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("1\u{b5}s"), Some(1_000));
    }

    #[test]
    fn clean_resolves_dots() {
        assert_eq!(clean(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
        assert_eq!(clean(Path::new("/..")), PathBuf::from("/"));
    }
}
