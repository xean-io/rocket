//go:build integration && !windows

package compose

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/ports"
)

func TestUpRetainsComposeWait(t *testing.T) {
	if testing.Short() {
		t.Skip("command integration")
	}
	dir := t.TempDir()
	trace := filepath.Join(dir, "argv")
	bin := filepath.Join(dir, "docker-fixture")
	if err := os.WriteFile(bin, []byte(`#!/bin/sh
printf '%s\n' "$@" >> "$TRACE_FILE"
printf '%s\n' --next-- >> "$TRACE_FILE"
case "$4" in ps) printf '%s\n' fixture-container ;; esac
`), 0o755); err != nil {
		t.Fatal(err)
	}
	target := ports.ComposeTarget{ProjectName: "rocket-probe-fixture-dev", Dir: dir, Env: []string{"TRACE_FILE=" + trace}}
	cid, err := (Driver{Bin: bin}).Up(context.Background(), target, "backend", io.Discard)
	if err != nil || cid != "fixture-container" {
		t.Fatalf("compose up = %q, %v", cid, err)
	}
	got, err := os.ReadFile(trace)
	if err != nil {
		t.Fatal(err)
	}
	want := strings.Join([]string{"compose", "-p", target.ProjectName, "up", "-d", "--wait", "backend", "--next--", "compose", "-p", target.ProjectName, "ps", "-q", "backend", "--next--", ""}, "\n")
	if string(got) != want {
		t.Fatalf("compose command trace = %q, want %q", got, want)
	}
}
