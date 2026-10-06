//go:build integration && unix

package main

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/xean-io/rocket/internal/adapters/probe"
	"github.com/xean-io/rocket/internal/app"
)

func TestCLIProfilesAndStopAll(t *testing.T) {
	if testing.Short() {
		t.Skip("real daemon and subprocesses")
	}
	e := setup(t)
	manifest := []byte(`version: 1
name: rocket-fixture
services:
  static:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    ports: {http: {default: 18431, env: PORT}}
    health: {http: /, timeout: 20s}
  trends:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    profiles: [trends]
    ports: {http: {default: 18432, env: PORT}}
    health: {http: /, timeout: 20s}
  reports:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    profiles: [reports]
    ports: {http: {default: 18433, env: PORT}}
    health: {http: /, timeout: 20s}
groups: {all: ["*"]}
`)
	if err := os.WriteFile(filepath.Join(e.project, "rocket.yaml"), manifest, 0o644); err != nil {
		t.Fatal(err)
	}
	var up app.UpResult
	e.json(&up, nil, 0, "up", "all", "--profile", "trends", "--profile", "reports", "--profile", "trends")
	if len(up.Services) != 3 || up.Failed() {
		t.Fatalf("repeated profiles startup = %+v", up)
	}
	reports := e.ps()["reports"]
	var restart app.UpResult
	e.json(&restart, nil, 0, "restart", "all", "--profile", "trends", "--profile", "trends")
	if len(restart.Services) != 2 || restart.Failed() {
		t.Fatalf("profile restart = %+v", restart)
	}
	if after := e.ps()["reports"]; after.PID != reports.PID || !after.State.Active() {
		t.Fatalf("restart touched unselected reports: %+v", after)
	}
	var down app.DownResult
	e.json(&down, nil, 0, "down", "--all")
	if len(down.Stopped) != 3 {
		t.Fatalf("down --all filtered active profiles: %+v", down)
	}
	for _, run := range down.Stopped {
		if !groupGone(run.PGID) {
			t.Errorf("test-owned process group %d remains", run.PGID)
		}
	}
	for _, port := range []int{18431, 18432, 18433} {
		if !(probe.Ports{}).Free(port) {
			t.Errorf("test-owned listener %d remains", port)
		}
	}
	e.json(&up, nil, 0, "up", "all")
	if len(up.Services) != 1 || up.Services[0].Service != "static" {
		t.Fatalf("default wildcard = %+v", up)
	}
	e.json(&up, nil, 0, "up", "*", "--profile", "reports")
	if len(up.Services) != 2 || up.Failed() {
		t.Fatalf("requested wildcard profile = %+v", up)
	}
	e.json(&down, nil, 0, "down", "--everywhere")
	if len(down.Stopped) != 2 {
		t.Fatalf("down --everywhere filtered active profiles: %+v", down)
	}
	var ports app.PortsResult
	e.json(&ports, nil, 0, "ports")
	if len(ports.Ports) != 0 {
		t.Fatalf("test leases remain: %+v", ports)
	}
}
