//! Raw YAML types to domain types, and semantic validation (Go: `convert`
//! and `validate`). Go iterates maps in random order; this port iterates in
//! sorted order so the aggregated problem list is deterministic.

use crate::decode::{Decoded, EnvYaml, PortYaml, ServiceYaml};
use crate::gostd::{base, parse_duration, quote};
use crate::node::{BOOL_TAG, Kind, Resolved, resolve};
use rocket_domain::{
    ALL_SERVICES, Deploy, Environment, HealthSpec, PortSpec, Project, Service, ServiceKind, Step,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// `^[A-Za-z_][A-Za-z0-9_]*$`
pub(crate) fn is_env_var_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `^[a-z0-9][a-z0-9._-]*$`
fn is_project_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

pub(crate) fn convert(decoded: &Decoded, root: &str) -> (Project, Vec<String>) {
    let f = &decoded.file;
    let mut problems = Vec::new();
    if f.version != 1 {
        problems.push(format!("version must be 1 (got {})", f.version));
    }
    let mut p = Project {
        name: f.name.clone(),
        root: root.to_string(),
        dotenv: f.dotenv.clone(),
        default_env: f.default_env.clone(),
        groups: f.groups.clone(),
        ..Project::default()
    };
    if p.name.is_empty() {
        p.name = base(root).to_lowercase();
    }
    for (name, s) in &f.setup {
        p.setup.insert(
            name.clone(),
            Step {
                task: s.task.clone(),
                run: s.run.clone(),
            },
        );
    }
    for (name, pipeline) in &f.pipelines {
        p.pipeline_needs
            .insert(name.clone(), pipeline.needs.clone());
        p.pipelines.insert(
            name.clone(),
            pipeline
                .steps
                .iter()
                .map(|s| Step {
                    task: s.task.clone(),
                    run: s.run.clone(),
                })
                .collect(),
        );
    }
    for (name, e) in &f.envs {
        convert_env(&mut p, &mut problems, name, e);
    }
    if p.envs.is_empty() {
        p.envs.insert(
            "dev".into(),
            Environment {
                name: "dev".into(),
                ..Environment::default()
            },
        );
    }
    if p.default_env.is_empty() {
        p.default_env = "dev".into();
        if !p.envs.contains_key("dev")
            && let Some(first) = p.envs.keys().next()
        {
            p.default_env = first.clone();
        }
    }
    for (name, s) in &f.services {
        let svc = convert_service(decoded, &mut problems, name, s);
        p.services.insert(name.clone(), svc);
    }
    for svc in p.services.values_mut() {
        let mut seen: BTreeSet<String> = svc.depends_on.iter().cloned().collect();
        for reference in svc.port_references() {
            if seen.insert(reference.service.clone()) {
                svc.depends_on.push(reference.service);
            }
        }
    }
    (p, problems)
}

fn convert_env(p: &mut Project, problems: &mut Vec<String>, name: &str, e: &EnvYaml) {
    let mut env = Environment {
        name: name.to_string(),
        compose: e.compose.clone(),
        profiles: e.profiles.clone(),
        deploy: None,
    };
    if let Some(d) = &e.deploy {
        env.deploy = Some(Deploy {
            task: d.task.clone(),
            run: d.run.clone(),
            confirm: d.confirm,
        });
        if d.task.is_empty() == d.run.is_empty() {
            problems.push(format!(
                "env {}: deploy needs exactly one of task or run",
                quote(name)
            ));
        }
    }
    p.envs.insert(name.to_string(), env);
}

fn convert_service(
    decoded: &Decoded,
    problems: &mut Vec<String>,
    name: &str,
    s: &ServiceYaml,
) -> Service {
    let mut svc = Service {
        name: name.to_string(),
        compose: s.compose.clone(),
        task: s.task.clone(),
        run: s.run.clone(),
        cwd: s.cwd.clone(),
        env: s.env.clone(),
        dotenv: s.dotenv.clone(),
        profiles: s.profiles.clone(),
        depends_on: s.depends_on.clone(),
        ..Service::default()
    };
    let mut kinds: Vec<&str> = Vec::new();
    if !s.compose.is_empty() {
        kinds.push("compose");
        svc.kind = ServiceKind::Compose;
    }
    if !s.task.is_empty() {
        kinds.push("task");
        svc.kind = ServiceKind::Task;
    }
    if !s.run.is_empty() {
        kinds.push("run");
        svc.kind = ServiceKind::Run;
    }
    if kinds.len() != 1 {
        let got = if kinds.is_empty() {
            "none".to_string()
        } else {
            kinds.join(", ")
        };
        problems.push(format!(
            "service {}: exactly one of compose, task or run is required (got {got})",
            quote(name)
        ));
    }
    // BTreeMap iteration is name-sorted, matching Go's final sort.
    for (pname, port) in &s.ports {
        svc.ports
            .push(convert_port(decoded, problems, name, pname, port));
    }
    if let Some(h) = &s.health {
        let mut spec = HealthSpec {
            http: h.http.clone(),
            tcp: h.tcp,
            port: h.port.clone(),
            timeout: Duration::ZERO,
        };
        if !h.timeout.is_empty() {
            match parse_duration(&h.timeout) {
                Some(ns) if ns > 0 => {
                    spec.timeout = Duration::from_nanos(u64::try_from(ns).unwrap_or(u64::MAX));
                }
                _ => problems.push(format!(
                    "service {}: invalid health.timeout {}",
                    quote(name),
                    quote(&h.timeout)
                )),
            }
        }
        svc.health = Some(spec);
    }
    svc
}

fn convert_port(
    decoded: &Decoded,
    problems: &mut Vec<String>,
    service: &str,
    pname: &str,
    port: &PortYaml,
) -> PortSpec {
    let tree = &decoded.tree;
    let mut spec = PortSpec {
        name: pname.to_string(),
        // Out-of-range values are reported by `validate`; the project is not
        // returned in that case.
        default: u16::try_from(port.default).unwrap_or(0),
        env: port.env.scalar.clone(),
        env_bindings: port.env.bindings.clone(),
        probe: None,
    };
    if let Some(id) = port.probe.0 {
        let n = tree.get(tree.deref(id));
        let not_bool = || {
            format!(
                "service {}: port {}: probe must be a boolean",
                quote(service),
                quote(pname)
            )
        };
        if n.kind != Kind::Scalar || n.tag != BOOL_TAG {
            problems.push(not_bool());
        } else {
            match resolve(&n.tag, &n.value) {
                Ok((_, Resolved::Bool(b))) => spec.probe = Some(b),
                _ => problems.push(not_bool()),
            }
        }
    }
    // Keep remaps.env and conflicts.env scalar and deterministic. A default
    // binding cannot authorize remapping because its override may be retained.
    for (variable, binding) in &port.env.bindings {
        if !binding.default && (spec.env.is_empty() || variable.as_str() < spec.env.as_str()) {
            spec.env = variable.clone();
        }
    }
    spec
}

pub(crate) fn validate(p: &Project, decoded: &Decoded) -> Vec<String> {
    let mut problems = Vec::new();
    let mut add = |msg: String| problems.push(msg);

    if !is_project_name(&p.name) {
        add(format!(
            "invalid project name {} (use lowercase letters, digits, '.', '_' or '-')",
            quote(&p.name)
        ));
    }
    if p.services.is_empty() {
        add("no services declared".to_string());
    }
    if !p.envs.contains_key(&p.default_env) {
        add(format!(
            "default_env {} is not a declared env",
            quote(&p.default_env)
        ));
    }
    for name in p.service_names() {
        let s = &p.services[&name];
        for reference in s.port_references() {
            let Some(provider) = p.services.get(&reference.service) else {
                add(format!(
                    "service {}: env reference {}: unknown service {}",
                    quote(&name),
                    reference.token,
                    quote(&reference.service)
                ));
                continue;
            };
            if !provider
                .ports
                .iter()
                .any(|port| port.name == reference.port)
            {
                add(format!(
                    "service {}: env reference {}: unknown port {} in service {}",
                    quote(&name),
                    reference.token,
                    quote(&reference.port),
                    quote(&reference.service)
                ));
            }
        }
        for d in &s.depends_on {
            if *d == name {
                add(format!("service {} depends on itself", quote(&name)));
            } else if !p.services.contains_key(d) {
                add(format!(
                    "service {}: depends_on references unknown service {}",
                    quote(&name),
                    quote(d)
                ));
            }
        }
        let mut port_specs: BTreeMap<&str, &PortSpec> = BTreeMap::new();
        for port in &s.ports {
            port_specs.insert(port.name.as_str(), port);
            let raw_default = decoded
                .file
                .services
                .get(&name)
                .and_then(|svc| svc.ports.get(&port.name))
                .map_or(i64::from(port.default), |raw| raw.default);
            validate_port(&mut add, &name, port, raw_default);
        }
        if let Some(h) = &s.health {
            if s.ports.is_empty() {
                add(format!(
                    "service {}: health requires at least one port",
                    quote(&name)
                ));
            }
            if !h.port.is_empty() {
                match port_specs.get(h.port.as_str()) {
                    None => add(format!(
                        "service {}: health.port {} is not a declared port",
                        quote(&name),
                        quote(&h.port)
                    )),
                    Some(spec) if !spec.probe_enabled() => add(format!(
                        "service {}: health.port {} has probe disabled",
                        quote(&name),
                        quote(&h.port)
                    )),
                    Some(_) => {}
                }
            }
            if !h.http.is_empty() && !h.http.starts_with('/') {
                add(format!(
                    "service {}: health.http {} must start with /",
                    quote(&name),
                    quote(&h.http)
                ));
            }
        }
    }
    for (g, members) in &p.groups {
        if p.services.contains_key(g) {
            add(format!("group {}: name collides with a service", quote(g)));
        }
        for m in members {
            if m == ALL_SERVICES {
                continue;
            }
            if !p.services.contains_key(m) {
                add(format!("group {}: unknown service {}", quote(g), quote(m)));
            }
        }
    }
    for (n, steps) in &p.pipelines {
        let needs = p.pipeline_needs.get(n).map_or(&[][..], Vec::as_slice);
        if let Err(err) = p.expand_targets(needs) {
            add(format!("pipeline {} needs: {}", quote(n), go_error(&err)));
        }
        for (i, s) in steps.iter().enumerate() {
            if s.task.is_empty() == s.run.is_empty() {
                add(format!(
                    "pipeline {} step {}: exactly one of task or run is required",
                    quote(n),
                    i + 1
                ));
            }
        }
    }
    for (n, s) in &p.setup {
        if s.task.is_empty() == s.run.is_empty() {
            add(format!(
                "setup {}: exactly one of task or run is required",
                quote(n)
            ));
        }
    }
    if let Err(err) = p.start_order(&p.service_names()) {
        add(go_error(&err));
    }
    problems
}

/// The Go text of a domain graph error (`%q` quoting for target names).
fn go_error(err: &rocket_domain::Error) -> String {
    match err {
        rocket_domain::Error::UnknownTarget { target, project } => {
            format!(
                "unknown service or group {} in project {project}",
                quote(target)
            )
        }
        other => other.to_string(),
    }
}

fn validate_port(add: &mut impl FnMut(String), service: &str, port: &PortSpec, raw_default: i64) {
    let prefix = format!("service {}: port {}", quote(service), quote(&port.name));
    if !(1..=65535).contains(&raw_default) {
        add(format!("{prefix}: port {raw_default} out of range"));
    }
    if !port.env.is_empty() && !is_env_var_name(&port.env) {
        add(format!(
            "{prefix}: invalid env var name {}",
            quote(&port.env)
        ));
    }
    for (variable, binding) in &port.env_bindings {
        if !is_env_var_name(variable) {
            add(format!(
                "{prefix}: invalid env var name {}",
                quote(variable)
            ));
        }
        if !binding.template.contains("{port}") {
            add(format!(
                "{prefix}: env binding {} template must contain {{port}}",
                quote(variable)
            ));
        }
    }
}
