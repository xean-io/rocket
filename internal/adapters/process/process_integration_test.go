//go:build integration && unix

package process

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/ports"
)

func start(t *testing.T, script string) (ports.ProcessHandle, string) {
	t.Helper()
	logPath := filepath.Join(t.TempDir(), "out.log")
	f, err := os.Create(logPath)
	if err != nil {
		t.Fatal(err)
	}
	defer f.Close()
	h, err := Runner{}.Start(ports.ProcessSpec{Argv: []string{"/bin/sh", "-c", script}, Dir: t.TempDir(), Env: []string{"GREETING=hi", "PATH=/usr/bin:/bin"}, Output: f})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = Runner{}.Stop(context.Background(), h.PGID, 100*time.Millisecond) })
	return h, logPath
}

func TestExitCodeAndOutput(t *testing.T) {
	h, logPath := start(t, "echo $GREETING; exit 3")
	select {
	case code := <-h.Done:
		if code != 3 {
			t.Fatalf("code %d", code)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("timeout")
	}
	data, _ := os.ReadFile(logPath)
	if strings.TrimSpace(string(data)) != "hi" {
		t.Fatalf("output %q", data)
	}
}

func TestStopKillsWholeGroup(t *testing.T) {
	h, _ := start(t, "sleep 30 & sleep 30 & wait")
	if h.PGID != h.PID || !(Runner{}).Alive(h.PID, h.PGID) {
		t.Fatal("process should be alive and lead its group")
	}
	if err := (Runner{}).Stop(context.Background(), h.PGID, 2*time.Second); err != nil {
		t.Fatal(err)
	}
	if err := syscall.Kill(-h.PGID, 0); err == nil {
		t.Fatal("group still has members")
	}
	if (Runner{}).Alive(h.PID, h.PGID) {
		t.Fatal("leader still alive")
	}
}

func TestStopEscalatesToSIGKILL(t *testing.T) {
	h, _ := start(t, "trap '' TERM; while true; do sleep 0.1; done")
	time.Sleep(200 * time.Millisecond) // let the trap install
	begin := time.Now()
	if err := (Runner{}).Stop(context.Background(), h.PGID, 300*time.Millisecond); err != nil {
		t.Fatal(err)
	}
	if time.Since(begin) < 300*time.Millisecond {
		t.Fatal("SIGKILL sent before grace elapsed")
	}
	select {
	case code := <-h.Done:
		if code != 128+int(syscall.SIGKILL) {
			t.Fatalf("code %d", code)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("no exit")
	}
}

func TestStopRefusesOwnGroup(t *testing.T) {
	if err := (Runner{}).Stop(context.Background(), syscall.Getpgrp(), time.Millisecond); err == nil {
		t.Fatal("must refuse to signal own process group")
	}
}
