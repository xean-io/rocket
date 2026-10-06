//! Compose file discovery and port parsing (Go: `detectCompose`,
//! `parsePortNode`, `parseShortPort`, `parsePublished`).

use crate::{Detected, Env, Error, Port, Service};
use rocket_manifest::yaml::{self, Node, NodeKind};
use std::collections::BTreeMap;
use std::path::Path;

const DEV_COMPOSE_ORDER: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// `^(docker-)?compose(\.([A-Za-z0-9_-]+))?\.ya?ml$`: the suffix (group 3)
/// when `name` is a compose file, empty for the plain names.
fn compose_suffix(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("docker-").unwrap_or(name);
    let rest = rest.strip_prefix("compose")?;
    for ext in [".yaml", ".yml"] {
        if rest == ext {
            return Some("");
        }
    }
    for ext in [".yaml", ".yml"] {
        if let Some(mid) = rest.strip_suffix(ext).and_then(|m| m.strip_prefix('.'))
            && !mid.is_empty()
            && mid
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Some(mid);
        }
    }
    None
}

fn compose_rank(f: &str) -> usize {
    DEV_COMPOSE_ORDER
        .iter()
        .position(|n| *n == f)
        .unwrap_or(DEV_COMPOSE_ORDER.len())
}

#[derive(Default)]
struct ComposeService {
    profiles: Vec<String>,
    ports: Vec<Port>,
}

#[derive(Default)]
struct ComposeFile {
    name: String,
    services: BTreeMap<String, ComposeService>,
}

/// yaml.v3's `cannot unmarshal` wording for a node of the wrong shape.
fn type_error(n: Node<'_>, into: &str) -> String {
    let what = if n.kind() == NodeKind::Scalar {
        format!("{} `{}`", n.tag(), n.value())
    } else {
        n.tag().to_owned()
    };
    format!(
        "yaml: unmarshal errors:\n  line {}: cannot unmarshal {what} into {into}",
        n.line()
    )
}

fn decode_compose(data: &str) -> Result<ComposeFile, String> {
    let mut cf = ComposeFile::default();
    let Some(doc) = yaml::parse(data)? else {
        return Ok(cf);
    };
    let root = doc.root();
    if root.is_null() {
        return Ok(cf);
    }
    if root.kind() != NodeKind::Mapping {
        return Err(type_error(root, "scaffold.composeFile"));
    }
    if let Some(name) = root.get("name")
        && !name.is_null()
    {
        if name.kind() != NodeKind::Scalar {
            return Err(type_error(name, "string"));
        }
        cf.name = name.value().to_owned();
    }
    let Some(services) = root.get("services") else {
        return Ok(cf);
    };
    if services.is_null() {
        return Ok(cf);
    }
    if services.kind() != NodeKind::Mapping {
        return Err(type_error(services, "map[string]struct"));
    }
    for (key, body) in services.entries() {
        let mut svc = ComposeService::default();
        if !body.is_null() {
            if body.kind() != NodeKind::Mapping {
                return Err(type_error(body, "struct"));
            }
            if let Some(profiles) = body.get("profiles")
                && !profiles.is_null()
            {
                if profiles.kind() != NodeKind::Sequence {
                    return Err(type_error(profiles, "[]string"));
                }
                for p in profiles.items() {
                    if p.kind() != NodeKind::Scalar {
                        return Err(type_error(p, "string"));
                    }
                    svc.profiles.push(p.value().to_owned());
                }
            }
            if let Some(ports) = body.get("ports")
                && !ports.is_null()
            {
                if ports.kind() != NodeKind::Sequence {
                    return Err(type_error(ports, "[]yaml.Node"));
                }
                svc.ports = ports
                    .items()
                    .into_iter()
                    .filter_map(parse_port_node)
                    .collect();
            }
        }
        cf.services.insert(key.value().to_owned(), svc);
    }
    Ok(cf)
}

pub(crate) fn detect(r: &mut Detected, root: &Path) -> Result<(), Error> {
    let entries = std::fs::read_dir(root)
        .map_err(|e| Error::Io(format!("open {}: {}", root.display(), reason(&e))))?;
    let mut files: Vec<String> = Vec::new();
    for e in entries {
        let e =
            e.map_err(|e| Error::Io(format!("readdirent {}: {}", root.display(), reason(&e))))?;
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_dir && compose_suffix(&name).is_some() {
            files.push(name);
        }
    }
    if files.is_empty() {
        return Ok(());
    }
    files.sort_by(|a, b| compose_rank(a).cmp(&compose_rank(b)).then_with(|| a.cmp(b)));
    r.compose_files = files.clone();

    let mut parsed: BTreeMap<&str, ComposeFile> = BTreeMap::new();
    for f in &files {
        let data = std::fs::read(root.join(f))
            .map_err(|e| Error::Io(format!("open {}: {}", root.join(f).display(), reason(&e))))?;
        let cf = decode_compose(&String::from_utf8_lossy(&data))
            .map_err(|e| Error::Yaml(format!("{f}: {e}")))?;
        parsed.insert(f, cf);
    }

    // The first un-suffixed file is dev (plus override files); every other
    // file becomes its own env.
    let mut dev = Env {
        name: "dev".into(),
        files: Vec::new(),
    };
    let mut others: Vec<Env> = Vec::new();
    let mut used: Vec<String> = vec!["dev".into()];
    let mut smoke = 0;
    for f in &files {
        let suffix = compose_suffix(f).unwrap_or_default();
        if (suffix.is_empty() && dev.files.is_empty()) || suffix == "override" {
            dev.files.push(f.clone());
        } else {
            let mut name = suffix.to_lowercase();
            if name.is_empty() {
                smoke += 1;
                name = "smoke".into();
                if smoke > 1 {
                    name = format!("smoke{smoke}");
                }
            }
            while used.contains(&name) {
                name.push_str("-alt");
            }
            used.push(name.clone());
            others.push(Env {
                name,
                files: vec![f.clone()],
            });
        }
    }
    if dev.files.is_empty() {
        // Only suffixed files: promote the first.
        dev.files = others.remove(0).files;
    }
    r.envs = std::iter::once(dev.clone())
        .chain(others.iter().cloned())
        .collect();

    let mut dev_services: Vec<String> = Vec::new();
    for f in &dev.files {
        for (name, s) in &parsed[f.as_str()].services {
            if dev_services.contains(name) {
                continue;
            }
            dev_services.push(name.clone());
            let mut svc = Service {
                name: name.clone(),
                compose: name.clone(),
                profiles: s.profiles.clone(),
                ports: s.ports.clone(),
                ..Service::default()
            };
            name_svc_ports(&mut svc.ports);
            r.services.push(svc);
        }
    }
    for e in &others {
        let only: Vec<&str> = parsed[e.files[0].as_str()]
            .services
            .keys()
            .filter(|n| !dev_services.contains(n))
            .map(String::as_str)
            .collect();
        if !only.is_empty() {
            r.notes.push(format!(
                "services only in {} (env {}, not added): {}",
                e.files[0],
                e.name,
                only.join(", ")
            ));
        }
    }
    let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in &files {
        let n = parsed[f.as_str()].name.as_str();
        if !n.is_empty() {
            by_name.entry(n).or_default().push(f);
        }
    }
    for (n, fs) in by_name {
        if fs.len() > 1 {
            r.notes.push(format!(
                "{} share the compose project name {}; rocket forces rocket-{}-<env> per env, so their volumes no longer collide",
                fs.join(" and "),
                crate::render::go_quote(n),
                r.name
            ));
        }
    }
    Ok(())
}

/// `os` error text without Rust's ` (os error N)` suffix, lowercased first
/// letter, like Go's.
fn reason(err: &std::io::Error) -> String {
    let text = err.to_string();
    let text = text
        .rfind(" (os error")
        .map_or(text.as_str(), |i| &text[..i]);
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn name_svc_ports(ports: &mut [Port]) {
    let single = ports.len() == 1;
    for p in ports {
        p.name = if single {
            "main".into()
        } else {
            format!("p{}", p.target)
        };
    }
}

fn parse_port_node(n: Node<'_>) -> Option<Port> {
    match n.kind() {
        NodeKind::Scalar => parse_short_port(n.value()),
        NodeKind::Mapping => {
            let published = n
                .get("published")
                .filter(|p| p.kind() == NodeKind::Scalar && !p.is_null())?;
            if published.value().is_empty() {
                return None;
            }
            let target = match n.get("target") {
                None => 0,
                Some(t) if t.is_null() => 0,
                Some(t) if t.kind() == NodeKind::Scalar && t.tag() == "!!int" => atoi(t.value())?,
                // yaml.v3 refuses a non-integer `target`, which skips the port.
                Some(_) => return None,
            };
            parse_published(published.value(), target)
        }
        NodeKind::Sequence => None,
    }
}

/// Go's `strconv.Atoi`: optional sign, decimal digits only.
fn atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Parses `[ip:]host:container[/proto]`, where host may be `${VAR:-default}`.
/// Container-only, ranges and ephemeral host ports yield `None`.
pub fn parse_short_port(s: &str) -> Option<Port> {
    let mut s = s.trim();
    if let Some(i) = s.rfind('/')
        && !s[i..].contains('}')
    {
        s = &s[..i];
    }
    let parts = split_outside_braces(s);
    if parts.len() < 2 {
        return None;
    }
    let target = atoi(parts[parts.len() - 1])?;
    parse_published(parts[parts.len() - 2], target)
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn port(default: i64, env: &str, target: i64) -> Port {
    Port {
        name: String::new(),
        default,
        env: env.to_owned(),
        target,
    }
}

fn parse_published(host: &str, target: i64) -> Option<Port> {
    let host = host.trim();
    // ^\$\{([A-Za-z_][A-Za-z0-9_]*)(:?-([^}]*))?\}$
    if let Some(inner) = host.strip_prefix("${").and_then(|h| h.strip_suffix('}'))
        && !inner.contains('}')
    {
        let (name, default_text) = match inner.find(['-', ':']) {
            Some(i) => {
                let rest = &inner[i..];
                let after = rest.strip_prefix(":-").or_else(|| rest.strip_prefix('-'));
                match after {
                    Some(d) => (&inner[..i], Some(d)),
                    None => (inner, None),
                }
            }
            None => (inner, None),
        };
        if is_ident(name) {
            let mut def = target;
            if let Some(d) = default_text
                && !d.is_empty()
            {
                def = atoi(d)?;
            }
            return (def > 0).then(|| port(def, name, target));
        }
    }
    // ^\$([A-Za-z_][A-Za-z0-9_]*)$
    if let Some(name) = host.strip_prefix('$')
        && is_ident(name)
    {
        return (target > 0).then(|| port(target, name, target));
    }
    let v = atoi(host)?;
    (v > 0).then(|| port(v, "", target))
}

fn split_outside_braces(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ':' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_file_names() {
        for (name, want) in [
            ("compose.yaml", Some("")),
            ("compose.yml", Some("")),
            ("docker-compose.yaml", Some("")),
            ("docker-compose.prod.yml", Some("prod")),
            ("compose.override.yaml", Some("override")),
            ("compose.a_b-C.yml", Some("a_b-C")),
            ("compose.yaml.yml", Some("yaml")),
            ("compose.a.b.yml", None),
            ("compose..yml", None),
            ("compose.txt", None),
            ("my-compose.yml", None),
            ("docker-docker-compose.yml", None),
        ] {
            assert_eq!(compose_suffix(name), want, "{name}");
        }
    }

    #[test]
    fn short_port_variable_forms() {
        // `${VAR-default}` (no colon) and an unterminated expansion.
        assert_eq!(parse_short_port("${A-1}:2").unwrap().default, 1);
        assert!(parse_short_port("${A:2").is_none());
        assert_eq!(parse_short_port("${A:-9:9}:2").map(|p| p.default), None);
    }
}
