//go:build integration_docker

package compose

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/ports"
)

func TestComposeLifecycle(t *testing.T) {
	if testing.Short() {
		t.Skip("Docker integration")
	}
	if err := exec.Command("docker", "info").Run(); err != nil {
		t.Skip("docker not available")
	}
	dir := t.TempDir()
	file := filepath.Join(dir, "compose.yaml")
	spec := "services:\n  box:\n    image: busybox\n    command: ['sh', '-c', 'sleep 300']\n    healthcheck:\n      test: ['CMD', 'true']\n      interval: 1s\n"
	if err := os.WriteFile(file, []byte(spec), 0o644); err != nil {
		t.Fatal(err)
	}
	d := Driver{}
	target := ports.ComposeTarget{ProjectName: fmt.Sprintf("rocket-itest-%x", time.Now().UnixNano()), Dir: dir, Files: []string{file}, Env: os.Environ()}
	ctx := context.Background()
	t.Cleanup(func() {
		if err := d.Down(ctx, target, os.Stderr); err != nil {
			t.Errorf("cleanup: %v", err)
		}
		assertNoOwnedContainers(t, target.ProjectName)
	})

	cid, err := d.Up(ctx, target, "box", os.Stderr)
	if err != nil || cid == "" {
		t.Fatalf("up: cid=%q err=%v", cid, err)
	}
	if ok, err := d.Running(ctx, target.ProjectName, "box"); err != nil || !ok {
		t.Fatalf("running: %v %v", ok, err)
	}
	if err := d.Stop(ctx, target, "box", os.Stderr); err != nil {
		t.Fatal(err)
	}
	if ok, _ := d.Running(ctx, target.ProjectName, "box"); ok {
		t.Fatal("still running after stop")
	}
}

func assertNoOwnedContainers(t *testing.T, project string) {
	t.Helper()
	out, err := exec.Command("docker", "ps", "-aq", "--filter", "label=com.docker.compose.project="+project).Output()
	if err != nil || len(out) != 0 {
		t.Errorf("owned containers remain for %s: %q, %v", project, out, err)
	}
}
