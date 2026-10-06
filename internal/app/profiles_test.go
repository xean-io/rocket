package app

import (
	"context"
	"encoding/json"
	"errors"
	"slices"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
)

func profileProject(t *testing.T) *domain.Project {
	t.Helper()
	p, err := manifest.Parse([]byte(`version: 1
name: profiles
default_env: dev
envs:
  dev: {compose: [compose.yaml], profiles: [base]}
  smoke: {compose: [compose.yaml], profiles: [trends]}
services:
  api: {run: dev, depends_on: [required]}
  required: {run: dependency, profiles: [internal]}
  ordinary: {run: ordinary}
  trends: {run: trends, profiles: [trends]}
  reporting: {compose: reporting, profiles: [reports, trends]}
groups:
  all: ["*"]
  explicit: [trends]
  mixed: ["*", trends]
`), "/code/profiles")
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func profileRequest(t *testing.T, p *domain.Project, body string) UpRequest {
	t.Helper()
	var req UpRequest
	if err := json.Unmarshal([]byte(body), &req); err != nil {
		t.Fatal(err)
	}
	req.Project = p.Root
	return req
}

func profileHarness(t *testing.T) (*harness, *domain.Project) {
	t.Helper()
	h := newHarness(t)
	p := profileProject(t)
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	return h, p
}

func TestUpProfileSelection(t *testing.T) {
	for _, tt := range []struct {
		name, body string
		want       []string
	}{
		{"empty wildcard", `{}`, []string{"api", "ordinary", "required"}},
		{"literal wildcard", `{"services":["*"]}`, []string{"api", "ordinary", "required"}},
		{"wildcard group", `{"services":["all"]}`, []string{"api", "ordinary", "required"}},
		{"requested profiles", `{"services":["all"],"profiles":["reports"]}`, []string{"api", "ordinary", "reporting", "required"}},
		{"selected environment profiles", `{"services":["all"],"env":"smoke"}`, []string{"api", "ordinary", "reporting", "required", "trends"}},
		{"explicit service", `{"services":["trends"]}`, []string{"trends"}},
		{"explicit group member", `{"services":["explicit"]}`, []string{"trends"}},
		{"mixed wildcard and explicit member", `{"services":["mixed"]}`, []string{"api", "ordinary", "required", "trends"}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h, p := profileHarness(t)
			res := h.up(t, profileRequest(t, p, tt.body))
			var got []string
			for _, service := range res.Services {
				if service.Action != ActionStarted {
					t.Fatalf("startup failed: %+v", res)
				}
				got = append(got, service.Service)
			}
			slices.Sort(got)
			if !slices.Equal(got, tt.want) {
				t.Fatalf("started = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestComposeProfilesPersistThroughStop(t *testing.T) {
	h, p := profileHarness(t)
	req := profileRequest(t, p, `{"services":["reporting"],"profiles":["extra","base","extra"]}`)
	res := h.up(t, req)
	if res.Failed() {
		t.Fatalf("up = %+v", res)
	}
	want := []string{"base", "extra", "reports", "trends"}
	h.compose.mu.Lock()
	profiles := slices.Clone(h.compose.calls[0].target.Profiles)
	h.compose.mu.Unlock()
	if !slices.Equal(profiles, want) {
		t.Fatalf("compose profiles = %v, want %v", profiles, want)
	}
	// Stop must retain the actual launch selection even after manifest changes.
	p.Envs["dev"] = domain.Environment{Name: "dev", Compose: []string{"compose.yaml"}}
	svc := p.Services["reporting"]
	svc.Profiles = nil
	p.Services["reporting"] = svc
	if _, err := h.app.Down(context.Background(), DownRequest{Project: p.Root}); err != nil {
		t.Fatal(err)
	}
	h.compose.mu.Lock()
	defer h.compose.mu.Unlock()
	for _, call := range h.compose.calls {
		if !slices.Equal(call.target.Profiles, want) {
			t.Errorf("%s profiles = %v, want %v", call.op, call.target.Profiles, want)
		}
	}
}

func TestLegacyComposeRunReconstructsProfiles(t *testing.T) {
	h, p := profileHarness(t)
	target := h.app.composeTargetForRun(p, domain.Run{Project: p.Name, Service: "reporting", Env: "dev", Kind: domain.KindCompose})
	if !slices.Equal(target.Profiles, []string{"base", "reports", "trends"}) {
		t.Fatalf("legacy run profiles = %v", target.Profiles)
	}
}

func TestRestartProfileSelection(t *testing.T) {
	h, p := profileHarness(t)
	initial := h.up(t, profileRequest(t, p, `{"services":["all"],"profiles":["trends"]}`))
	oldTrends := h.run(t, p.Name, "trends")
	res, err := h.app.Restart(context.Background(), profileRequest(t, p, `{"services":["all"]}`))
	if err != nil || res.Failed() {
		t.Fatalf("restart = %+v, %v", res, err)
	}
	if got := h.run(t, p.Name, "trends"); got.PID != oldTrends.PID || !got.State.Active() {
		t.Fatalf("unselected gated service changed: %+v", got)
	}
	if len(res.Services) != 3 {
		t.Fatalf("restart selected gated service: %+v (initial %+v)", res, initial)
	}
	res, err = h.app.Restart(context.Background(), profileRequest(t, p, `{"services":["all"],"profiles":["trends"]}`))
	if err != nil || res.Failed() || len(res.Services) != 5 {
		t.Fatalf("profile restart = %+v, %v", res, err)
	}
	if got := h.run(t, p.Name, "trends"); got.PID == oldTrends.PID {
		t.Fatal("requested gated service was not restarted")
	}
}

func TestRestartRejectsUnknownEnvironmentBeforeStopping(t *testing.T) {
	h, p := profileHarness(t)
	h.up(t, profileRequest(t, p, `{"services":["ordinary"]}`))
	before := h.run(t, p.Name, "ordinary")
	_, err := h.app.Restart(context.Background(), profileRequest(t, p, `{"services":["ordinary"],"env":"unknown"}`))
	if !errors.Is(err, ErrInvalid) {
		t.Fatalf("restart error = %v, want invalid", err)
	}
	if after := h.run(t, p.Name, "ordinary"); after.PID != before.PID || !after.State.Active() {
		t.Fatalf("invalid environment stopped service: %+v", after)
	}
}

func TestRestartRejectsInvalidTTLBeforeStopping(t *testing.T) {
	for _, ttl := range []string{"nope", "0s", "-1m"} {
		t.Run(ttl, func(t *testing.T) {
			h, p := profileHarness(t)
			h.up(t, profileRequest(t, p, `{"services":["ordinary"]}`))
			before := h.run(t, p.Name, "ordinary")
			req := profileRequest(t, p, `{"services":["ordinary"]}`)
			req.TTL = ttl
			_, err := h.app.Restart(context.Background(), req)
			if !errors.Is(err, ErrInvalid) {
				t.Fatalf("restart error = %v, want invalid", err)
			}
			if after := h.run(t, p.Name, "ordinary"); after.PID != before.PID || !after.State.Active() {
				t.Fatalf("invalid TTL stopped service: %+v", after)
			}
		})
	}
}

func TestRestartEmptyWildcardDoesNotStopGatedRuns(t *testing.T) {
	h, p := profileHarness(t)
	p.Services = map[string]domain.Service{"trends": p.Services["trends"]}
	p.Groups = map[string][]string{"all": {"*"}}
	h.up(t, profileRequest(t, p, `{"services":["trends"]}`))
	before := h.run(t, p.Name, "trends")
	res, err := h.app.Restart(context.Background(), profileRequest(t, p, `{"services":["all"]}`))
	if err != nil || len(res.Services) != 0 {
		t.Fatalf("empty restart = %+v, %v", res, err)
	}
	if after := h.run(t, p.Name, "trends"); after.PID != before.PID || !after.State.Active() {
		t.Fatalf("empty wildcard stopped gated service: %+v", after)
	}
}

func TestDownWildcardAndEverywhereIgnoreProfiles(t *testing.T) {
	for _, tt := range []struct {
		name string
		req  DownRequest
	}{
		{"wildcard group", DownRequest{Services: []string{"all"}}},
		{"whole project", DownRequest{}},
		{"everywhere", DownRequest{Everywhere: true}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h, p := profileHarness(t)
			h.up(t, profileRequest(t, p, `{"services":["all"],"profiles":["trends"]}`))
			req := tt.req
			if !req.Everywhere {
				req.Project = p.Root
			}
			res, err := h.app.Down(context.Background(), req)
			if err != nil || len(res.Stopped) != 5 {
				t.Fatalf("down = %+v, %v", res, err)
			}
			for _, run := range res.Stopped {
				if run.State.Active() {
					t.Errorf("still active: %+v", run)
				}
			}
		})
	}
}
