//go:build integration && unix

package task

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/adapters/process"
	"github.com/xean-io/rocket/internal/ports"
)

func TestTaskRunsUnderProcessRunner(t *testing.T) {
	if _, err := exec.LookPath("task"); err != nil {
		t.Skip("go-task not installed")
	}
	dir := t.TempDir()
	taskfile := "version: '3'\ntasks:\n  hello:\n    cmds:\n      - echo hello-$NAME\n"
	if err := os.WriteFile(filepath.Join(dir, "Taskfile.yml"), []byte(taskfile), 0o644); err != nil {
		t.Fatal(err)
	}
	out, err := os.Create(filepath.Join(dir, "out.log"))
	if err != nil {
		t.Fatal(err)
	}
	defer out.Close()
	h, err := process.Runner{}.Start(ports.ProcessSpec{
		Argv: Driver{}.Argv("hello"), Dir: dir, Env: append(os.Environ(), "NAME=rocket"), Output: out,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer process.Runner{}.Stop(context.Background(), h.PGID, time.Second)
	select {
	case code := <-h.Done:
		if code != 0 {
			t.Fatalf("exit %d", code)
		}
	case <-time.After(20 * time.Second):
		t.Fatal("timeout")
	}
	data, _ := os.ReadFile(out.Name())
	if !strings.Contains(string(data), "hello-rocket") {
		t.Fatalf("output %q", data)
	}
}
