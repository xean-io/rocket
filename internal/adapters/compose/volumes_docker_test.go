//go:build integration_docker

package compose

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

func TestComposeNamedVolumesFreshExistingAndExternal(t *testing.T) {
	if testing.Short() {
		t.Skip("Docker integration")
	}
	if err := exec.Command("docker", "info").Run(); err != nil {
		t.Skip("docker not available")
	}
	dir := t.TempDir()
	file := filepath.Join(dir, "compose.yaml")
	project := fmt.Sprintf("rocket-volume-itest-%x", time.Now().UnixNano())
	external := project + "_external"
	if out, err := exec.Command("docker", "volume", "create", "--label", "rocket.test.project="+project, external).CombinedOutput(); err != nil {
		t.Fatalf("create owned external fixture volume: %v\n%s", err, out)
	}
	spec := `name: ignored-manual-name
services:
  box:
    image: busybox
    profiles: [fixture]
    command: ['sh', '-c', 'sleep 300']
    healthcheck: {test: ['CMD', 'true'], interval: 1s}
    volumes:
      - data:/data
      - custom:/custom
      - external:/external
      - ./bind:/bind
volumes:
  data: {}
  custom: {name: "${COMPOSE_PROJECT_NAME}_${VOLUME_SUFFIX}"}
  unused: {}
  external: {external: true, name: "` + external + `"}
`
	if err := os.Mkdir(filepath.Join(dir, "bind"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(file, []byte(spec), 0o644); err != nil {
		t.Fatal(err)
	}
	target := ports.ComposeTarget{ProjectName: project, Dir: dir, Files: []string{file}, Profiles: []string{"fixture"},
		Env: domain.BuildEnv(os.Environ(), map[string]string{"COMPOSE_PROJECT_NAME": project, "VOLUME_SUFFIX": "custom"})}
	d := Driver{}
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	t.Cleanup(func() {
		cleanup, done := context.WithTimeout(context.Background(), 30*time.Second)
		defer done()
		if err := d.Down(cleanup, target, os.Stderr); err != nil {
			t.Errorf("cleanup owned project: %v", err)
		}
		assertNoOwnedContainers(t, project)
		out, err := exec.Command("docker", "volume", "ls", "--format", "{{.Name}}", "--filter", "label=com.docker.compose.project="+project).Output()
		if err != nil {
			t.Errorf("list retained fixture volumes: %v", err)
		}
		retained := append(strings.Fields(string(out)), external)
		slices.Sort(retained)
		t.Logf("retained fixture volumes (never deleted): %v", retained)
	})
	names, err := d.NamedVolumes(ctx, target, "box")
	want := []string{project + "_custom", project + "_data"}
	if err != nil || !slices.Equal(names, want) {
		t.Fatalf("service-mounted nonexternal volumes = %v, %v; want %v", names, err, want)
	}
	for _, name := range names {
		if exists, err := d.VolumeExists(ctx, target, name); err != nil || exists {
			t.Fatalf("fresh volume %s exists=%v, error=%v", name, exists, err)
		}
	}
	if exists, err := d.VolumeExists(ctx, target, external); err != nil || !exists {
		t.Fatalf("owned external fixture volume exists=%v, error=%v", exists, err)
	}
	if cid, err := d.Up(ctx, target, "box", io.Discard); err != nil || cid == "" {
		t.Fatalf("fresh up = %q, %v", cid, err)
	}
	for _, name := range names {
		if exists, err := d.VolumeExists(ctx, target, name); err != nil || !exists {
			t.Fatalf("new volume %s not confirmed after Up: %v, %v", name, exists, err)
		}
	}
	if err := d.Down(ctx, target, io.Discard); err != nil {
		t.Fatal(err)
	}
	assertNoOwnedContainers(t, project)
	for _, name := range names {
		if exists, err := d.VolumeExists(ctx, target, name); err != nil || !exists {
			t.Fatalf("Down failed to retain %s: %v, %v", name, exists, err)
		}
	}
	if cid, err := d.Up(ctx, target, "box", io.Discard); err != nil || cid == "" {
		t.Fatalf("existing-volume up = %q, %v", cid, err)
	}
	if err := d.Down(ctx, target, io.Discard); err != nil {
		t.Fatal(err)
	}
	assertNoOwnedContainers(t, project)
}
