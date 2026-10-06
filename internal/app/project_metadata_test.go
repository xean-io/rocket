package app

import (
	"context"
	"encoding/json"
	"slices"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func TestSummaryDeclaredProjectLists(t *testing.T) {
	for _, empty := range []bool{false, true} {
		t.Run(map[bool]string{false: "declared sorted", true: "empty arrays"}[empty], func(t *testing.T) {
			h := newHarness(t)
			p := jobsProject()
			want := map[string][]string{"envs": {"dev", "prod", "stage"}, "pipelines": {"broken", "ci", "slow"}, "deploy_envs": {"prod", "stage"}}
			if empty {
				p.Envs, p.Pipelines = nil, nil
				want = map[string][]string{"envs": {}, "pipelines": {}, "deploy_envs": {}}
			}
			h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
			summary, err := h.app.Summary(context.Background(), p.Root)
			if err != nil {
				t.Fatal(err)
			}
			data, _ := json.Marshal(summary.Project)
			var wire map[string]json.RawMessage
			if err := json.Unmarshal(data, &wire); err != nil {
				t.Fatal(err)
			}
			for key, expected := range want {
				var got []string
				if err := json.Unmarshal(wire[key], &got); err != nil || got == nil || !slices.Equal(got, expected) {
					t.Errorf("project.%s = %s, want %v (non-null array), error=%v", key, wire[key], expected, err)
				}
			}
			if summary.Project.Name != p.Name || summary.Project.Root != p.Root || summary.Project.DefaultEnv != p.DefaultEnv {
				t.Fatalf("legacy project fields changed: %+v", summary.Project)
			}
		})
	}
}
