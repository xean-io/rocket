package manifest

import "encoding/json"

// Schema returns the JSON Schema (draft 2020-12) for rocket.yaml.
func Schema() []byte {
	str := map[string]any{"type": "string"}
	strList := map[string]any{"type": "array", "items": str}
	strMap := map[string]any{"type": "object", "description": "service environment; {service.port} tokens resolve live provider ports and add startup dependencies", "additionalProperties": map[string]any{"type": []string{"string", "number", "boolean"}}}
	step := map[string]any{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]any{
			"task": map[string]any{"type": "string", "description": "go-task task name"},
			"run":  map[string]any{"type": "string", "description": "shell command"},
		},
		"oneOf": []any{
			map[string]any{"required": []string{"task"}},
			map[string]any{"required": []string{"run"}},
		},
	}
	pipelineSteps := map[string]any{"type": "array", "items": map[string]any{"$ref": "#/$defs/step"}}
	pipeline := map[string]any{"oneOf": []any{
		pipelineSteps,
		map[string]any{
			"type": "object", "additionalProperties": false, "required": []string{"steps"},
			"properties": map[string]any{
				"needs": strList,
				"steps": pipelineSteps,
			},
		},
	}}
	portTemplate := map[string]any{"type": "string", "pattern": `\{port\}`, "description": "template containing {port}, replaced by the resolved port"}
	portEnv := map[string]any{
		"description": "variable name or variable-to-template bindings; an unconditional binding permits remapping",
		"oneOf": []any{
			map[string]any{"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
			map[string]any{
				"type": "object", "minProperties": 1,
				"propertyNames": map[string]any{"pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
				"additionalProperties": map[string]any{"oneOf": []any{
					portTemplate,
					map[string]any{
						"type": "object", "additionalProperties": false,
						"required": []string{"default"}, "properties": map[string]any{"default": portTemplate},
						"description": "inject only when the inherited/dotenv/service value is empty",
					},
				}},
			},
		},
	}
	port := map[string]any{
		"type":                 "object",
		"additionalProperties": false,
		"required":             []string{"default"},
		"properties": map[string]any{
			"default": map[string]any{"type": "integer", "minimum": 1, "maximum": 65535},
			"env":     portEnv,
			"probe":   map[string]any{"type": "boolean", "default": true, "description": "use this port for Rocket readiness; false still leases and remaps it"},
		},
	}
	health := map[string]any{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]any{
			"http":    map[string]any{"type": "string", "pattern": "^/", "description": "HTTP path probed on the health port"},
			"tcp":     map[string]any{"type": "boolean"},
			"port":    map[string]any{"type": "string", "description": "port name with probe enabled; defaults to the first eligible port"},
			"timeout": map[string]any{"type": "string", "description": "Go duration, e.g. 60s"},
		},
	}
	service := map[string]any{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]any{
			"compose":    map[string]any{"type": "string", "description": "service name in the env's compose files"},
			"task":       map[string]any{"type": "string", "description": "long-running go-task task"},
			"run":        map[string]any{"type": "string", "description": "raw shell command"},
			"cwd":        map[string]any{"type": "string", "description": "working dir relative to project root"},
			"env":        strMap,
			"dotenv":     strList,
			"profiles":   map[string]any{"type": "array", "items": str, "description": "gates startup wildcard selection; explicit members and required dependencies remain selectable"},
			"depends_on": strList,
			"ports":      map[string]any{"type": "object", "additionalProperties": map[string]any{"$ref": "#/$defs/port"}},
			"health":     map[string]any{"$ref": "#/$defs/health"},
		},
		"oneOf": []any{
			map[string]any{"required": []string{"compose"}},
			map[string]any{"required": []string{"task"}},
			map[string]any{"required": []string{"run"}},
		},
	}
	env := map[string]any{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]any{
			"compose":  strList,
			"profiles": map[string]any{"type": "array", "items": str, "description": "enabled startup profiles and Compose profiles for this environment"},
			"deploy": map[string]any{
				"type":                 "object",
				"additionalProperties": false,
				"properties": map[string]any{
					"task":    map[string]any{"type": "string", "description": "go-task task that deploys"},
					"run":     map[string]any{"type": "string", "description": "shell command that deploys"},
					"confirm": map[string]any{"type": "boolean", "description": "require --yes"},
				},
				"oneOf": []any{
					map[string]any{"required": []string{"task"}},
					map[string]any{"required": []string{"run"}},
				},
			},
		},
	}
	schema := map[string]any{
		"$schema":              "https://json-schema.org/draft/2020-12/schema",
		"$id":                  "https://github.com/xean-io/rocket/rocket.schema.json",
		"title":                "rocket.yaml",
		"type":                 "object",
		"additionalProperties": false,
		"required":             []string{"version", "services"},
		"properties": map[string]any{
			"version":     map[string]any{"const": 1},
			"name":        map[string]any{"type": "string", "pattern": "^[a-z0-9][a-z0-9._-]*$"},
			"dotenv":      strList,
			"default_env": str,
			"setup":       map[string]any{"type": "object", "additionalProperties": map[string]any{"$ref": "#/$defs/step"}},
			"envs":        map[string]any{"type": "object", "additionalProperties": map[string]any{"$ref": "#/$defs/env"}},
			"services":    map[string]any{"type": "object", "minProperties": 1, "additionalProperties": map[string]any{"$ref": "#/$defs/service"}},
			"groups":      map[string]any{"type": "object", "additionalProperties": strList},
			"pipelines":   map[string]any{"type": "object", "additionalProperties": pipeline},
		},
		"$defs": map[string]any{
			"step":    step,
			"port":    port,
			"health":  health,
			"service": service,
			"env":     env,
		},
	}
	out, _ := json.MarshalIndent(schema, "", "  ")
	return out
}
