//! Go `encoding/json` byte compatibility of the API encoders. The expected
//! strings come from the Go daemon (`writeJSON`: indent, no HTML escaping;
//! SSE `json.Marshal` and the 401 body: HTML escaping), with U+2028/U+2029
//! escaped by both.

use rocket_api::json::{compact_html, compact_html_line, pretty};
use serde_json::json;

#[test]
fn pretty_indents_with_two_spaces_and_ends_with_newline() {
    let out = pretty(&json!({"a": 1, "b": ["x", "y"], "c": {}, "d": []}));
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"a\": 1,\n  \"b\": [\n    \"x\",\n    \"y\"\n  ],\n  \"c\": {},\n  \"d\": []\n}\n"
    );
}

#[test]
fn pretty_leaves_html_characters_alone_but_escapes_line_separators() {
    // Go: SetEscapeHTML(false) keeps <, > and &; U+2028/2029 are always escaped.
    let out = pretty(&json!({"s": "<a href=\"x\">&</a>\u{2028}\u{2029}é€😀"}));
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\n  \"s\": \"<a href=\\\"x\\\">&</a>\\u2028\\u2029é€😀\"\n}\n"
    );
}

#[test]
fn control_characters_use_go_escapes() {
    let out = compact_html(&json!("\u{8}\u{c}\n\r\t\u{1}\u{1f}\u{7f}"));
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "\"\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f}\""
    );
}

#[test]
fn compact_html_escapes_like_json_marshal() {
    let out = compact_html(&json!({"line": "<&>\u{2028}\u{2029}", "n": 1}));
    assert_eq!(
        String::from_utf8(out).unwrap(),
        r#"{"line":"\u003c\u0026\u003e\u2028\u2029","n":1}"#
    );
}

#[test]
fn compact_html_line_adds_the_encoder_newline() {
    let out = compact_html_line(&json!({"error": "a<b", "code": "x"}));
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "{\"code\":\"x\",\"error\":\"a\\u003cb\"}\n"
    );
}

#[test]
fn html_escaping_applies_to_object_keys_too() {
    let out = compact_html(&json!({"<k>": 1}));
    assert_eq!(String::from_utf8(out).unwrap(), r#"{"\u003ck\u003e":1}"#);
}
