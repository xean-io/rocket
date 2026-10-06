//go:build integration && unix

package main

import (
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
)

func TestPortReferencesPropagateRealRemap(t *testing.T) {
	if testing.Short() {
		t.Skip("real daemon and subprocesses")
	}
	e := setup(t)
	data := []byte(`version: 1
name: rocket-fixture
services:
  static:
    run: "exec python3 -m http.server $PORT --bind 127.0.0.1"
    ports: {http: {default: 18431, env: {PORT: "{port}"}}}
    health: {http: /, timeout: 20s}
  consumer:
    run: 'echo provider_url=$API_URL; echo shell_literal=$SHELL_LITERAL; exec sleep 600'
    env:
      API_URL: "http://127.0.0.1:{static.http}"
      SHELL_LITERAL: "${static.http}"
`)
	if err := os.WriteFile(filepath.Join(e.project, "rocket.yaml"), data, 0o644); err != nil {
		t.Fatal(err)
	}
	holder, err := net.Listen("tcp", "127.0.0.1:18431")
	if err != nil {
		t.Fatal(err)
	}
	defer holder.Close()
	var up app.UpResult
	e.json(&up, nil, 0, "up", "consumer")
	if len(up.Services) != 2 || up.Services[0].Service != "static" || up.Services[1].Service != "consumer" {
		t.Fatalf("implicit dependency start order: %+v", up)
	}
	if up.Services[0].Ports["http"] != 18531 || !httpOK(18531) {
		t.Fatalf("provider did not start at remapped port: %+v", up.Services[0])
	}
	waitFor(t, 5*time.Second, "consumer resolved environment", func() bool {
		var logs app.LogsResult
		e.json(&logs, nil, 0, "logs", "consumer")
		lines := strings.Join(logs.Lines, "\n")
		return strings.Contains(lines, "provider_url=http://127.0.0.1:18531") && strings.Contains(lines, "shell_literal=${static.http}")
	})
	var down app.DownResult
	e.json(&down, nil, 0, "down", "--all")
	if len(down.Stopped) != 2 || down.Stopped[0].Service != "consumer" {
		t.Fatalf("reverse dependency shutdown: %+v", down)
	}
	for name, run := range e.ps() {
		if run.State != domain.StateStopped {
			t.Errorf("test-owned %s still active: %+v", name, run)
		}
	}
	var ports app.PortsResult
	e.json(&ports, nil, 0, "ports")
	if len(ports.Ports) != 0 {
		t.Fatalf("test leases remain: %+v", ports)
	}
}
