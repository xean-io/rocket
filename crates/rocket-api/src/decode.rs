//! Request-body decoding with Go `json.Decoder` + `DisallowUnknownFields`
//! semantics and, where observable, Go's error texts.
//!
//! Go reads the first JSON value of the body (anything after it is ignored),
//! validates its whole syntax up front, then decodes it into the target
//! struct: field names match case-insensitively, `null` is a no-op, the first
//! unknown field or type mismatch (in document order) is reported. That is
//! reproduced in three steps: a port of Go's scanner state machine
//! ([`scan_value`]) for the syntax errors, a document-ordered walk checking
//! every key against a per-request [`Schema`], and finally serde for the typed
//! value (all keys are canonicalized by then, so serde's own
//! `deny_unknown_fields` can no longer trigger).
//!
//! Remaining difference: Go's numeric type mismatches (e.g. a float for an
//! integer field) are not modelled because the request types only contain
//! strings, string lists and booleans.

use serde::Deserialize;
use serde::de::{DeserializeOwned, MapAccess, Visitor};
use serde_json::{Map, Value};
use std::fmt;

/// JSON type of one request field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    Str,
    Bool,
    StrList,
    /// A string-typed Go named type (`domain.JobKind`), reported by name.
    Named(&'static str),
}

/// The Go request struct a body is decoded into.
#[derive(Debug, Clone, Copy)]
pub struct Schema {
    /// Qualified Go type, e.g. `app.UpRequest`.
    pub go_type: &'static str,
    /// JSON field names in Go struct order with their types.
    pub fields: &'static [(&'static str, Ty)],
}

/// `POST /v1/up` and `POST /v1/restart`.
pub const UP: Schema = Schema {
    go_type: "app.UpRequest",
    fields: &[
        ("project", Ty::Str),
        ("services", Ty::StrList),
        ("env", Ty::Str),
        ("profiles", Ty::StrList),
        ("owner", Ty::Str),
        ("ttl", Ty::Str),
    ],
};

/// `POST /v1/down`.
pub const DOWN: Schema = Schema {
    go_type: "app.DownRequest",
    fields: &[
        ("project", Ty::Str),
        ("services", Ty::StrList),
        ("owner", Ty::Str),
        ("everywhere", Ty::Bool),
    ],
};

/// `POST /v1/projects`.
pub const ADD_PROJECT: Schema = Schema {
    go_type: "api.AddProjectRequest",
    fields: &[("path", Ty::Str)],
};

/// `POST /v1/jobs`.
pub const JOB: Schema = Schema {
    go_type: "app.JobRequest",
    fields: &[
        ("project", Ty::Str),
        ("kind", Ty::Named("domain.JobKind")),
        ("name", Ty::Str),
        ("env", Ty::Str),
        ("profiles", Ty::StrList),
        ("ttl", Ty::Str),
        ("args", Ty::StrList),
        ("owner", Ty::Str),
        ("yes", Ty::Bool),
        ("allow_agent_deploy", Ty::Bool),
    ],
};

/// Decodes `body` into `T`. `Err` carries the Go error text (without the
/// `invalid request: bad JSON body: ` prefix the handlers add).
pub fn decode<T: DeserializeOwned>(body: &[u8], schema: &Schema) -> Result<T, String> {
    let object = decode_object(body, schema)?;
    serde_json::from_value(Value::Object(object)).map_err(|e| e.to_string())
}

/// Validates `body` against `schema` and returns its fields under their
/// canonical names. `null` fields are dropped and a `null` body yields an
/// empty object (Go leaves the struct zeroed).
pub fn decode_object(body: &[u8], schema: &Schema) -> Result<Map<String, Value>, String> {
    let end = match scan_value(body) {
        Ok(end) => end,
        Err(ScanError::Eof) => return Err("EOF".into()),
        Err(ScanError::UnexpectedEof) => return Err("unexpected EOF".into()),
        Err(ScanError::Syntax(msg)) => return Err(msg),
    };
    let value = &body[..end];
    let first = value
        .iter()
        .find(|b| !is_space(**b))
        .copied()
        .unwrap_or(b' ');
    match first {
        b'{' => {}
        b'n' => return Ok(Map::new()),
        _ => {
            let kind = match first {
                b'[' => "array",
                b'"' => "string",
                b't' | b'f' => "bool",
                _ => "number",
            };
            return Err(format!(
                "json: cannot unmarshal {kind} into Go value of type {}",
                schema.go_type
            ));
        }
    }
    let Ordered(entries) = serde_json::from_slice(value).map_err(|e| e.to_string())?;
    let go_struct = schema.go_type.rsplit('.').next().unwrap_or(schema.go_type);
    let mut out = Map::new();
    for (key, val) in entries {
        let Some((name, ty)) = schema.fields.iter().find(|(n, _)| *n == key).or_else(|| {
            schema
                .fields
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&key))
        }) else {
            return Err(format!("json: unknown field {key:?}"));
        };
        check_type(go_struct, name, *ty, &val)?;
        if !val.is_null() {
            out.insert((*name).to_string(), val);
        }
    }
    Ok(out)
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn check_type(go_struct: &str, field: &str, ty: Ty, v: &Value) -> Result<(), String> {
    let mismatch = |got: &Value, want: &str| {
        Err(format!(
            "json: cannot unmarshal {} into Go struct field {go_struct}.{field} of type {want}",
            kind_name(got)
        ))
    };
    match (ty, v) {
        (_, Value::Null)
        | (Ty::Str | Ty::Named(_), Value::String(_))
        | (Ty::Bool, Value::Bool(_)) => Ok(()),
        (Ty::Str, other) => mismatch(other, "string"),
        (Ty::Named(name), other) => mismatch(other, name),
        (Ty::Bool, other) => mismatch(other, "bool"),
        (Ty::StrList, Value::Array(items)) => items
            .iter()
            .find(|i| !matches!(i, Value::String(_) | Value::Null))
            .map_or(Ok(()), |bad| mismatch(bad, "string")),
        (Ty::StrList, other) => mismatch(other, "[]string"),
    }
}

/// The top-level object's entries in document order (duplicates kept).
struct Ordered(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V)
    }
}

// --- Go scanner port (encoding/json/scanner.go) -------------------------

/// Why [`scan_value`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    /// Only whitespace (Go `io.EOF`).
    Eof,
    /// The value is cut short (Go `io.ErrUnexpectedEOF`).
    UnexpectedEof,
    /// `invalid character ...`.
    Syntax(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    BeginValue,
    BeginValueOrEmpty,
    BeginStringOrEmpty,
    BeginString,
    EndValue,
    EndTop,
    InString,
    InStringEsc,
    InStringEscU(u8),
    Neg,
    Zero,
    Digits,
    Dot,
    DotDigits,
    E,
    ESign,
    EDigits,
    Lit(&'static [u8], &'static str, usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    ObjectKey,
    ObjectValue,
    ArrayValue,
}

enum Step {
    Next(St),
    /// The value ended before this byte (Go `scanEnd`).
    End,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

/// Returns the length of the first JSON value in `b` (leading whitespace
/// included, the byte that terminates a bare scalar excluded) like
/// `Decoder.readValue`, or Go's error for it.
pub fn scan_value(b: &[u8]) -> Result<usize, ScanError> {
    let mut st = St::BeginValue;
    let mut stack: Vec<Ctx> = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        match step(st, &mut stack, c).map_err(ScanError::Syntax)? {
            Step::End => return Ok(i),
            Step::Next(next) => {
                st = next;
                if st == St::EndTop {
                    return Ok(i + 1);
                }
            }
        }
    }
    // EOF: a bare number or a completed scalar needs no terminator.
    let complete = stack.is_empty()
        && matches!(
            st,
            St::EndValue | St::Zero | St::Digits | St::DotDigits | St::EDigits
        );
    if complete {
        Ok(b.len())
    } else if b.iter().any(|c| !is_space(*c)) {
        Err(ScanError::UnexpectedEof)
    } else {
        Err(ScanError::Eof)
    }
}

fn err(c: u8, context: &str) -> String {
    format!("invalid character {} {context}", quote_char(c))
}

/// Go `quoteChar`.
fn quote_char(c: u8) -> String {
    match c {
        b'\'' => "'\\''".into(),
        b'"' => "'\"'".into(),
        b'\\' => "'\\\\'".into(),
        0x07 => "'\\a'".into(),
        0x08 => "'\\b'".into(),
        0x0c => "'\\f'".into(),
        b'\n' => "'\\n'".into(),
        b'\r' => "'\\r'".into(),
        b'\t' => "'\\t'".into(),
        0x0b => "'\\v'".into(),
        0x00..=0x1f | 0x7f => format!("'\\x{c:02x}'"),
        0x80..=0xa0 | 0xad => format!("'\\u{c:04x}'"),
        _ => format!("'{}'", char::from(c)),
    }
}

fn after_pop(stack: &[Ctx]) -> St {
    if stack.is_empty() {
        St::EndTop
    } else {
        St::EndValue
    }
}

/// Go `stateEndValue`: `c` follows a completed value.
fn end_value(stack: &mut Vec<Ctx>, c: u8) -> Result<Step, String> {
    let Some(top) = stack.last().copied() else {
        return Ok(Step::End);
    };
    if is_space(c) {
        return Ok(Step::Next(St::EndValue));
    }
    let last = stack.len() - 1;
    match (top, c) {
        (Ctx::ObjectKey, b':') => {
            stack[last] = Ctx::ObjectValue;
            Ok(Step::Next(St::BeginValue))
        }
        (Ctx::ObjectKey, _) => Err(err(c, "after object key")),
        (Ctx::ObjectValue, b',') => {
            stack[last] = Ctx::ObjectKey;
            Ok(Step::Next(St::BeginString))
        }
        (Ctx::ObjectValue, b'}') => {
            stack.pop();
            Ok(Step::Next(after_pop(stack)))
        }
        (Ctx::ObjectValue, _) => Err(err(c, "after object key:value pair")),
        (Ctx::ArrayValue, b',') => Ok(Step::Next(St::BeginValue)),
        (Ctx::ArrayValue, b']') => {
            stack.pop();
            Ok(Step::Next(after_pop(stack)))
        }
        (Ctx::ArrayValue, _) => Err(err(c, "after array element")),
    }
}

fn begin_value(stack: &mut Vec<Ctx>, c: u8) -> Result<Step, String> {
    if is_space(c) {
        return Ok(Step::Next(St::BeginValue));
    }
    let next = match c {
        b'{' => {
            stack.push(Ctx::ObjectKey);
            St::BeginStringOrEmpty
        }
        b'[' => {
            stack.push(Ctx::ArrayValue);
            St::BeginValueOrEmpty
        }
        b'"' => St::InString,
        b'-' => St::Neg,
        b'0' => St::Zero,
        b'1'..=b'9' => St::Digits,
        b't' => St::Lit(b"rue", "true", 0),
        b'f' => St::Lit(b"alse", "false", 0),
        b'n' => St::Lit(b"ull", "null", 0),
        _ => return Err(err(c, "looking for beginning of value")),
    };
    Ok(Step::Next(next))
}

fn step(st: St, stack: &mut Vec<Ctx>, c: u8) -> Result<Step, String> {
    match st {
        St::BeginValue => begin_value(stack, c),
        St::BeginValueOrEmpty => {
            if is_space(c) {
                Ok(Step::Next(St::BeginValueOrEmpty))
            } else if c == b']' {
                end_value(stack, c)
            } else {
                begin_value(stack, c)
            }
        }
        St::BeginStringOrEmpty => {
            if is_space(c) {
                Ok(Step::Next(St::BeginStringOrEmpty))
            } else if c == b'}' {
                if let Some(top) = stack.last_mut() {
                    *top = Ctx::ObjectValue;
                }
                end_value(stack, c)
            } else {
                step(St::BeginString, stack, c)
            }
        }
        St::BeginString => {
            if is_space(c) {
                Ok(Step::Next(St::BeginString))
            } else if c == b'"' {
                Ok(Step::Next(St::InString))
            } else {
                Err(err(c, "looking for beginning of object key string"))
            }
        }
        St::EndValue => end_value(stack, c),
        St::EndTop => Ok(Step::End),
        St::InString => Ok(Step::Next(match c {
            b'"' => St::EndValue,
            b'\\' => St::InStringEsc,
            0..=0x1f => return Err(err(c, "in string literal")),
            _ => St::InString,
        })),
        St::InStringEsc => match c {
            b'b' | b'f' | b'n' | b'r' | b't' | b'\\' | b'/' | b'"' => Ok(Step::Next(St::InString)),
            b'u' => Ok(Step::Next(St::InStringEscU(0))),
            _ => Err(err(c, "in string escape code")),
        },
        St::InStringEscU(n) => {
            if c.is_ascii_hexdigit() {
                Ok(Step::Next(if n == 3 {
                    St::InString
                } else {
                    St::InStringEscU(n + 1)
                }))
            } else {
                Err(err(c, "in \\u hexadecimal character escape"))
            }
        }
        St::Neg => match c {
            b'0' => Ok(Step::Next(St::Zero)),
            b'1'..=b'9' => Ok(Step::Next(St::Digits)),
            _ => Err(err(c, "in numeric literal")),
        },
        St::Digits if c.is_ascii_digit() => Ok(Step::Next(St::Digits)),
        St::Digits | St::Zero => match c {
            b'.' => Ok(Step::Next(St::Dot)),
            b'e' | b'E' => Ok(Step::Next(St::E)),
            _ => end_value(stack, c),
        },
        St::Dot => {
            if c.is_ascii_digit() {
                Ok(Step::Next(St::DotDigits))
            } else {
                Err(err(c, "after decimal point in numeric literal"))
            }
        }
        St::DotDigits => match c {
            b'0'..=b'9' => Ok(Step::Next(St::DotDigits)),
            b'e' | b'E' => Ok(Step::Next(St::E)),
            _ => end_value(stack, c),
        },
        St::E | St::ESign => {
            if st == St::E && (c == b'+' || c == b'-') {
                Ok(Step::Next(St::ESign))
            } else if c.is_ascii_digit() {
                Ok(Step::Next(St::EDigits))
            } else {
                Err(err(c, "in exponent of numeric literal"))
            }
        }
        St::EDigits => {
            if c.is_ascii_digit() {
                Ok(Step::Next(St::EDigits))
            } else {
                end_value(stack, c)
            }
        }
        St::Lit(rest, word, n) => {
            if c == rest[n] {
                Ok(Step::Next(if n + 1 == rest.len() {
                    St::EndValue
                } else {
                    St::Lit(rest, word, n + 1)
                }))
            } else {
                Err(err(
                    c,
                    &format!("in literal {word} (expecting {})", quote_char(rest[n])),
                ))
            }
        }
    }
}
