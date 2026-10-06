//! Query strings with Go's `url.Values` and `strconv` semantics.

/// A parsed query string. Mirrors `net/url.ParseQuery` followed by
/// `Values.Get`: the first value of a key wins, a missing key is `""`, and
/// pairs with a bad percent escape or a semicolon are silently dropped.
#[derive(Debug, Default, Clone)]
pub struct Query(Vec<(String, String)>);

impl Query {
    /// Parses the raw (still percent-encoded) query of a request URI.
    pub fn parse(raw: Option<&str>) -> Self {
        let mut pairs = Vec::new();
        for part in raw.unwrap_or("").split('&') {
            if part.is_empty() || part.contains(';') {
                continue;
            }
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            if let (Some(k), Some(v)) = (unescape(key), unescape(value)) {
                pairs.push((k, v));
            }
        }
        Self(pairs)
    }

    /// Go `Values.Get`.
    pub fn get(&self, name: &str) -> &str {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v.as_str())
    }

    /// Go `strconv.ParseBool` with errors folded to `false` (`boolParam`).
    pub fn flag(&self, name: &str) -> bool {
        matches!(self.get(name), "1" | "t" | "T" | "TRUE" | "true" | "True")
    }

    /// Go `strconv.Atoi`: `None` when the value is absent or not an integer.
    pub fn int(&self, name: &str) -> Option<i64> {
        self.get(name).parse().ok()
    }
}

/// Go `url.QueryUnescape`.
fn unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hi = hex(*bytes.get(i + 1)?)?;
                let lo = hex(*bytes.get(i + 2)?)?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

fn hex(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}
