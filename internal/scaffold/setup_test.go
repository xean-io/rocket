package scaffold

import (
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/xean-io/rocket/internal/manifest"
)

func TestDetectSetupInstallTaskPrecedence(t *testing.T) {
	for _, tt := range []struct {
		name, tasks, wantTask string
	}{
		{"setup fallback", "  setup: echo setup\n", "setup"},
		{"explicit install", "  install: echo install\n", "install"},
		{"setup before install", "  setup: echo setup\n  install: echo install\n", "install"},
		{"install before setup", "  install: echo install\n  setup: echo setup\n", "install"},
		{"namespaced only", "", ""},
		{"internal setup", "  setup: {internal: true, cmds: [echo setup]}\n", ""},
	} {
		t.Run(tt.name, func(t *testing.T) {
			root := t.TempDir()
			rootTaskfile := "version: '3'\nincludes: {infra: ./infra.yml}\ntasks:\n  dev: echo dev\n" + tt.tasks
			for name, text := range map[string]string{
				"Taskfile.yml": rootTaskfile,
				"infra.yml":    "version: '3'\ntasks:\n  setup: echo included setup\n  install: echo included install\n",
			} {
				if err := os.WriteFile(filepath.Join(root, name), []byte(text), 0o644); err != nil {
					t.Fatal(err)
				}
			}
			r, err := Detect(root)
			if err != nil {
				t.Fatal(err)
			}
			var task string
			for _, step := range r.Setup {
				if step.Name == "install" {
					task = step.Task
				}
			}
			if task != tt.wantTask {
				t.Errorf("setup.install = %q, want %q", task, tt.wantTask)
			}
			if slices.Contains(r.Pipelines, "setup") {
				t.Errorf("root setup must not be exposed as an arbitrary pipeline: %v", r.Pipelines)
			}
			if !slices.Equal(r.Pipelines, []string{"infra:install", "infra:setup"}) {
				t.Errorf("namespaced task classification changed: %v", r.Pipelines)
			}
			p, err := manifest.Parse(r.Render(), root)
			if err != nil {
				t.Fatalf("rendered manifest invalid: %v\n%s", err, r.Render())
			}
			if got := p.Setup["install"].Task; got != tt.wantTask {
				t.Errorf("rendered setup.install = %q, want %q", got, tt.wantTask)
			}
			if _, exists := p.Pipelines["setup"]; exists {
				t.Error("rendered root setup pipeline")
			}
		})
	}
}
