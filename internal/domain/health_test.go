package domain

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestHealthCheckForProbeSelection(t *testing.T) {
	for _, tt := range []struct {
		name    string
		service string
		want    *HealthCheck
	}{
		{"legacy default", `{"kind":"run","ports":[{"name":"debug"},{"name":"http"}]}`, &HealthCheck{Kind: "tcp", Port: 9100}},
		{"explicit true", `{"kind":"task","ports":[{"name":"debug","probe":true}]}`, &HealthCheck{Kind: "tcp", Port: 9100}},
		{"first eligible", `{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http"}]}`, &HealthCheck{Kind: "tcp", Port: 8180}},
		{"http on first eligible", `{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http"}],"health":{"http":"/ready"}}`, &HealthCheck{Kind: "http", Port: 8180, Path: "/ready"}},
		{"explicit eligible", `{"kind":"run","ports":[{"name":"debug"},{"name":"http","probe":true}],"health":{"port":"http"}}`, &HealthCheck{Kind: "tcp", Port: 8180}},
		{"all disabled", `{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http","probe":false}],"health":{"http":"/ready"}}`, nil},
		{"explicit disabled is never probed", `{"kind":"run","ports":[{"name":"debug","probe":false},{"name":"http"}],"health":{"port":"debug"}}`, nil},
		{"unknown explicit port", `{"kind":"run","ports":[{"name":"debug"}],"health":{"port":"unknown"}}`, nil},
		{"compose owns default readiness", `{"kind":"compose","ports":[{"name":"http"}]}`, nil},
		{"compose explicit tcp eligible", `{"kind":"compose","ports":[{"name":"debug","probe":false},{"name":"http"}],"health":{"tcp":true}}`, &HealthCheck{Kind: "tcp", Port: 8180}},
		{"compose tcp disabled", `{"kind":"compose","ports":[{"name":"http","probe":false}],"health":{"tcp":true}}`, nil},
		{"no ports", `{"kind":"run"}`, nil},
	} {
		t.Run(tt.name, func(t *testing.T) {
			var svc Service
			if err := json.Unmarshal([]byte(tt.service), &svc); err != nil {
				t.Fatal(err)
			}
			got := HealthCheckFor(svc, map[string]int{"debug": 9100, "http": 8180, "unknown": 9999})
			if !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("health check = %+v, want %+v", got, tt.want)
			}
		})
	}
	if got := HealthCheckFor(Service{Kind: KindRun, Ports: []PortSpec{{Name: "http"}}}, nil); got != nil {
		t.Fatalf("unresolved port must not be probed: %+v", got)
	}
}
