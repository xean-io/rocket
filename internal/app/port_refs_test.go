package app

import (
	"context"
	"errors"
	"slices"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
)

func portRefsHarness(t *testing.T) (*harness, *domain.Project) {
	t.Helper()
	p, err := manifest.Parse([]byte(`version: 1
name: refs
envs: {dev: {}, smoke: {}}
services:
  api: {run: api-server, cwd: api, ports: {http: {default: 8080, env: PORT}}}
  web:
    task: web:dev
    cwd: web
    env:
      API_URL: "http://127.0.0.1:{api.http}/{api.http}/${api.http}"
      STATIC_VALUE: "literal {value} ${ghost.http}"
`), "/code/refs")
	if err != nil {
		t.Fatal(err)
	}
	h := newHarness(t)
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	h.probe.busy[8080] = &domain.PortHolder{PID: 77, Command: "foreign"}
	return h, p
}

func TestUpCrossServicePortReferences(t *testing.T) {
	h, p := portRefsHarness(t)
	res := h.up(t, UpRequest{Project: p.Root, Services: []string{"web"}, Env: "smoke"})
	var names []string
	for _, svc := range res.Services {
		names = append(names, svc.Service)
		if svc.Action != ActionStarted {
			t.Fatalf("startup failed: %+v", res)
		}
	}
	if !slices.Equal(names, []string{"api", "web"}) {
		t.Fatalf("provider must start before consumer: %v", names)
	}
	env := envMap(h.runner.specFor(t, "web").Env)
	if env["API_URL"] != "http://127.0.0.1:8180/8180/${api.http}" || env["STATIC_VALUE"] != "literal {value} ${ghost.http}" {
		t.Fatalf("consumer env: %v", env)
	}
	if h.run(t, p.Name, "api").Env != "smoke" || h.run(t, p.Name, "web").Env != "smoke" {
		t.Fatal("provider and consumer must use selected environment")
	}
	if p.Services["web"].Env["API_URL"] != "http://127.0.0.1:{api.http}/{api.http}/${api.http}" {
		t.Fatal("runtime expansion mutated the manifest")
	}
}

func TestUpPortReferenceProviderFailureSkipsConsumer(t *testing.T) {
	h, p := portRefsHarness(t)
	api := p.Services["api"]
	api.Ports[0].Env = ""
	p.Services["api"] = api
	res := h.up(t, UpRequest{Project: p.Root, Services: []string{"web"}})
	if actions(res)["api"] != ActionFailed || actions(res)["web"] != ActionSkipped || len(h.runner.started) != 0 {
		t.Fatalf("provider failure must prevent consumer start: %+v", res)
	}
}

func TestUpPortReferencesRejectLiveEnvironmentMismatch(t *testing.T) {
	for _, tt := range []struct {
		name            string
		initialServices []string
	}{
		{"provider", []string{"api"}},
		{"consumer", []string{"web"}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h, p := portRefsHarness(t)
			h.up(t, UpRequest{Project: p.Root, Services: tt.initialServices, Env: "dev"})
			if tt.name == "consumer" {
				if _, err := h.app.Down(context.Background(), DownRequest{Project: p.Root, Services: []string{"api"}}); err != nil {
					t.Fatal(err)
				}
				h.up(t, UpRequest{Project: p.Root, Services: []string{"api"}, Env: "smoke"})
			}
			before := len(h.runner.started)
			stoppedBefore := len(h.runner.stopped)
			_, err := h.app.Up(context.Background(), UpRequest{Project: p.Root, Services: []string{"web"}, Env: "smoke"})
			if !errors.Is(err, ErrInvalid) || !strings.Contains(err.Error(), "dev") || !strings.Contains(err.Error(), "smoke") {
				t.Fatalf("environment mismatch error = %v", err)
			}
			if len(h.runner.started) != before || len(h.runner.stopped) != stoppedBefore {
				t.Fatal("mismatched up must leave existing services untouched")
			}
		})
	}
}

func TestUpPortReferencesUseComposeProviderAlias(t *testing.T) {
	for _, reuse := range []bool{false, true} {
		name := "fresh provider"
		if reuse {
			name = "live provider"
		}
		t.Run(name, func(t *testing.T) {
			h, p := portRefsHarness(t)
			api := p.Services["api"]
			api.Kind, api.Run, api.Compose = domain.KindCompose, "", "backend"
			p.Services["api"] = api
			if reuse {
				h.up(t, UpRequest{Project: p.Root, Services: []string{"api"}})
			}
			res := h.up(t, UpRequest{Project: p.Root, Services: []string{"web"}})
			if res.Failed() || actions(res)["web"] != ActionStarted {
				t.Fatalf("compose alias must resolve as a live provider: %+v", res)
			}
			if reuse && actions(res)["api"] != ActionAlreadyRunning {
				t.Fatalf("live compose provider must be reused: %+v", res)
			}
			if got := envMap(h.runner.specFor(t, "web").Env)["API_URL"]; got != "http://127.0.0.1:8180/8180/${api.http}" {
				t.Fatalf("consumer URL = %q", got)
			}
		})
	}
}

func TestUpPortReferencesUseLiveProviderAndPreserveConsumerIdempotency(t *testing.T) {
	h, p := portRefsHarness(t)
	h.up(t, UpRequest{Project: p.Root, Services: []string{"api"}, Env: "smoke"})
	res := h.up(t, UpRequest{Project: p.Root, Services: []string{"web"}, Env: "smoke"})
	if actions(res)["api"] != ActionAlreadyRunning || actions(res)["web"] != ActionStarted {
		t.Fatalf("live provider should be reused: %+v", res)
	}
	if got := envMap(h.runner.specFor(t, "web").Env)["API_URL"]; got != "http://127.0.0.1:8180/8180/${api.http}" {
		t.Fatalf("URL = %q", got)
	}
	if _, err := h.app.Down(context.Background(), DownRequest{Project: p.Root, Services: []string{"api"}}); err != nil {
		t.Fatal(err)
	}
	delete(h.probe.busy, 8080)
	h.up(t, UpRequest{Project: p.Root, Services: []string{"api"}, Env: "smoke"})
	before := len(h.runner.started)
	res = h.up(t, UpRequest{Project: p.Root, Services: []string{"web"}, Env: "smoke"})
	if actions(res)["web"] != ActionAlreadyRunning || len(h.runner.started) != before {
		t.Fatalf("up must preserve existing consumer: %+v", res)
	}
	res, err := h.app.Restart(context.Background(), UpRequest{Project: p.Root, Services: []string{"web"}, Env: "smoke"})
	if err != nil || actions(res)["web"] != ActionStarted {
		t.Fatalf("restart = %+v, %v", res, err)
	}
	lastEnv := envMap(h.runner.started[len(h.runner.started)-1].Env)
	if lastEnv["API_URL"] != "http://127.0.0.1:8080/8080/${api.http}" {
		t.Fatalf("restart must refresh consumer URL: %v", lastEnv)
	}
}
