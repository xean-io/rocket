package app

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"reflect"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

type hintCompose struct {
	*fakeCompose
	volumes      []string
	exists       map[string]bool
	created      []string
	configErr    error
	inspectErrAt int
	inspections  int
	upErr        error
	targets      []ports.ComposeTarget
}

func (c *hintCompose) NamedVolumes(_ context.Context, target ports.ComposeTarget, _ string) ([]string, error) {
	c.targets = append(c.targets, target)
	return c.volumes, c.configErr
}

func (c *hintCompose) VolumeExists(_ context.Context, _ ports.ComposeTarget, name string) (bool, error) {
	c.inspections++
	if c.inspections == c.inspectErrAt {
		return false, errors.New("volume listing unavailable")
	}
	return c.exists[name], nil
}

func (c *hintCompose) Up(ctx context.Context, target ports.ComposeTarget, service string, out io.Writer) (string, error) {
	for _, volume := range c.created {
		c.exists[volume] = true
	}
	if c.upErr != nil {
		return "", c.upErr
	}
	return c.fakeCompose.Up(ctx, target, service, out)
}

func resultHints(t *testing.T, result UpResult) []string {
	t.Helper()
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	var wire struct {
		Hints []string `json:"hints"`
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		t.Fatal(err)
	}
	return wire.Hints
}

func TestUpNewComposeVolumeHints(t *testing.T) {
	for _, tt := range []struct {
		name         string
		setup        map[string]domain.Step
		exists       bool
		created      bool
		configErr    bool
		inspectErrAt int
		upErr        bool
		want         []string
	}{
		{"fresh both actions", map[string]domain.Step{"migrate": {Task: "db:migrate"}, "assets": {Task: "infra:assets"}}, false, true, false, 0, false, []string{"rocket migrate", "rocket setup assets"}},
		{"fresh migration only", map[string]domain.Step{"migrate": {Run: "migrate"}}, false, true, false, 0, false, []string{"rocket migrate"}},
		{"fresh assets only", map[string]domain.Step{"assets": {Task: "infra:assets"}}, false, true, false, 0, false, []string{"rocket setup assets"}},
		{"fresh no configured actions", nil, false, true, false, 0, false, nil},
		{"existing volume", map[string]domain.Step{"migrate": {Task: "migrate"}}, true, true, false, 0, false, nil},
		{"config error", map[string]domain.Step{"migrate": {Task: "migrate"}}, false, true, true, 0, false, nil},
		{"before inspect error", map[string]domain.Step{"migrate": {Task: "migrate"}}, false, true, false, 1, false, nil},
		{"after inspect error", map[string]domain.Step{"migrate": {Task: "migrate"}}, false, true, false, 2, false, nil},
		{"not created", map[string]domain.Step{"migrate": {Task: "migrate"}}, false, false, false, 0, false, nil},
		{"failed startup without creation", map[string]domain.Step{"migrate": {Task: "migrate"}}, false, false, false, 0, true, nil},
		{"failed startup with after evidence", map[string]domain.Step{"migrate": {Task: "migrate"}}, false, true, false, 0, true, []string{"rocket migrate"}},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h := newHarness(t)
			p := &domain.Project{Name: "volumes", Root: "/code/volumes", DefaultEnv: "dev", Setup: tt.setup,
				Envs:     map[string]domain.Environment{"dev": {Name: "dev", Compose: []string{"compose.yaml"}, Profiles: []string{"infra"}}},
				Services: map[string]domain.Service{"db": {Name: "db", Kind: domain.KindCompose, Compose: "database", Profiles: []string{"data"}}}}
			h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
			c := &hintCompose{fakeCompose: h.compose, volumes: []string{"rocket-volumes-dev_data", "rocket-volumes-dev_data"}, exists: map[string]bool{"rocket-volumes-dev_data": tt.exists}, inspectErrAt: tt.inspectErrAt}
			if tt.created {
				c.created = []string{"rocket-volumes-dev_data"}
			}
			if tt.configErr {
				c.configErr = errors.New("invalid config")
			}
			if tt.upErr {
				c.upErr = errors.New("startup failed")
			}
			h.app.d.Compose = c
			res := h.up(t, UpRequest{Project: p.Root, Services: []string{"db"}, Profiles: []string{"requested"}})
			if res.Failed() != tt.upErr {
				t.Fatalf("volume evidence changed startup result: %+v", res)
			}
			hints := resultHints(t, res)
			if len(tt.want) == 0 {
				if len(hints) != 0 {
					t.Fatalf("unsupported new-volume claim: %v", hints)
				}
			} else {
				if len(hints) != 1 || strings.Count(hints[0], "rocket-volumes-dev_data") != 1 {
					t.Fatalf("new volume hint must be deduplicated: %v", hints)
				}
				for _, action := range tt.want {
					if !strings.Contains(hints[0], action) {
						t.Errorf("hint %q missing %q", hints[0], action)
					}
				}
			}
			if len(c.targets) != 1 || c.targets[0].ProjectName != "rocket-volumes-dev" || !reflect.DeepEqual(c.targets[0].Profiles, []string{"data", "infra", "requested"}) || envMap(c.targets[0].Env)["COMPOSE_PROJECT_NAME"] != "rocket-volumes-dev" {
				t.Fatalf("volume config target must match launch identity/environment: %+v", c.targets)
			}
		})
	}
}

func TestUpVolumeHintsRequireActualNewStartup(t *testing.T) {
	h := newHarness(t)
	p := nuvara()
	p.Setup = map[string]domain.Step{"migrate": {Task: "migrate"}}
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	c := &hintCompose{fakeCompose: h.compose, volumes: []string{"rocket-nuvara-dev_data"}, exists: map[string]bool{}, created: []string{"rocket-nuvara-dev_data"}}
	h.app.d.Compose = c
	first := h.up(t, UpRequest{Services: []string{"postgres"}})
	if len(resultHints(t, first)) != 1 {
		t.Fatalf("first startup hint missing: %+v", first)
	}
	second := h.up(t, UpRequest{Services: []string{"postgres"}})
	if len(resultHints(t, second)) != 0 || len(c.targets) != 1 {
		t.Fatalf("idempotent up repeated volume hint: %+v; config calls=%d", second, len(c.targets))
	}
}

func TestUpVolumeHintsSortAndDeduplicateAcrossServices(t *testing.T) {
	h := newHarness(t)
	p := nuvara()
	p.Setup = map[string]domain.Step{"migrate": {Task: "migrate"}}
	p.Services = map[string]domain.Service{
		"one": {Name: "one", Kind: domain.KindCompose, Compose: "one"},
		"two": {Name: "two", Kind: domain.KindCompose, Compose: "two"},
	}
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	c := &hintCompose{fakeCompose: h.compose, volumes: []string{"rocket-fixture_z", "rocket-fixture_a", "rocket-fixture_z"}, exists: map[string]bool{}, created: []string{"rocket-fixture_z", "rocket-fixture_a"}}
	h.app.d.Compose = c
	res := h.up(t, UpRequest{Services: []string{"one", "two"}})
	hints := resultHints(t, res)
	if len(hints) != 1 || !strings.Contains(hints[0], "rocket-fixture_a, rocket-fixture_z") || strings.Count(hints[0], "rocket-fixture_z") != 1 {
		t.Fatalf("unstable/duplicate project hint: %v", hints)
	}
}

func TestVolumeHintTargetsProjectAndSelectedEnvironment(t *testing.T) {
	p := &domain.Project{Name: "fixture", DefaultEnv: "dev", Setup: map[string]domain.Step{"migrate": {Run: "migrate"}, "assets": {Task: "assets"}}}
	for _, env := range []string{"dev", "smoke", "stage 'name"} {
		hints := composeVolumeHints(p, env, map[string]bool{"volume": true})
		want := []string{"rocket migrate -p fixture", "rocket setup assets -p fixture"}
		if env == "smoke" {
			want = []string{"rocket migrate -p fixture --env smoke", "rocket setup assets -p fixture --env smoke"}
		} else if env != "dev" {
			want = []string{`rocket migrate -p fixture --env 'stage '"'"'name'`, `rocket setup assets -p fixture --env 'stage '"'"'name'`}
		}
		for _, command := range want {
			if len(hints) != 1 || !strings.Contains(hints[0], command) {
				t.Errorf("hint %v missing usable command %q", hints, command)
			}
		}
		if env == "dev" && strings.Contains(hints[0], "--env") {
			t.Errorf("default environment unnecessarily specified: %v", hints)
		}
	}
}
