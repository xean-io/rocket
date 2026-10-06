//! `{service.port}` tokens in service environment values (Go: `port_refs.go`).

use crate::project::Service;
use std::collections::BTreeMap;

/// A service environment token. The final dot separates the service name from
/// its named port; whitespace is not allowed inside tokens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PortReference {
    pub token: String,
    pub service: String,
    pub port: String,
}

/// Go's RE2 `\s`: ASCII whitespace only.
fn is_re2_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

/// Byte ranges of every match of Go's `\{[^{}\s]+\}`, leftmost and
/// non-overlapping. All delimiters are ASCII, so slicing stays on char
/// boundaries.
fn token_ranges(value: &str) -> Vec<(usize, usize)> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && !matches!(bytes[j], b'{' | b'}') && !is_re2_space(bytes[j]) {
            j += 1;
        }
        if j > i + 1 && j < bytes.len() && bytes[j] == b'}' {
            out.push((i, j + 1));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Whether the match at `start` is a shell `${...}` expression.
fn is_shell_expr(value: &str, start: usize) -> bool {
    start > 0 && value.as_bytes()[start - 1] == b'$'
}

impl PortReference {
    /// Expands this token while preserving shell `${...}` expressions.
    pub fn replace(&self, value: &str, replacement: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut cursor = 0;
        for (start, end) in token_ranges(value) {
            if value[start..end] != self.token || is_shell_expr(value, start) {
                continue;
            }
            out.push_str(&value[cursor..start]);
            out.push_str(replacement);
            cursor = end;
        }
        out.push_str(&value[cursor..]);
        out
    }
}

fn sorted(refs: BTreeMap<String, PortReference>) -> Vec<PortReference> {
    refs.into_values().collect()
}

/// Unique references in lexical order. Shell `${...}` expressions and brace
/// text without a dot remain literal environment values.
pub fn port_references(value: &str) -> Vec<PortReference> {
    let mut refs = BTreeMap::new();
    for (start, end) in token_ranges(value) {
        if is_shell_expr(value, start) {
            continue;
        }
        let token = &value[start..end];
        let name = &token[1..token.len() - 1];
        let Some(i) = name.rfind('.') else { continue };
        if i == 0 || i == name.len() - 1 {
            continue;
        }
        refs.insert(
            token.to_string(),
            PortReference {
                token: token.to_string(),
                service: name[..i].to_string(),
                port: name[i + 1..].to_string(),
            },
        );
    }
    sorted(refs)
}

impl Service {
    /// The unique references declared in the service's env.
    pub fn port_references(&self) -> Vec<PortReference> {
        let mut refs = BTreeMap::new();
        for value in self.env.values() {
            for r in port_references(value) {
                refs.insert(r.token.clone(), r);
            }
        }
        sorted(refs)
    }
}
