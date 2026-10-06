package app

import (
	"context"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
)

func TestUpPortEnvTemplates(t *testing.T) {
	for _, tt := range []struct {
		name       string
		base       []string
		dotenv     map[string]string
		serviceEnv string
		wantURL    string
	}{
		{"unset default", nil, nil, "{}", "http://localhost:8180"},
		{"base override", []string{"PUBLIC_URL=https://base.example"}, nil, "{}", "https://base.example"},
		{"dotenv override", []string{"PUBLIC_URL=https://base.example"}, map[string]string{"PUBLIC_URL": "https://dotenv.example"}, "{}", "https://dotenv.example"},
		{"service override", nil, map[string]string{"PUBLIC_URL": "https://dotenv.example"}, `{PUBLIC_URL: "https://service.example"}`, "https://service.example"},
		{"empty override falls back", []string{"PUBLIC_URL=https://base.example"}, map[string]string{"PUBLIC_URL": ""}, "{}", "http://localhost:8180"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			p, err := manifest.Parse([]byte(`version: 1
name: templates
services:
  api:
    task: api:dev
    env: `+tt.serviceEnv+`
    ports:
      http:
        default: 8080
        env:
          PORT: "{port}"
          API_BIND: "0.0.0.0:{port}"
          PUBLIC_URL: {default: "http://localhost:{port}"}
`), "/code/templates")
			if err != nil {
				t.Fatalf("parse port templates: %v", err)
			}
			h := newHarness(t)
			h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
			h.app.d.Env = fakeEnv{values: tt.dotenv}
			h.app.d.BaseEnv = func() []string {
				return append([]string{"PORT=9999", "API_BIND=ignored", "ROCKET_OWNER=agent:test"}, tt.base...)
			}
			h.probe.busy[8080] = &domain.PortHolder{PID: 77, Command: "foreign"}
			res := h.up(t, UpRequest{Project: p.Root})
			if res.Failed() || len(res.Services) != 1 || res.Services[0].Ports["http"] != 8180 {
				t.Fatalf("up result: %+v", res)
			}
			remaps := res.Services[0].Remaps
			if len(remaps) != 1 || remaps[0].Env != "API_BIND" || remaps[0].Holder.PID != 77 {
				t.Fatalf("remap env must remain a representative string: %+v", remaps)
			}
			spec := h.runner.specFor(t, p.Root)
			if len(spec.Argv) != 2 || spec.Argv[0] != "task" || spec.Argv[1] != "api:dev" {
				t.Fatalf("task argv: %v", spec.Argv)
			}
			env := envMap(spec.Env)
			for name, want := range map[string]string{"PORT": "8180", "API_BIND": "0.0.0.0:8180", "PUBLIC_URL": tt.wantURL} {
				if env[name] != want {
					t.Errorf("%s = %q, want %q", name, env[name], want)
				}
			}
			if _, ok := env["ROCKET_OWNER"]; ok {
				t.Error("inherited ROCKET_OWNER leaked")
			}
			if _, err := h.app.Down(context.Background(), DownRequest{Project: p.Root}); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestUpDefaultPortBindingCannotRemap(t *testing.T) {
	p, err := manifest.Parse([]byte(`version: 1
name: templates
services: {api: {run: dev, ports: {http: {default: 8080, env: {PUBLIC_URL: {default: "http://localhost:{port}"}}}}}}
`), "/code/templates")
	if err != nil {
		t.Fatalf("parse default port binding: %v", err)
	}
	h := newHarness(t)
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	h.probe.busy[8080] = &domain.PortHolder{PID: 77}
	res := h.up(t, UpRequest{Project: p.Root})
	if !res.Failed() || len(h.runner.started) != 0 {
		t.Fatalf("default-only binding must not authorize remap: %+v", res)
	}
	leases, err := h.store.ListLeases()
	if err != nil || len(leases) != 0 {
		t.Fatalf("failed start leaked leases: %v, %v", leases, err)
	}
}
