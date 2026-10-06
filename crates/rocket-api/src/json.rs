//! Byte-exact emulation of the Go daemon's JSON encoders.
//!
//! * [`pretty`]: `writeJSON`, i.e. `json.Encoder` with `SetIndent("", "  ")`
//!   and `SetEscapeHTML(false)`; every API response body except the 401.
//! * [`compact_html`]: `json.Marshal`, used for SSE `data:` payloads. Go's
//!   default `EscapeHTML` is on, so `<`, `>` and `&` become `<`, `>`
//!   and `&`.
//! * [`compact_html_line`]: `json.NewEncoder(w).Encode(v)` with defaults, used
//!   by the 401 body: compact, HTML-escaped and newline-terminated.
//!
//! `serde_json` already matches Go for everything else (separators, 2-space
//! indentation, `[]`/`{}` for empty containers, `\b` `\f` `\n` `\r` `\t` and
//! lowercase `\u00xx` escapes, unescaped DEL and non-ASCII). The only
//! differences are U+2028 and U+2029, which Go always escapes (even with
//! `EscapeHTML(false)`), and the three HTML characters above. All of these
//! can only occur inside JSON strings, so a byte-level rewrite of the encoded
//! output is safe and cheap.

use serde::Serialize;

/// Go `writeJSON`: indented with two spaces, no HTML escaping, trailing newline.
pub fn pretty<T: Serialize + ?Sized>(v: &T) -> Vec<u8> {
    let encoded = serde_json::to_vec_pretty(v).unwrap_or_default();
    let mut out = escape(&encoded, false);
    out.push(b'\n');
    out
}

/// Go `json.Marshal`: compact, HTML-escaped, no trailing newline.
pub fn compact_html<T: Serialize + ?Sized>(v: &T) -> Vec<u8> {
    escape(&serde_json::to_vec(v).unwrap_or_default(), true)
}

/// Go `json.NewEncoder(w).Encode(v)` with defaults: [`compact_html`] plus `\n`.
pub fn compact_html_line<T: Serialize + ?Sized>(v: &T) -> Vec<u8> {
    let mut out = compact_html(v);
    out.push(b'\n');
    out
}

fn escape(src: &[u8], html: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'<' if html => out.extend_from_slice(b"\\u003c"),
            b'>' if html => out.extend_from_slice(b"\\u003e"),
            b'&' if html => out.extend_from_slice(b"\\u0026"),
            // U+2028 = E2 80 A8, U+2029 = E2 80 A9. 0xE2 is never a
            // continuation byte, so this cannot split another character.
            0xE2 if src.get(i + 1) == Some(&0x80)
                && matches!(src.get(i + 2), Some(0xA8 | 0xA9)) =>
            {
                out.extend_from_slice(if src[i + 2] == 0xA8 {
                    b"\\u2028"
                } else {
                    b"\\u2029"
                });
                i += 3;
                continue;
            }
            b => out.push(b),
        }
        i += 1;
    }
    out
}
