package manifest

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func TestPortProbeParsing(t *testing.T) {
	for _, tt := range []struct {
		name, field string
		enabled     bool
	}{
		{"omitted defaults on", "", true},
		{"enabled", ", probe: true", true},
		{"disabled", ", probe: false", false},
		{"anchored disabled", ", probe: &disabled false", false},
	} {
		t.Run(tt.name, func(t *testing.T) {
			p, err := Parse([]byte("version: 1\nname: probes\nservices: {api: {run: dev, ports: {http: {default: 8080"+tt.field+"}}}}"), "/code/probes")
			if err != nil {
				t.Fatal(err)
			}
			got := domain.HealthCheckFor(p.Services["api"], map[string]int{"http": 8180})
			if (got != nil) != tt.enabled {
				t.Fatalf("probe enabled = %v, want %v (check %+v)", got != nil, tt.enabled, got)
			}
		})
	}
	p, err := Parse([]byte("version: 1\nname: probes\nservices: {api: {run: dev, ports: {debug: {default: 9000, probe: &disabled false}, http: {default: 8080, probe: *disabled}}}}"), "/code/probes")
	if err != nil {
		t.Fatal(err)
	}
	if check := domain.HealthCheckFor(p.Services["api"], map[string]int{"debug": 9000, "http": 8080}); check != nil {
		t.Fatalf("aliased false must disable probing: %+v", check)
	}
}

func TestPortProbeValidation(t *testing.T) {
	for _, value := range []string{"null", `"false"`, "off", "0", "[]", "{}"} {
		t.Run(value, func(t *testing.T) {
			_, err := Parse([]byte("version: 1\nname: probes\nservices: {api: {run: dev, ports: {http: {default: 8080, probe: "+value+"}}}}"), "/code/probes")
			if err == nil || !strings.Contains(err.Error(), "probe must be a boolean") {
				t.Fatalf("error = %v, want probe must be a boolean", err)
			}
		})
	}
	for _, tt := range []struct{ name, target, want string }{
		{"disabled explicit target", "debug", `health.port "debug" has probe disabled`},
		{"unknown explicit target", "unknown", `health.port "unknown" is not a declared port`},
		{"enabled explicit target", "http", ""},
		{"implicit eligible target", "", ""},
	} {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Parse([]byte("version: 1\nname: probes\nservices: {api: {run: dev, ports: {debug: {default: 9100, probe: false}, http: {default: 8080}}, health: {tcp: true, port: '"+tt.target+"'}}}"), "/code/probes")
			if tt.want == "" && err != nil {
				t.Fatal(err)
			}
			if tt.want != "" && (err == nil || !strings.Contains(err.Error(), tt.want)) {
				t.Fatalf("error = %v, want %q", err, tt.want)
			}
		})
	}
}

func TestPortProbeSchema(t *testing.T) {
	var schema map[string]any
	if err := json.Unmarshal(Schema(), &schema); err != nil {
		t.Fatal(err)
	}
	defs := schema["$defs"].(map[string]any)
	properties := defs["port"].(map[string]any)["properties"].(map[string]any)
	probe, ok := properties["probe"].(map[string]any)
	if !ok || probe["type"] != "boolean" || probe["default"] != true {
		t.Fatalf("probe schema = %v, want boolean default true", probe)
	}
}
