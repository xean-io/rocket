//! A yaml.v3-style node tree plus yaml.v3's plain-scalar resolution rules.
//!
//! `saphyr-parser` supplies events (with line numbers, anchors, tags and
//! scalar styles); this module folds them into a tree that mirrors
//! `yaml.Node` so the decoder can reproduce yaml.v3's behavior and messages.

use saphyr_parser::{Event, Parser, ScalarStyle};
use std::collections::HashMap;

pub(crate) type NodeId = usize;

pub(crate) const NULL_TAG: &str = "!!null";
pub(crate) const BOOL_TAG: &str = "!!bool";
pub(crate) const STR_TAG: &str = "!!str";
pub(crate) const INT_TAG: &str = "!!int";
pub(crate) const FLOAT_TAG: &str = "!!float";
pub(crate) const TIMESTAMP_TAG: &str = "!!timestamp";
pub(crate) const SEQ_TAG: &str = "!!seq";
pub(crate) const MAP_TAG: &str = "!!map";
pub(crate) const MERGE_TAG: &str = "!!merge";
const LONG_TAG_PREFIX: &str = "tag:yaml.org,2002:";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Scalar,
    Mapping,
    Sequence,
    Alias,
}

#[derive(Debug)]
pub(crate) struct Node {
    pub kind: Kind,
    /// yaml.v3 `Node.Tag`: short form; resolved for plain scalars, empty for
    /// aliases.
    pub tag: String,
    pub value: String,
    pub line: usize,
    pub content: Vec<NodeId>,
    pub alias: Option<NodeId>,
}

#[derive(Debug, Default)]
pub(crate) struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    pub(crate) fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Follows alias nodes to the aliased node.
    pub(crate) fn deref(&self, mut id: NodeId) -> NodeId {
        while let (Kind::Alias, Some(target)) = (self.nodes[id].kind, self.nodes[id].alias) {
            id = target;
        }
        id
    }

    /// yaml.v3 `Node.ShortTag`.
    pub(crate) fn short_tag(&self, id: NodeId) -> String {
        let n = &self.nodes[id];
        match n.kind {
            Kind::Alias => n.alias.map_or(String::new(), |t| self.short_tag(t)),
            _ => n.tag.clone(),
        }
    }

    pub(crate) fn is_null(&self, id: NodeId) -> bool {
        self.short_tag(id) == NULL_TAG
    }

    fn push(&mut self, node: Node) -> NodeId {
        self.nodes.push(node);
        self.nodes.len() - 1
    }
}

/// yaml.v3 `shortTag`.
fn short_tag(tag: &str) -> String {
    match tag.strip_prefix(LONG_TAG_PREFIX) {
        Some(rest) => format!("!!{rest}"),
        None => tag.to_string(),
    }
}

/// The value a plain scalar resolves to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Resolved {
    Null,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Str(String),
    Timestamp,
    Merge,
}

fn resolvable_tag(tag: &str) -> bool {
    matches!(
        tag,
        "" | STR_TAG | BOOL_TAG | INT_TAG | FLOAT_TAG | NULL_TAG | TIMESTAMP_TAG
    )
}

fn resolve_map(input: &str) -> Option<(&'static str, Resolved)> {
    Some(match input {
        "true" | "True" | "TRUE" => (BOOL_TAG, Resolved::Bool(true)),
        "false" | "False" | "FALSE" => (BOOL_TAG, Resolved::Bool(false)),
        "" | "~" | "null" | "Null" | "NULL" => (NULL_TAG, Resolved::Null),
        ".nan" | ".NaN" | ".NAN" => (FLOAT_TAG, Resolved::Float(f64::NAN)),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => {
            (FLOAT_TAG, Resolved::Float(f64::INFINITY))
        }
        "-.inf" | "-.Inf" | "-.INF" => (FLOAT_TAG, Resolved::Float(f64::NEG_INFINITY)),
        "<<" => (MERGE_TAG, Resolved::Merge),
        _ => return None,
    })
}

/// Go's `strconv.ParseInt(s, 0, 64)` / `ParseUint(s, 0, 64)` (no underscores:
/// the caller strips them like yaml.v3 does).
fn parse_int_base0(s: &str, signed: bool) -> Option<i128> {
    let (neg, body) = match s.as_bytes().first()? {
        b'-' if signed => (true, &s[1..]),
        b'+' if signed => (false, &s[1..]),
        _ => (false, s),
    };
    let (radix, digits) =
        if let Some(r) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16, r)
        } else if let Some(r) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            (2, r)
        } else if let Some(r) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            (8, r)
        } else if body.len() > 1 && body.starts_with('0') {
            (8, &body[1..])
        } else {
            (10, body)
        };
    if digits.is_empty() || digits.starts_with(['+', '-']) {
        return None;
    }
    let v = u64::from_str_radix(digits, radix).ok()?;
    let v = i128::from(v);
    let v = if neg { -v } else { v };
    if signed {
        (i128::from(i64::MIN)..=i128::from(i64::MAX))
            .contains(&v)
            .then_some(v)
    } else {
        Some(v)
    }
}

/// `^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$`
fn yaml_style_float(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    } else {
        if digits(&mut i) == 0 {
            return false;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            digits(&mut i);
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

fn two_digit(s: &str) -> Option<u32> {
    (!s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// A subset of yaml.v3's `parseTimestamp`: `YYYY-M-D`, optionally followed by
/// `T`/`t`/space and `H:M:S[.frac]` with an optional `Z` or `+hh:mm` zone.
fn parse_timestamp(s: &str) -> bool {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits != 4 || digits == s.len() || s.as_bytes()[digits] != b'-' {
        return false;
    }
    let year: u32 = s[..4].parse().unwrap_or(0);
    let rest = &s[5..];
    let (date, time) = match rest.find(['T', 't', ' ']) {
        Some(i) => (&rest[..i], Some((rest.as_bytes()[i], &rest[i + 1..]))),
        None => (rest, None),
    };
    let mut parts = date.split('-');
    let (Some(m), Some(d), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Some(month), Some(day)) = (two_digit(m), two_digit(d)) else {
        return false;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day {
        return false;
    }
    let Some((sep, time)) = time else {
        return true;
    };
    // Go's layouts: "T"/"t" allow a zone, the space layout does not.
    let (clock, zone) = if sep == b' ' {
        (time, "")
    } else if let Some(stripped) = time.strip_suffix(['Z', 'z']) {
        (stripped, "Z")
    } else if let Some(i) = time.rfind(['+', '-']) {
        (&time[..i], &time[i..])
    } else {
        return false;
    };
    if sep != b' ' && zone.is_empty() {
        return false;
    }
    let (hms, frac) = match clock.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (clock, None),
    };
    if frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let mut it = hms.split(':');
    let (Some(h), Some(mi), Some(sec), None) = (it.next(), it.next(), it.next(), it.next()) else {
        return false;
    };
    let ok_clock = matches!(
        (two_digit(h), two_digit(mi), two_digit(sec)),
        (Some(h), Some(mi), Some(sec)) if h < 24 && mi < 60 && sec < 60
    );
    if !ok_clock {
        return false;
    }
    if zone.is_empty() || zone == "Z" {
        return true;
    }
    let z = &zone[1..];
    match z.split_once(':') {
        Some((zh, zm)) => {
            zh.len() == 2
                && zm.len() == 2
                && zh.bytes().all(|b| b.is_ascii_digit())
                && zm.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

/// yaml.v3 `resolve(tag, in)`. `Err` carries the message of a fatal
/// "cannot decode ... as ..." failure (without the `yaml: ` prefix).
pub(crate) fn resolve(tag: &str, input: &str) -> Result<(String, Resolved), String> {
    let tag = short_tag(tag);
    if !resolvable_tag(&tag) {
        return Ok((tag, Resolved::Str(input.to_string())));
    }
    let (rtag, out) = resolve_inner(&tag, input);
    // Mirrors the deferred check in yaml.v3: an explicit tag must agree.
    if tag.is_empty() || tag == rtag || tag == STR_TAG {
        return Ok((rtag, out));
    }
    if tag == FLOAT_TAG {
        match out {
            Resolved::Int(v) if rtag == INT_TAG => {
                return Ok((FLOAT_TAG.to_string(), Resolved::Float(v as f64)));
            }
            Resolved::Uint(v) if rtag == INT_TAG => {
                return Ok((FLOAT_TAG.to_string(), Resolved::Float(v as f64)));
            }
            _ => {}
        }
    }
    Err(format!(
        "cannot decode {} `{}` as a {}",
        short_tag(&rtag),
        input,
        short_tag(&tag)
    ))
}

fn resolve_inner(tag: &str, input: &str) -> (String, Resolved) {
    let str_result = || (STR_TAG.to_string(), Resolved::Str(input.to_string()));
    if tag == STR_TAG {
        return str_result();
    }
    let hint = match input.bytes().next() {
        None => b'N',
        Some(b'+' | b'-') => b'S',
        Some(b'0'..=b'9') => b'D',
        Some(b'y' | b'Y' | b'n' | b'N' | b't' | b'T' | b'f' | b'F' | b'o' | b'O' | b'~') => b'M',
        Some(b'.') => b'.',
        Some(_) => 0,
    };
    if hint == 0 {
        return str_result();
    }
    if let Some((t, v)) = resolve_map(input) {
        return (t.to_string(), v);
    }
    match hint {
        b'.' => {
            if let Ok(f) = input.parse::<f64>()
                && f.is_finite()
            {
                return (FLOAT_TAG.to_string(), Resolved::Float(f));
            }
        }
        b'D' | b'S' => {
            if (tag.is_empty() || tag == TIMESTAMP_TAG) && parse_timestamp(input) {
                return (TIMESTAMP_TAG.to_string(), Resolved::Timestamp);
            }
            let plain = input.replace('_', "");
            if let Some(v) = parse_int_base0(&plain, true) {
                return (INT_TAG.to_string(), Resolved::Int(v as i64));
            }
            if let Some(v) = parse_int_base0(&plain, false)
                && let Ok(v) = u64::try_from(v)
            {
                return (INT_TAG.to_string(), Resolved::Uint(v));
            }
            if yaml_style_float(&plain)
                && let Ok(f) = plain.parse::<f64>()
                && f.is_finite()
            {
                return (FLOAT_TAG.to_string(), Resolved::Float(f));
            }
        }
        _ => {}
    }
    str_result()
}

/// Builds the node tree of the first document. `Ok(None)` means the stream
/// holds no document (Go's `io.EOF`).
pub(crate) fn parse_tree(src: &str) -> Result<Option<(Tree, NodeId)>, String> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let chars: Vec<char> = src.chars().collect();
    let mut anchor_names: HashMap<usize, String> = HashMap::new();
    let parser = Parser::new_from_str(src);
    let mut tree = Tree::default();
    let mut stack: Vec<NodeId> = Vec::new();
    let mut anchors: HashMap<usize, NodeId> = HashMap::new();
    let mut root: Option<NodeId> = None;
    let mut in_document = false;

    let fail = |e: saphyr_parser::ScanError| {
        if e.info().contains("unknown anchor")
            && let Some(name) = alias_name_near(&chars, e.marker().index())
        {
            return format!("yaml: unknown anchor '{name}' referenced");
        }
        // libyaml reports the "while parsing X" context separately from the
        // problem text yaml.v3 prints.
        let info = e.info();
        let problem = match info.strip_prefix("while ").and_then(|r| r.split_once(", ")) {
            Some((_, rest)) => rest,
            None => info,
        };
        format!("yaml: line {}: {}", e.marker().line(), problem)
    };

    for next in parser {
        let (event, span) = next.map_err(fail)?;
        let line = span.start.line();
        match event {
            Event::DocumentStart(_) => in_document = true,
            Event::DocumentEnd | Event::StreamEnd => {
                if root.is_some() || !in_document {
                    break;
                }
            }
            Event::Alias(id) => {
                let target = anchors
                    .get(&id)
                    .copied()
                    .ok_or_else(|| format!("yaml: line {line}: unknown anchor referenced"))?;
                let n = tree.push(Node {
                    kind: Kind::Alias,
                    tag: String::new(),
                    value: anchor_names
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| format!("#{id}")),
                    line,
                    content: Vec::new(),
                    alias: Some(target),
                });
                attach(&mut tree, &stack, &mut root, n);
            }
            Event::Scalar(value, style, anchor, tag) => {
                let props = usize::from(anchor != 0) + usize::from(tag.is_some());
                let (first, name) = node_props(&chars, span.start.index(), props);
                let line = if props > 0 {
                    line_of(&chars, first)
                } else {
                    line
                };
                if let (true, Some(name)) = (anchor != 0, name) {
                    anchor_names.insert(anchor, name);
                }
                let explicit = explicit_tag(tag.as_deref());
                let value = value.into_owned();
                let node_tag = match explicit {
                    Some(t) => t,
                    None if style != ScalarStyle::Plain => STR_TAG.to_string(),
                    None if value == "<<" => MERGE_TAG.to_string(),
                    None => match resolve("", &value) {
                        Ok((t, _)) => t,
                        Err(_) => STR_TAG.to_string(),
                    },
                };
                let n = tree.push(Node {
                    kind: Kind::Scalar,
                    tag: node_tag,
                    value,
                    line,
                    content: Vec::new(),
                    alias: None,
                });
                if anchor != 0 {
                    anchors.insert(anchor, n);
                }
                attach(&mut tree, &stack, &mut root, n);
            }
            Event::SequenceStart(anchor, tag) => {
                let props = usize::from(anchor != 0) + usize::from(tag.is_some());
                let (first, name) = node_props(&chars, span.start.index(), props);
                let line = if props > 0 {
                    line_of(&chars, first)
                } else {
                    line
                };
                if let (true, Some(name)) = (anchor != 0, name) {
                    anchor_names.insert(anchor, name);
                }
                let explicit = explicit_tag(tag.as_deref());
                let n = tree.push(Node {
                    kind: Kind::Sequence,
                    tag: explicit.unwrap_or_else(|| SEQ_TAG.to_string()),
                    value: String::new(),
                    line,
                    content: Vec::new(),
                    alias: None,
                });
                if anchor != 0 {
                    anchors.insert(anchor, n);
                }
                attach(&mut tree, &stack, &mut root, n);
                stack.push(n);
            }
            Event::MappingStart(anchor, tag) => {
                let props = usize::from(anchor != 0) + usize::from(tag.is_some());
                let (first, name) = node_props(&chars, span.start.index(), props);
                let line = if props > 0 {
                    line_of(&chars, first)
                } else {
                    line
                };
                if let (true, Some(name)) = (anchor != 0, name) {
                    anchor_names.insert(anchor, name);
                }
                let explicit = explicit_tag(tag.as_deref());
                let n = tree.push(Node {
                    kind: Kind::Mapping,
                    tag: explicit.unwrap_or_else(|| MAP_TAG.to_string()),
                    value: String::new(),
                    line,
                    content: Vec::new(),
                    alias: None,
                });
                if anchor != 0 {
                    anchors.insert(anchor, n);
                }
                attach(&mut tree, &stack, &mut root, n);
                stack.push(n);
            }
            Event::SequenceEnd | Event::MappingEnd => {
                stack.pop();
            }
            Event::StreamStart | Event::Nothing => {}
        }
        if root.is_some() && stack.is_empty() {
            break;
        }
    }
    Ok(root.map(|r| (tree, r)))
}

/// saphyr reports node spans that start after the anchor and tag, and anchor
/// ids instead of names. yaml.v3 (libyaml) starts a node at its first
/// property, so walk back over up to `props` `&anchor` / `!tag` tokens to
/// recover the node's start line and the anchor name.
fn node_props(chars: &[char], start: usize, props: usize) -> (usize, Option<String>) {
    let mut first = start.min(chars.len());
    let mut name = None;
    for _ in 0..props {
        let mut end = first;
        while end > 0 && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        let mut begin = end;
        while begin > 0
            && !chars[begin - 1].is_whitespace()
            && !matches!(chars[begin - 1], ',' | '[' | ']' | '{' | '}')
        {
            begin -= 1;
        }
        match chars.get(begin) {
            Some('&') if begin < end => {
                name = Some(chars[begin + 1..end].iter().collect());
                first = begin;
            }
            Some('!') if begin < end => first = begin,
            _ => break,
        }
    }
    (first, name)
}

/// The `*name` alias token at (or just before) `index`.
fn alias_name_near(chars: &[char], index: usize) -> Option<String> {
    let mut i = index.min(chars.len().saturating_sub(1));
    loop {
        if chars.get(i) == Some(&'*') {
            let name: String = chars[i + 1..]
                .iter()
                .take_while(|c| !c.is_whitespace() && !matches!(c, ',' | '[' | ']' | '{' | '}'))
                .collect();
            return Some(name);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn line_of(chars: &[char], index: usize) -> usize {
    1 + chars[..index.min(chars.len())]
        .iter()
        .filter(|c| **c == '\n')
        .count()
}

fn explicit_tag(tag: Option<&saphyr_parser::Tag>) -> Option<String> {
    tag.map(|t| short_tag(&format!("{}{}", t.handle, t.suffix)))
        .filter(|t| !t.is_empty() && t != "!")
}

fn attach(tree: &mut Tree, stack: &[NodeId], root: &mut Option<NodeId>, n: NodeId) {
    match stack.last() {
        Some(&parent) => tree.nodes[parent].content.push(n),
        None => *root = Some(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_like_yaml_v3() {
        let tag = |s: &str| resolve("", s).unwrap().0;
        assert_eq!(tag("1"), INT_TAG);
        assert_eq!(tag("0755"), INT_TAG);
        assert_eq!(tag("089"), FLOAT_TAG);
        assert_eq!(tag("1_000"), INT_TAG);
        assert_eq!(tag("1.5"), FLOAT_TAG);
        assert_eq!(tag("true"), BOOL_TAG);
        assert_eq!(tag("off"), STR_TAG);
        assert_eq!(tag("~"), NULL_TAG);
        assert_eq!(tag(""), NULL_TAG);
        assert_eq!(tag("2001-12-14"), TIMESTAMP_TAG);
        assert_eq!(tag("1e999"), STR_TAG);
        assert_eq!(tag("12abc"), STR_TAG);
    }

    #[test]
    fn builds_tree_with_anchors() {
        let (tree, root) = parse_tree("a: &x 1\nb: *x\n").unwrap().unwrap();
        let n = tree.get(root);
        assert_eq!(n.kind, Kind::Mapping);
        assert_eq!(n.content.len(), 4);
        assert_eq!(tree.get(n.content[3]).kind, Kind::Alias);
        assert_eq!(tree.deref(n.content[3]), n.content[1]);
    }

    #[test]
    fn empty_stream_has_no_document() {
        assert!(parse_tree("# only a comment\n").unwrap().is_none());
        assert!(parse_tree("").unwrap().is_none());
    }
}
