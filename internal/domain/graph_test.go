package domain

import (
	"reflect"
	"strings"
	"testing"
)

func sampleProject() *Project {
	return &Project{
		Name: "nuvara",
		Services: map[string]Service{
			"postgres":  {Name: "postgres", Kind: KindCompose, Compose: "postgres"},
			"redis":     {Name: "redis", Kind: KindCompose, Compose: "redis"},
			"api":       {Name: "api", Kind: KindRun, Run: "bun run dev", DependsOn: []string{"postgres", "redis"}},
			"web":       {Name: "web", Kind: KindRun, Run: "bun run dev", DependsOn: []string{"api"}},
			"causation": {Name: "causation", Kind: KindTask, Task: "dev:causation"},
		},
		Groups: map[string][]string{
			"deps": {"postgres", "redis"},
			"core": {"api", "web"},
			"all":  {"*"},
		},
	}
}

func TestExpandTargets(t *testing.T) {
	p := sampleProject()
	tests := []struct {
		name    string
		in      []string
		want    []string
		wantErr string
	}{
		{name: "empty means all services", in: nil, want: []string{"api", "causation", "postgres", "redis", "web"}},
		{name: "single service", in: []string{"web"}, want: []string{"web"}},
		{name: "group expands", in: []string{"deps"}, want: []string{"postgres", "redis"}},
		{name: "star group expands to all", in: []string{"all"}, want: []string{"api", "causation", "postgres", "redis", "web"}},
		{name: "literal star", in: []string{"*"}, want: []string{"api", "causation", "postgres", "redis", "web"}},
		{name: "duplicates removed", in: []string{"web", "core"}, want: []string{"api", "web"}},
		{name: "unknown target", in: []string{"nope"}, wantErr: `unknown service or group "nope"`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := p.ExpandTargets(tt.in)
			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("err = %v, want %q", err, tt.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("got %v want %v", got, tt.want)
			}
		})
	}
}

func TestStartOrderIncludesDependenciesFirst(t *testing.T) {
	p := sampleProject()
	got, err := p.StartOrder([]string{"web"})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"postgres", "redis", "api", "web"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v want %v", got, want)
	}
}

func TestStartOrderDetectsCycle(t *testing.T) {
	p := &Project{Services: map[string]Service{
		"a": {Name: "a", DependsOn: []string{"b"}},
		"b": {Name: "b", DependsOn: []string{"c"}},
		"c": {Name: "c", DependsOn: []string{"a"}},
	}}
	_, err := p.StartOrder([]string{"a"})
	if err == nil || !strings.Contains(err.Error(), "cycle") {
		t.Fatalf("err = %v, want cycle", err)
	}
}

func TestStopOrderIsReverseDependencyOrder(t *testing.T) {
	p := sampleProject()
	got := p.StopOrder([]string{"redis", "web", "api", "postgres"})
	want := []string{"web", "api", "redis", "postgres"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v want %v", got, want)
	}
}

func TestRemapCandidates(t *testing.T) {
	got := RemapCandidates(3000)
	if got[0] != 3100 || got[1] != 3101 || got[2] != 3102 {
		t.Fatalf("unexpected head %v", got[:3])
	}
	high := RemapCandidates(65500)
	for _, c := range high {
		if c > 65535 {
			t.Fatalf("candidate %d out of range", c)
		}
	}
}

func TestComposeProjectName(t *testing.T) {
	if got := ComposeProjectName("Xean.SpectrAI", "dev"); got != "rocket-xean-spectrai-dev" {
		t.Fatalf("got %q", got)
	}
}

func TestBuildEnvPrecedence(t *testing.T) {
	base := []string{"PATH=/bin", "PORT=1", "ROCKET_OWNER=agent:x", "KEEP=base"}
	got := BuildEnv(base,
		map[string]string{"PORT": "2", "FROM_DOTENV": "yes", "KEEP": "dotenv"},
		map[string]string{"KEEP": "svc"},
		map[string]string{"PORT": "3100"},
	)
	m := map[string]string{}
	for _, kv := range got {
		k, v, _ := strings.Cut(kv, "=")
		m[k] = v
	}
	if m["PORT"] != "3100" || m["KEEP"] != "svc" || m["FROM_DOTENV"] != "yes" || m["PATH"] != "/bin" {
		t.Fatalf("unexpected env %v", m)
	}
	if _, ok := m["ROCKET_OWNER"]; ok {
		t.Fatalf("ROCKET_* vars must not leak to children")
	}
}

func TestRunStateActive(t *testing.T) {
	for _, s := range []RunState{StateStarting, StateRunning, StateStopping} {
		if !s.Active() {
			t.Errorf("%s should be active", s)
		}
	}
	for _, s := range []RunState{StateStopped, StateExited, StateFailed, StateDead} {
		if s.Active() {
			t.Errorf("%s should not be active", s)
		}
	}
}
