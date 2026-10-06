//! The JSON Schema (draft 2020-12) for `rocket.yaml`, byte-identical to Go's
//! `json.MarshalIndent(schema, "", "  ")` output: object keys sorted, two
//! space indent, `<`, `>` and `&` escaped, no trailing newline.

use serde_json::{Value, json};

/// The schema as a JSON value.
pub fn schema_value() -> Value {
    let string = json!({"type": "string"});
    let string_list = json!({"type": "array", "items": string});
    let string_map = json!({
        "type": "object",
        "description": "service environment; {service.port} tokens resolve live provider ports and add startup dependencies",
        "additionalProperties": {"type": ["string", "number", "boolean"]},
    });
    let step = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "task": {"type": "string", "description": "go-task task name"},
            "run": {"type": "string", "description": "shell command"},
        },
        "oneOf": [{"required": ["task"]}, {"required": ["run"]}],
    });
    let pipeline_steps = json!({"type": "array", "items": {"$ref": "#/$defs/step"}});
    let pipeline = json!({"oneOf": [
        pipeline_steps,
        {
            "type": "object", "additionalProperties": false, "required": ["steps"],
            "properties": {"needs": string_list, "steps": pipeline_steps},
        },
    ]});
    let port_template = json!({
        "type": "string",
        "pattern": r"\{port\}",
        "description": "template containing {port}, replaced by the resolved port",
    });
    let env_var_pattern = "^[A-Za-z_][A-Za-z0-9_]*$";
    let port_env = json!({
        "description": "variable name or variable-to-template bindings; an unconditional binding permits remapping",
        "oneOf": [
            {"type": "string", "pattern": env_var_pattern},
            {
                "type": "object", "minProperties": 1,
                "propertyNames": {"pattern": env_var_pattern},
                "additionalProperties": {"oneOf": [
                    port_template,
                    {
                        "type": "object", "additionalProperties": false,
                        "required": ["default"], "properties": {"default": port_template},
                        "description": "inject only when the inherited/dotenv/service value is empty",
                    },
                ]},
            },
        ],
    });
    let port = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["default"],
        "properties": {
            "default": {"type": "integer", "minimum": 1, "maximum": 65535},
            "env": port_env,
            "probe": {"type": "boolean", "default": true, "description": "use this port for Rocket readiness; false still leases and remaps it"},
        },
    });
    let health = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "http": {"type": "string", "pattern": "^/", "description": "HTTP path probed on the health port"},
            "tcp": {"type": "boolean"},
            "port": {"type": "string", "description": "port name with probe enabled; defaults to the first eligible port"},
            "timeout": {"type": "string", "description": "Go duration, e.g. 60s"},
        },
    });
    let service = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "compose": {"type": "string", "description": "service name in the env's compose files"},
            "task": {"type": "string", "description": "long-running go-task task"},
            "run": {"type": "string", "description": "raw shell command"},
            "cwd": {"type": "string", "description": "working dir relative to project root"},
            "env": string_map,
            "dotenv": string_list,
            "profiles": {"type": "array", "items": string, "description": "gates startup wildcard selection; explicit members and required dependencies remain selectable"},
            "depends_on": string_list,
            "ports": {"type": "object", "additionalProperties": {"$ref": "#/$defs/port"}},
            "health": {"$ref": "#/$defs/health"},
        },
        "oneOf": [{"required": ["compose"]}, {"required": ["task"]}, {"required": ["run"]}],
    });
    let env = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "compose": string_list,
            "profiles": {"type": "array", "items": string, "description": "enabled startup profiles and Compose profiles for this environment"},
            "deploy": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "task": {"type": "string", "description": "go-task task that deploys"},
                    "run": {"type": "string", "description": "shell command that deploys"},
                    "confirm": {"type": "boolean", "description": "require --yes"},
                },
                "oneOf": [{"required": ["task"]}, {"required": ["run"]}],
            },
        },
    });
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/xean-io/rocket/rocket.schema.json",
        "title": "rocket.yaml",
        "type": "object",
        "additionalProperties": false,
        "required": ["version", "services"],
        "properties": {
            "version": {"const": 1},
            "name": {"type": "string", "pattern": "^[a-z0-9][a-z0-9._-]*$"},
            "dotenv": string_list,
            "default_env": string,
            "setup": {"type": "object", "additionalProperties": {"$ref": "#/$defs/step"}},
            "envs": {"type": "object", "additionalProperties": {"$ref": "#/$defs/env"}},
            "services": {"type": "object", "minProperties": 1, "additionalProperties": {"$ref": "#/$defs/service"}},
            "groups": {"type": "object", "additionalProperties": string_list},
            "pipelines": {"type": "object", "additionalProperties": pipeline},
        },
        "$defs": {
            "step": step,
            "port": port,
            "health": health,
            "service": service,
            "env": env,
        },
    })
}

/// The schema document exactly as Go's `manifest.Schema()` returns it
/// (without a trailing newline).
pub fn schema() -> String {
    let mut out = String::new();
    write_value(&schema_value(), 0, &mut out);
    out
}

fn write_string(s: &str, out: &mut String) {
    // serde_json escapes like Go except for the HTML-sensitive characters and
    // U+2028/U+2029 that Go's encoder escapes by default.
    let encoded = serde_json::to_string(s).unwrap_or_default();
    for c in encoded.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
}

fn indent(depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_value(value: &Value, depth: usize, out: &mut String) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(depth + 1, out);
                write_value(item, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(depth, out);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            // Sorted explicitly: serde_json may preserve insertion order when
            // another crate in the workspace enables `preserve_order`.
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push_str("{\n");
            for (i, (key, item)) in entries.iter().enumerate() {
                indent(depth + 1, out);
                write_string(key, out);
                out.push_str(": ");
                write_value(item, depth + 1, out);
                out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
            }
            indent(depth, out);
            out.push('}');
        }
        Value::String(s) => write_string(s, out),
        other => out.push_str(&other.to_string()),
    }
}
