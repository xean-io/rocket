package manifest

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestPortEnvTemplates(t *testing.T) {
	for _, tt := range []struct {
		name string
		env  string
		want string
	}{
		{"legacy scalar", "PORT", "PORT"},
		{"host port template", `{API_BIND: "0.0.0.0:{port}"}`, "API_BIND"},
		{"representative ignores defaults", `{A_PUBLIC_URL: {default: "http://localhost:{port}"}, Z_PORT: "{port}", API_BIND: "0.0.0.0:{port}"}`, "API_BIND"},
		{"default only cannot remap", `{PUBLIC_URL: {default: "http://localhost:{port}"}}`, ""},
	} {
		t.Run(tt.name, func(t *testing.T) {
			p, err := Parse([]byte("version: 1\nname: templates\nservices: {api: {task: dev, ports: {http: {default: 8080, env: "+tt.env+"}}}}"), "/code/templates")
			if err != nil {
				t.Fatalf("parse port env: %v", err)
			}
			if got := p.Services["api"].Ports[0].Env; got != tt.want {
				t.Fatalf("representative env = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestPortEnvTemplateValidation(t *testing.T) {
	for _, tt := range []struct {
		name string
		env  string
		want string
	}{
		{"missing placeholder", `{API_BIND: "0.0.0.0:8080"}`, "{port}"},
		{"default missing placeholder", `{PORT: "{port}", URL: {default: "http://localhost:8080"}}`, "{port}"},
		{"bad variable", `{'bad-name': "{port}"}`, "invalid env var name"},
		{"unknown default field", `{PORT: "{port}", URL: {default: "http://localhost:{port}", fallback: x}}`, "fallback"},
		{"wrong default object", `{PORT: {value: "{port}"}}`, "value"},
		{"numeric binding", `{PORT: 1234}`, "string"},
		{"null binding", `{PORT: null}`, "string"},
		{"numeric default", `{PORT: {default: 1234}}`, "string"},
		{"sequence", `[PORT]`, "string or mapping"},
		{"empty mapping", `{}`, "empty"},
		{"duplicate variable", `{PORT: "{port}", PORT: "{port}"}`, "already defined"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Parse([]byte("version: 1\nname: templates\nservices: {api: {run: dev, ports: {http: {default: 8080, env: "+tt.env+"}}}}"), "/code/templates")
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Fatalf("error = %v, want %q", err, tt.want)
			}
		})
	}
	_, err := Parse([]byte(`version: 1
name: templates
services: {api: {run: dev, ports: {http: {default: 8080, env: {PORT: "{port}"}, typo: true}}}}
`), "/code/templates")
	if err == nil || !strings.Contains(err.Error(), "typo") {
		t.Fatalf("unknown port field accepted: %v", err)
	}
}

func TestPortEnvTemplateSchema(t *testing.T) {
	var schema map[string]any
	if err := json.Unmarshal(Schema(), &schema); err != nil {
		t.Fatal(err)
	}
	port := schema["$defs"].(map[string]any)["port"].(map[string]any)
	env := port["properties"].(map[string]any)["env"].(map[string]any)
	choices, ok := env["oneOf"].([]any)
	if !ok || len(choices) != 2 {
		t.Fatalf("port env schema must support scalar and mapping: %v", env)
	}
	bindings := choices[1].(map[string]any)
	if bindings["type"] != "object" || bindings["minProperties"] != float64(1) {
		t.Fatalf("template binding map schema: %v", bindings)
	}
	values := bindings["additionalProperties"].(map[string]any)["oneOf"].([]any)
	defaultBinding := values[1].(map[string]any)
	if defaultBinding["additionalProperties"] != false {
		t.Fatalf("default binding allows unknown keys: %v", defaultBinding)
	}
}
