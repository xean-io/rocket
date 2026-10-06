//! A port of the parts of yaml.v3's decoder that `internal/manifest` relies
//! on, specialised to the raw manifest types: strict unknown-field checks
//! (`KnownFields(true)`), duplicate-key detection, merge keys, anchors and
//! aliases, `UnmarshalYAML` hooks and yaml.v3's aggregated `TypeError`
//! messages.

use crate::gostd::quote;
use crate::node::{
    Kind, MAP_TAG, MERGE_TAG, NodeId, Resolved, SEQ_TAG, STR_TAG, Tree, parse_tree, resolve,
};
use rocket_domain::PortEnvBinding;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

/// Aborts decoding (yaml.v3 `fail`/`failf` panics): the message is final.
#[derive(Debug)]
pub(crate) struct Fatal(pub String);

type Res<T> = Result<T, Fatal>;

fn failf<T>(msg: impl AsRef<str>) -> Res<T> {
    Err(Fatal(format!("yaml: {}", msg.as_ref())))
}

fn fatal<T>(msg: String) -> Res<T> {
    Err(Fatal(msg))
}

const BINARY_TAG: &str = "!!binary";

/// Go's `base64.StdEncoding.DecodeString` (ignores `\r` and `\n`).
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut padding = 0;
    for c in input.bytes().filter(|b| !matches!(b, b'\r' | b'\n')) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            _ => return None,
        };
        if padding > 0 {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    let data_chars = input
        .bytes()
        .filter(|b| !matches!(b, b'\r' | b'\n' | b'='))
        .count();
    (data_chars + padding).is_multiple_of(4).then_some(out)
}

type Merged = Rc<RefCell<HashSet<String>>>;

pub(crate) struct Decoder<'a> {
    tree: &'a Tree,
    pub(crate) terrors: Vec<String>,
    aliases: HashSet<NodeId>,
    known_fields: bool,
    decode_count: usize,
    alias_count: usize,
    alias_depth: usize,
    merged: Option<Merged>,
}

fn allowed_alias_ratio(decode_count: usize) -> f64 {
    const LOW: f64 = 400_000.0;
    const HIGH: f64 = 4_000_000.0;
    let c = decode_count as f64;
    if c <= LOW {
        0.99
    } else if c >= HIGH {
        0.10
    } else {
        0.99 - 0.89 * ((c - LOW) / (HIGH - LOW))
    }
}

fn truncate_bytes(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(tree: &'a Tree, known_fields: bool) -> Self {
        Self {
            tree,
            terrors: Vec::new(),
            aliases: HashSet::new(),
            known_fields,
            decode_count: 0,
            alias_count: 0,
            alias_depth: 0,
            merged: None,
        }
    }

    fn count(&mut self) -> Res<()> {
        self.decode_count += 1;
        if self.alias_depth > 0 {
            self.alias_count += 1;
        }
        if self.alias_count > 100
            && self.decode_count > 1000
            && (self.alias_count as f64) / (self.decode_count as f64)
                > allowed_alias_ratio(self.decode_count)
        {
            return failf("document contains excessive aliasing");
        }
        Ok(())
    }

    fn terror(&mut self, id: NodeId, tag: &str, ty: &str) {
        let n = self.tree.get(id);
        let tag = if n.tag.is_empty() {
            tag
        } else {
            n.tag.as_str()
        };
        let value = if tag != SEQ_TAG && tag != MAP_TAG {
            if n.value.len() > 10 {
                format!(" `{}...`", truncate_bytes(&n.value, 7))
            } else {
                format!(" `{}`", n.value)
            }
        } else {
            String::new()
        };
        self.terrors.push(format!(
            "line {}: cannot unmarshal {}{} into {}",
            n.line, tag, value, ty
        ));
    }

    fn alias<T: Yaml>(&mut self, id: NodeId, out: &mut T) -> Res<bool> {
        let n = self.tree.get(id);
        if self.aliases.contains(&id) {
            return failf(format!("anchor '{}' value contains itself", n.value));
        }
        let Some(target) = n.alias else {
            return Ok(false);
        };
        self.aliases.insert(id);
        self.alias_depth += 1;
        let good = T::decode(self, target, out);
        self.alias_depth -= 1;
        self.aliases.remove(&id);
        good
    }

    /// yaml.v3 `d.unmarshal` for a type without a pointer or `yaml.Node` hook.
    fn unmarshal_default<T: Yaml>(&mut self, id: NodeId, out: &mut T) -> Res<bool> {
        self.count()?;
        let tree = self.tree;
        let kind = tree.get(id).kind;
        if kind == Kind::Alias {
            return self.alias(id, out);
        }
        if !tree.is_null(id)
            && let Some(result) = T::unmarshaler(self, id, out)
        {
            return result;
        }
        match kind {
            Kind::Scalar => self.scalar(id, out),
            Kind::Mapping => self.mapping(id, out),
            Kind::Sequence => T::sequence(self, id, out),
            Kind::Alias => Ok(false),
        }
    }

    fn scalar<T: Yaml>(&mut self, id: NodeId, out: &mut T) -> Res<bool> {
        let n = self.tree.get(id);
        let (tag, resolved) = if n.tag == STR_TAG {
            (STR_TAG.to_string(), Resolved::Str(n.value.clone()))
        } else {
            match resolve(&n.tag, &n.value) {
                Ok(r) => r,
                Err(msg) => return failf(msg),
            }
        };
        if resolved == Resolved::Null {
            return Ok(T::null(out));
        }
        // `!!binary` scalars are base64; strings receive the decoded bytes.
        let text = if tag == BINARY_TAG {
            match decode_base64(&n.value) {
                Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                None => return failf("!!binary value contains invalid base64 data"),
            }
        } else {
            n.value.clone()
        };
        if T::set_scalar(out, &resolved, &text) {
            return Ok(true);
        }
        self.terror(id, &tag, &T::go_type());
        Ok(false)
    }

    fn mapping<T: Yaml>(&mut self, id: NodeId, out: &mut T) -> Res<bool> {
        let tree = self.tree;
        let n = tree.get(id);
        let l = n.content.len();
        let nerrs = self.terrors.len();
        for i in (0..l).step_by(2) {
            let ni = tree.get(n.content[i]);
            for j in ((i + 2)..l).step_by(2) {
                let nj = tree.get(n.content[j]);
                if ni.kind == nj.kind && ni.value == nj.value {
                    self.terrors.push(format!(
                        "line {}: mapping key {} already defined at line {}",
                        nj.line,
                        quote(&nj.value),
                        ni.line
                    ));
                }
            }
        }
        if self.terrors.len() > nerrs {
            return Ok(false);
        }
        T::mapping(self, id, out)
    }

    /// Whether a mapping key is a `<<` merge key.
    fn is_merge(&self, id: NodeId) -> bool {
        let n = self.tree.get(id);
        n.kind == Kind::Scalar
            && n.value == "<<"
            && (n.tag.is_empty() || n.tag == "!" || n.tag == MERGE_TAG)
    }

    fn mapping_struct<S: Yaml>(
        &mut self,
        id: NodeId,
        out: &mut S,
        ty: &str,
        fields: &[&str],
        assign: fn(&mut Self, &mut S, &str, NodeId) -> Res<()>,
    ) -> Res<bool> {
        let tree = self.tree;
        let n = tree.get(id);
        let merged_in = self.merged.take();
        let mut done: HashSet<String> = HashSet::new();
        let mut merge_node = None;
        for i in (0..n.content.len()).step_by(2) {
            let kid = n.content[i];
            if self.is_merge(kid) {
                merge_node = Some(n.content[i + 1]);
                continue;
            }
            let mut name = String::new();
            if !String::decode(self, kid, &mut name)? {
                continue;
            }
            if let Some(m) = &merged_in {
                if m.borrow().contains(&name) {
                    continue;
                }
                m.borrow_mut().insert(name.clone());
            }
            if fields.contains(&name.as_str()) {
                if !done.insert(name.clone()) {
                    self.terrors.push(format!(
                        "line {}: field {} already set in type {}",
                        tree.get(kid).line,
                        name,
                        ty
                    ));
                    continue;
                }
                assign(self, out, &name, n.content[i + 1])?;
            } else if self.known_fields {
                self.terrors.push(format!(
                    "line {}: field {} not found in type {}",
                    tree.get(kid).line,
                    name,
                    ty
                ));
            }
        }
        self.merged = merged_in;
        if let Some(m) = merge_node {
            self.merge(id, m, out)?;
        }
        Ok(true)
    }

    fn mapping_map<V: Yaml>(&mut self, id: NodeId, out: &mut BTreeMap<String, V>) -> Res<bool> {
        let tree = self.tree;
        let n = tree.get(id);
        let merged_in = self.merged.take();
        let map_is_new = out.is_empty();
        let mut merge_node = None;
        for i in (0..n.content.len()).step_by(2) {
            let kid = n.content[i];
            if self.is_merge(kid) {
                merge_node = Some(n.content[i + 1]);
                continue;
            }
            let mut key = String::new();
            if !String::decode(self, kid, &mut key)? {
                continue;
            }
            if let Some(m) = &merged_in {
                if m.borrow().contains(&key) {
                    continue;
                }
                m.borrow_mut().insert(key.clone());
            }
            let vid = n.content[i + 1];
            let mut value = V::default();
            let good = V::decode(self, vid, &mut value)?;
            if good || (tree.is_null(vid) && (map_is_new || !out.contains_key(&key))) {
                out.insert(key, value);
            }
        }
        self.merged = merged_in;
        if let Some(m) = merge_node {
            self.merge(id, m, out)?;
        }
        Ok(true)
    }

    fn merge<T: Yaml>(&mut self, parent: NodeId, merge: NodeId, out: &mut T) -> Res<()> {
        let tree = self.tree;
        let saved = self.merged.clone();
        if self.merged.is_none() {
            let set = Rc::new(RefCell::new(HashSet::new()));
            for &k in tree.get(parent).content.iter().step_by(2) {
                let kn = tree.get(tree.deref(k));
                if kn.kind == Kind::Scalar {
                    set.borrow_mut().insert(kn.value.clone());
                }
            }
            self.merged = Some(set);
        }
        let want_map = || failf("map merge requires map or sequence of maps as the value");
        let m = tree.get(merge);
        match m.kind {
            Kind::Mapping => {
                T::decode(self, merge, out)?;
            }
            Kind::Alias => {
                if m.alias.is_some_and(|t| tree.get(t).kind != Kind::Mapping) {
                    return want_map();
                }
                T::decode(self, merge, out)?;
            }
            Kind::Sequence => {
                for &item in &m.content {
                    let ni = tree.get(item);
                    if ni.kind == Kind::Alias {
                        if ni.alias.is_some_and(|t| tree.get(t).kind != Kind::Mapping) {
                            return want_map();
                        }
                    } else if ni.kind != Kind::Mapping {
                        return want_map();
                    }
                    T::decode(self, item, out)?;
                }
            }
            Kind::Scalar => return want_map(),
        }
        self.merged = saved;
        Ok(())
    }
}

/// A Go type the decoder can fill, with the hooks yaml.v3 dispatches on.
pub(crate) trait Yaml: Default {
    /// Go's `out.Type().String()`, used in error messages.
    fn go_type() -> String;

    /// `UnmarshalYAML` hook; invoked for non-null nodes only.
    fn unmarshaler(_d: &mut Decoder<'_>, _id: NodeId, _out: &mut Self) -> Option<Res<bool>> {
        None
    }

    /// `d.null`: reset the value; true for pointers, maps and slices.
    fn null(_out: &mut Self) -> bool {
        false
    }

    /// Stores a resolved scalar; false means a type error.
    fn set_scalar(_out: &mut Self, _resolved: &Resolved, _value: &str) -> bool {
        false
    }

    fn mapping(d: &mut Decoder<'_>, id: NodeId, _out: &mut Self) -> Res<bool> {
        d.terror(id, MAP_TAG, &Self::go_type());
        Ok(false)
    }

    fn sequence(d: &mut Decoder<'_>, id: NodeId, _out: &mut Self) -> Res<bool> {
        d.terror(id, SEQ_TAG, &Self::go_type());
        Ok(false)
    }

    fn decode(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.unmarshal_default(id, out)
    }
}

impl Yaml for String {
    fn go_type() -> String {
        "string".into()
    }
    fn set_scalar(out: &mut Self, _resolved: &Resolved, value: &str) -> bool {
        *out = value.to_string();
        true
    }
}

impl Yaml for i64 {
    fn go_type() -> String {
        "int".into()
    }
    fn set_scalar(out: &mut Self, resolved: &Resolved, _value: &str) -> bool {
        match resolved {
            Resolved::Int(v) => *out = *v,
            Resolved::Uint(v) if i64::try_from(*v).is_ok() => *out = *v as i64,
            Resolved::Float(f) if !f.is_nan() && *f <= i64::MAX as f64 => {
                *out = *f as i64;
            }
            _ => return false,
        }
        true
    }
}

impl Yaml for bool {
    fn go_type() -> String {
        "bool".into()
    }
    fn set_scalar(out: &mut Self, resolved: &Resolved, _value: &str) -> bool {
        match resolved {
            Resolved::Bool(b) => *out = *b,
            Resolved::Str(s) => match s.as_str() {
                "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" => *out = true,
                "n" | "N" | "no" | "No" | "NO" | "off" | "Off" | "OFF" => *out = false,
                _ => return false,
            },
            _ => return false,
        }
        true
    }
}

impl<E: Yaml> Yaml for Vec<E> {
    fn go_type() -> String {
        format!("[]{}", E::go_type())
    }
    fn null(out: &mut Self) -> bool {
        out.clear();
        true
    }
    fn sequence(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        let tree = d.tree;
        let mut items = Vec::new();
        for &c in &tree.get(id).content {
            let mut e = E::default();
            if E::decode(d, c, &mut e)? {
                items.push(e);
            }
        }
        *out = items;
        Ok(true)
    }
}

impl<V: Yaml> Yaml for BTreeMap<String, V> {
    fn go_type() -> String {
        format!("map[string]{}", V::go_type())
    }
    fn null(out: &mut Self) -> bool {
        out.clear();
        true
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_map(id, out)
    }
}

impl<T: Yaml> Yaml for Option<T> {
    fn go_type() -> String {
        T::go_type()
    }
    fn decode(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        if d.tree.is_null(id) {
            d.count()?;
            *out = None;
            return Ok(true);
        }
        let mut value = out.take().unwrap_or_default();
        let good = T::decode(d, id, &mut value);
        *out = Some(value);
        good
    }
}

/// A retained `yaml.Node` (used for the polymorphic `probe` field).
#[derive(Default)]
pub(crate) struct RawNode(pub Option<NodeId>);

impl Yaml for RawNode {
    fn go_type() -> String {
        "yaml.Node".into()
    }
    fn decode(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.count()?;
        out.0 = Some(id);
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Raw manifest types (Go: fileYAML and friends).

#[derive(Default)]
pub(crate) struct FileYaml {
    pub version: i64,
    pub name: String,
    pub dotenv: Vec<String>,
    pub default_env: String,
    pub setup: BTreeMap<String, StepYaml>,
    pub envs: BTreeMap<String, EnvYaml>,
    pub services: BTreeMap<String, ServiceYaml>,
    pub groups: BTreeMap<String, Vec<String>>,
    pub pipelines: BTreeMap<String, PipelineYaml>,
}

impl Yaml for FileYaml {
    fn go_type() -> String {
        "manifest.fileYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.fileYAML",
            &[
                "version",
                "name",
                "dotenv",
                "default_env",
                "setup",
                "envs",
                "services",
                "groups",
                "pipelines",
            ],
            |d, s, name, v| {
                match name {
                    "version" => i64::decode(d, v, &mut s.version)?,
                    "name" => String::decode(d, v, &mut s.name)?,
                    "dotenv" => Vec::decode(d, v, &mut s.dotenv)?,
                    "default_env" => String::decode(d, v, &mut s.default_env)?,
                    "setup" => BTreeMap::decode(d, v, &mut s.setup)?,
                    "envs" => BTreeMap::decode(d, v, &mut s.envs)?,
                    "services" => BTreeMap::decode(d, v, &mut s.services)?,
                    "groups" => BTreeMap::decode(d, v, &mut s.groups)?,
                    _ => BTreeMap::decode(d, v, &mut s.pipelines)?,
                };
                Ok(())
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct ServiceYaml {
    pub compose: String,
    pub task: String,
    pub run: String,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub dotenv: Vec<String>,
    pub profiles: Vec<String>,
    pub depends_on: Vec<String>,
    pub ports: BTreeMap<String, PortYaml>,
    pub health: Option<HealthYaml>,
}

impl Yaml for ServiceYaml {
    fn go_type() -> String {
        "manifest.serviceYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.serviceYAML",
            &[
                "compose",
                "task",
                "run",
                "cwd",
                "env",
                "dotenv",
                "profiles",
                "depends_on",
                "ports",
                "health",
            ],
            |d, s, name, v| {
                match name {
                    "compose" => String::decode(d, v, &mut s.compose)?,
                    "task" => String::decode(d, v, &mut s.task)?,
                    "run" => String::decode(d, v, &mut s.run)?,
                    "cwd" => String::decode(d, v, &mut s.cwd)?,
                    "env" => BTreeMap::decode(d, v, &mut s.env)?,
                    "dotenv" => Vec::decode(d, v, &mut s.dotenv)?,
                    "profiles" => Vec::decode(d, v, &mut s.profiles)?,
                    "depends_on" => Vec::decode(d, v, &mut s.depends_on)?,
                    "ports" => BTreeMap::decode(d, v, &mut s.ports)?,
                    _ => Option::decode(d, v, &mut s.health)?,
                };
                Ok(())
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct PortYaml {
    pub default: i64,
    pub env: PortEnvYaml,
    /// Retained node: explicit `null` differs from omission.
    pub probe: RawNode,
}

impl Yaml for PortYaml {
    fn go_type() -> String {
        "manifest.portYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.portYAML",
            &["default", "env", "probe"],
            |d, s, name, v| {
                match name {
                    "default" => i64::decode(d, v, &mut s.default)?,
                    "env" => PortEnvYaml::decode(d, v, &mut s.env)?,
                    _ => RawNode::decode(d, v, &mut s.probe)?,
                };
                Ok(())
            },
        )
    }
}

/// Legacy variable name or variable-to-template bindings.
#[derive(Default)]
pub(crate) struct PortEnvYaml {
    pub scalar: String,
    pub bindings: BTreeMap<String, PortEnvBinding>,
}

impl Yaml for PortEnvYaml {
    fn go_type() -> String {
        "manifest.portEnvYAML".into()
    }
    fn unmarshaler(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Option<Res<bool>> {
        Some(unmarshal_port_env(d, id, out))
    }
}

fn unmarshal_port_env(d: &mut Decoder<'_>, id: NodeId, out: &mut PortEnvYaml) -> Res<bool> {
    let tree = d.tree;
    let node = tree.get(id);
    if node.kind == Kind::Scalar && node.tag == STR_TAG {
        out.scalar = node.value.clone();
        return Ok(true);
    }
    if node.kind != Kind::Mapping {
        return fatal(format!(
            "line {}: port env must be a string or mapping",
            node.line
        ));
    }
    if node.content.is_empty() {
        return fatal(format!(
            "line {}: port env mapping must not be empty",
            node.line
        ));
    }
    let mut bindings: BTreeMap<String, PortEnvBinding> = BTreeMap::new();
    for pair in node.content.chunks(2) {
        let key = tree.get(pair[0]);
        if key.kind != Kind::Scalar || key.tag != STR_TAG {
            return fatal(format!(
                "line {}: port env variable name must be a string",
                key.line
            ));
        }
        if bindings.contains_key(&key.value) {
            return fatal(format!(
                "line {}: port env variable {} already defined",
                key.line,
                quote(&key.value)
            ));
        }
        let mut binding = PortEnvBinding::default();
        let mut value = tree.get(pair[1]);
        if value.kind == Kind::Mapping {
            if value.content.len() != 2 || tree.get(value.content[0]).value != "default" {
                let fields: Vec<&str> = value
                    .content
                    .iter()
                    .step_by(2)
                    .map(|&c| tree.get(c).value.as_str())
                    .collect();
                return fatal(format!(
                    "line {}: port env binding {} requires only the default field (got {})",
                    value.line,
                    quote(&key.value),
                    fields.join(", ")
                ));
            }
            binding.default = true;
            value = tree.get(value.content[1]);
        }
        if value.kind != Kind::Scalar || value.tag != STR_TAG {
            return fatal(format!(
                "line {}: port env binding {} template must be a string",
                value.line,
                quote(&key.value)
            ));
        }
        binding.template = value.value.clone();
        bindings.insert(key.value.clone(), binding);
    }
    out.bindings = bindings;
    Ok(true)
}

#[derive(Default)]
pub(crate) struct HealthYaml {
    pub http: String,
    pub tcp: bool,
    pub port: String,
    pub timeout: String,
}

impl Yaml for HealthYaml {
    fn go_type() -> String {
        "manifest.healthYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.healthYAML",
            &["http", "tcp", "port", "timeout"],
            |d, s, name, v| {
                match name {
                    "http" => String::decode(d, v, &mut s.http)?,
                    "tcp" => bool::decode(d, v, &mut s.tcp)?,
                    "port" => String::decode(d, v, &mut s.port)?,
                    _ => String::decode(d, v, &mut s.timeout)?,
                };
                Ok(())
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct EnvYaml {
    pub compose: Vec<String>,
    pub profiles: Vec<String>,
    pub deploy: Option<DeployYaml>,
}

impl Yaml for EnvYaml {
    fn go_type() -> String {
        "manifest.envYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.envYAML",
            &["compose", "profiles", "deploy"],
            |d, s, name, v| {
                match name {
                    "compose" => Vec::decode(d, v, &mut s.compose)?,
                    "profiles" => Vec::decode(d, v, &mut s.profiles)?,
                    _ => Option::decode(d, v, &mut s.deploy)?,
                };
                Ok(())
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct DeployYaml {
    pub task: String,
    pub run: String,
    pub confirm: bool,
}

impl Yaml for DeployYaml {
    fn go_type() -> String {
        "manifest.deployYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.deployYAML",
            &["task", "run", "confirm"],
            |d, s, name, v| {
                match name {
                    "task" => String::decode(d, v, &mut s.task)?,
                    "run" => String::decode(d, v, &mut s.run)?,
                    _ => bool::decode(d, v, &mut s.confirm)?,
                };
                Ok(())
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct StepYaml {
    pub task: String,
    pub run: String,
}

impl Yaml for StepYaml {
    fn go_type() -> String {
        "manifest.stepYAML".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.stepYAML",
            &["task", "run"],
            |d, s, name, v| {
                match name {
                    "task" => String::decode(d, v, &mut s.task)?,
                    _ => String::decode(d, v, &mut s.run)?,
                };
                Ok(())
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct PipelineYaml {
    pub needs: Vec<String>,
    pub steps: Vec<StepYaml>,
}

impl Yaml for PipelineYaml {
    fn go_type() -> String {
        "manifest.pipelineYAML".into()
    }
    fn unmarshaler(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Option<Res<bool>> {
        Some(unmarshal_pipeline(d, id, out))
    }
}

/// Go's `type plain pipelineYAML`: the same fields without the hook.
#[derive(Default)]
struct PlainPipeline(PipelineYaml);

impl Yaml for PlainPipeline {
    fn go_type() -> String {
        "manifest.plain".into()
    }
    fn mapping(d: &mut Decoder<'_>, id: NodeId, out: &mut Self) -> Res<bool> {
        d.mapping_struct(
            id,
            out,
            "manifest.plain",
            &["needs", "steps"],
            |d, s, name, v| {
                match name {
                    "needs" => Vec::decode(d, v, &mut s.0.needs)?,
                    _ => Vec::decode(d, v, &mut s.0.steps)?,
                };
                Ok(())
            },
        )
    }
}

fn unmarshal_pipeline(d: &mut Decoder<'_>, id: NodeId, out: &mut PipelineYaml) -> Res<bool> {
    let tree = d.tree;
    let node = tree.get(id);
    let mut steps: Option<NodeId> = None;
    match node.kind {
        Kind::Sequence => steps = Some(id),
        Kind::Mapping => {
            for pair in node.content.chunks(2) {
                let key = tree.get(pair[0]);
                match key.value.as_str() {
                    "needs" => {
                        let needs = tree.get(tree.deref(pair[1]));
                        if needs.kind != Kind::Sequence {
                            return fatal(format!(
                                "line {}: pipeline needs must be an array",
                                needs.line
                            ));
                        }
                    }
                    "steps" => steps = Some(pair[1]),
                    other => {
                        return fatal(format!(
                            "line {}: unknown pipeline field {}",
                            key.line,
                            quote(other)
                        ));
                    }
                }
            }
            if steps.is_none() {
                return fatal(format!(
                    "line {}: pipeline object requires steps",
                    node.line
                ));
            }
        }
        _ => {
            return fatal(format!(
                "line {}: pipeline must be an array or object",
                node.line
            ));
        }
    }
    let steps = tree.get(tree.deref(steps.unwrap_or(id)));
    if steps.kind != Kind::Sequence {
        return fatal(format!(
            "line {}: pipeline steps must be an array",
            steps.line
        ));
    }
    // Node.Decode does not inherit KnownFields, so each step is checked here.
    for &step in &steps.content {
        let step = tree.get(tree.deref(step));
        if step.kind == Kind::Mapping {
            for &k in step.content.iter().step_by(2) {
                let key = tree.get(k);
                if key.value != "run" && key.value != "task" {
                    return fatal(format!(
                        "line {}: unknown pipeline step field {}",
                        key.line,
                        quote(&key.value)
                    ));
                }
            }
        }
    }
    let mut inner = Decoder::new(tree, false);
    if node.kind == Kind::Sequence {
        Vec::decode(&mut inner, id, &mut out.steps)?;
    } else {
        let mut plain = PlainPipeline(PipelineYaml {
            needs: std::mem::take(&mut out.needs),
            steps: std::mem::take(&mut out.steps),
        });
        PlainPipeline::decode(&mut inner, id, &mut plain)?;
        *out = plain.0;
    }
    if inner.terrors.is_empty() {
        Ok(true)
    } else {
        d.terrors.append(&mut inner.terrors);
        Ok(false)
    }
}

/// The decoded manifest and the tree its retained nodes point into.
pub(crate) struct Decoded {
    pub tree: Tree,
    pub file: FileYaml,
}

/// Mirrors `yaml.NewDecoder(...).KnownFields(true).Decode(&fileYAML{})`.
/// The error string is final (everything after `parse rocket.yaml: `).
pub(crate) fn decode_file(data: &[u8]) -> Result<Decoded, String> {
    let src =
        std::str::from_utf8(data).map_err(|_| "yaml: invalid leading UTF-8 octet".to_string())?;
    let Some((tree, root)) = parse_tree(src)? else {
        return Err("EOF".to_string());
    };
    let mut file = FileYaml::default();
    let terrors = {
        let mut d = Decoder::new(&tree, true);
        FileYaml::decode(&mut d, root, &mut file).map_err(|Fatal(m)| m)?;
        d.terrors
    };
    if !terrors.is_empty() {
        return Err(format!(
            "yaml: unmarshal errors:\n  {}",
            terrors.join("\n  ")
        ));
    }
    Ok(Decoded { tree, file })
}
