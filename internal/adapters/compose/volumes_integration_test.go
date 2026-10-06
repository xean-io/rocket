//go:build integration && !windows

package compose

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

func TestVolumeCommandsAndErrors(t *testing.T) {
	if testing.Short() {
		t.Skip("command integration")
	}
	dir := t.TempDir()
	trace := filepath.Join(dir, "argv")
	bin := filepath.Join(dir, "docker-fixture")
	script := `#!/bin/sh
case "$1" in
  compose)
    printf '%s\n' "$@" > "$TRACE_FILE"
    if [ "$FAIL_CONFIG" = 1 ]; then exit 1; fi
    printf '%s\n' '{"services":{"db":{"volumes":[{"type":"volume","source":"data"}]}},"volumes":{"data":{"name":"fixture_data"}}}'
    ;;
  volume)
    if [ "$INVENTORY_MARKER" != from-target ] || [ ! . -ef "$EXPECTED_DIR" ]; then
      printf '%s\n' wrong_endpoint
      exit 0
    fi
    if [ "$FAIL_VOLUMES" = 1 ]; then exit 1; fi
    printf '%s\n' fixture_data fixture_data_old
    ;;
esac
`
	if err := os.WriteFile(bin, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	d := Driver{Bin: bin}
	target := ports.ComposeTarget{ProjectName: "rocket-volumes-fixture-dev", Dir: dir, Files: []string{"compose.yaml"}, Profiles: []string{"fixture"}, Env: domain.BuildEnv(os.Environ(), map[string]string{"TRACE_FILE": trace, "INVENTORY_MARKER": "from-target", "EXPECTED_DIR": dir})}
	ctx := context.Background()
	names, err := d.NamedVolumes(ctx, target, "db")
	if err != nil || !slices.Equal(names, []string{"fixture_data"}) {
		t.Fatalf("config volumes = %v, %v", names, err)
	}
	got, err := os.ReadFile(trace)
	if err != nil {
		t.Fatal(err)
	}
	wantArgs := []string{"compose", "-p", target.ProjectName, "-f", "compose.yaml", "--profile", "fixture", "config", "--format", "json"}
	if !slices.Equal(strings.Fields(string(got)), wantArgs) {
		t.Fatalf("config arguments = %q, want %v", got, wantArgs)
	}
	for _, tt := range []struct {
		name   string
		exists bool
	}{{"fixture_data", true}, {"fixture", false}, {"missing", false}} {
		exists, err := d.VolumeExists(ctx, target, tt.name)
		if err != nil || exists != tt.exists {
			t.Errorf("exact name %q exists = %v, %v; want %v", tt.name, exists, err, tt.exists)
		}
	}
	target.Env = domain.BuildEnv(target.Env, map[string]string{"FAIL_CONFIG": "1"})
	if _, err := d.NamedVolumes(ctx, target, "db"); err == nil {
		t.Error("failed config command provided evidence")
	}
	target.Env = domain.BuildEnv(target.Env, map[string]string{"FAIL_VOLUMES": "1"})
	if exists, err := d.VolumeExists(ctx, target, "fixture_data"); err == nil || exists {
		t.Fatalf("failed volume inventory provided evidence: %v, %v", exists, err)
	}
}
