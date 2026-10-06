package manifest

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestPipelineNeedsAndLegacySteps(t *testing.T) {
	p, err := Parse([]byte(`version: 1
name: pipelines
services: {db: {run: database}, api: {run: api, depends_on: [db]}}
groups: {infra: [db]}
pipelines:
  legacy: [{run: lint}]
  check: {needs: [infra, api], steps: [{task: test}]}
`), "/code/pipelines")
	if err != nil {
		t.Fatalf("parse pipeline forms: %v", err)
	}
	if len(p.Pipelines["legacy"]) != 1 || p.Pipelines["legacy"][0].Run != "lint" || len(p.Pipelines["check"]) != 1 || p.Pipelines["check"][0].Task != "test" {
		t.Fatalf("pipeline steps: %v", p.Pipelines)
	}
}

func TestPipelineNeedsValidation(t *testing.T) {
	for _, tt := range []struct {
		name     string
		pipeline string
		want     string
	}{
		{"unknown prerequisite", `{needs: [missing], steps: [{run: test}]}`, "missing"},
		{"unknown object key", `{needs: [api], steps: [{run: test}], needz: []}`, "needz"},
		{"unknown object step key", `{needs: [api], steps: [{run: test, typo: x}]}`, "typo"},
		{"unknown legacy step key", `[{run: test, typo: x}]`, "typo"},
		{"missing steps", `{needs: [api]}`, "steps"},
		{"null steps", `{needs: [api], steps: null}`, "steps"},
		{"null needs", `{needs: null, steps: [{run: test}]}`, "needs"},
		{"invalid step", `{needs: [api], steps: [{}]}`, "exactly one"},
		{"wrong pipeline kind", `test`, "array or object"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Parse([]byte("version: 1\nname: pipelines\nservices: {api: {run: api}}\npipelines: {check: "+tt.pipeline+"}\n"), "/code/pipelines")
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Fatalf("error = %v, want %q", err, tt.want)
			}
		})
	}
}

func TestPipelineNeedsSchema(t *testing.T) {
	var schema map[string]any
	if err := json.Unmarshal(Schema(), &schema); err != nil {
		t.Fatal(err)
	}
	pipelines := schema["properties"].(map[string]any)["pipelines"].(map[string]any)
	definition := pipelines["additionalProperties"].(map[string]any)
	if choices, ok := definition["oneOf"].([]any); !ok || len(choices) != 2 {
		t.Fatalf("schema must accept legacy array and object: %v", definition)
	}
}
