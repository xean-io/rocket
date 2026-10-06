package app

import (
	"slices"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
)

func TestUpProbeSettingsPreserveLeasesAndRemapping(t *testing.T) {
	for _, tt := range []struct {
		name, kind, httpProbe, health string
		wantChecks                    []domain.HealthCheck
	}{
		{"mixed local", "run: dev", "", "http: /ready", []domain.HealthCheck{{Kind: "http", Port: 8180, Path: "/ready"}}},
		{"disabled local", "task: dev", ", probe: false", "http: /ready", nil},
		{"disabled compose", "compose: backend", ", probe: false", "tcp: true", nil},
		{"mixed compose explicit tcp", "compose: backend", "", "tcp: true", []domain.HealthCheck{{Kind: "tcp", Port: 8180}}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			p, err := manifest.Parse([]byte(`version: 1
name: probes
envs: {dev: {compose: [compose.yaml]}}
default_env: dev
services:
  api:
    `+tt.kind+`
    ports:
      debug: {default: 9000, env: DEBUG_PORT, probe: false}
      http: {default: 8080, env: PORT`+tt.httpProbe+`}
    health: {`+tt.health+`, timeout: 10ms}
`), "/code/probes")
			if err != nil {
				t.Fatal(err)
			}
			h := newHarness(t)
			h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
			h.probe.busy[9000] = &domain.PortHolder{PID: 99, Command: "foreign"}
			h.probe.busy[8080] = &domain.PortHolder{PID: 98, Command: "foreign"}
			h.health.failing[9100] = true
			if tt.httpProbe != "" {
				h.health.failing[8180] = true
			}
			res := h.up(t, UpRequest{Project: p.Root})
			if res.Failed() || len(res.Services) != 1 || res.Services[0].Action != ActionStarted {
				t.Fatalf("up = %+v", res)
			}
			result := res.Services[0]
			if result.Ports["debug"] != 9100 || result.Ports["http"] != 8180 || len(result.Remaps) != 2 {
				t.Fatalf("ports/remaps = %+v", result)
			}
			leases, err := h.store.ListLeases()
			if err != nil || len(leases) != 2 || leases[0].Port != 8180 || leases[1].Port != 9100 {
				t.Fatalf("leases = %+v, error %v", leases, err)
			}
			h.health.mu.Lock()
			checks := slices.Clone(h.health.checks)
			h.health.mu.Unlock()
			if !slices.Equal(checks, tt.wantChecks) {
				t.Fatalf("health checks = %+v, want %+v", checks, tt.wantChecks)
			}
			var env map[string]string
			if p.Services["api"].Kind == domain.KindCompose {
				if got := h.compose.ops(); !slices.Equal(got, []string{"up:backend"}) {
					t.Fatalf("compose readiness must still run: %v", got)
				}
				h.compose.mu.Lock()
				env = envMap(h.compose.calls[0].target.Env)
				h.compose.mu.Unlock()
			} else {
				env = envMap(h.runner.specFor(t, p.Root).Env)
			}
			if env["DEBUG_PORT"] != "9100" || env["PORT"] != "8180" {
				t.Fatalf("injected ports = %v", env)
			}
		})
	}
}

func TestUpDisabledProbeCannotBypassPortConflict(t *testing.T) {
	p, err := manifest.Parse([]byte("version: 1\nname: probes\nservices: {api: {run: dev, ports: {http: {default: 8080, probe: false}}}}"), "/code/probes")
	if err != nil {
		t.Fatal(err)
	}
	h := newHarness(t)
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	h.probe.busy[8080] = &domain.PortHolder{PID: 99}
	res := h.up(t, UpRequest{Project: p.Root})
	if !res.Failed() || len(h.runner.started) != 0 {
		t.Fatalf("disabled probe bypassed lease conflict: %+v", res)
	}
	leases, err := h.store.ListLeases()
	if err != nil || len(leases) != 0 {
		t.Fatalf("leases = %v, error %v", leases, err)
	}
}
