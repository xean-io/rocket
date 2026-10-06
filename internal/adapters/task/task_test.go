package task

import (
	"os"
	"path/filepath"
	"slices"
	"testing"
)

func TestArgv(t *testing.T) {
	if got := (Driver{}).Argv("dev:causation"); !slices.Equal(got, []string{"task", "dev:causation"}) {
		t.Fatalf("got %v", got)
	}
	if got := (Driver{Bin: "/opt/task"}).Argv("test", "-run", "X"); !slices.Equal(got, []string{"/opt/task", "test", "--", "-run", "X"}) {
		t.Fatalf("got %v", got)
	}
}

func TestTaskfileDetection(t *testing.T) {
	dir := t.TempDir()
	if got := (Driver{}).Taskfile(dir); got != "" {
		t.Fatalf("empty dir: %q", got)
	}
	if err := os.WriteFile(filepath.Join(dir, "Taskfile.yaml"), []byte("version: '3'\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := (Driver{}).Taskfile(dir); got != filepath.Join(dir, "Taskfile.yaml") {
		t.Fatalf("got %q", got)
	}
}
