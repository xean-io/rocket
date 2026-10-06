//go:build integration && unix

package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
)

func TestPipelineNeedsStartAndCancelRealPrerequisites(t *testing.T) {
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
  waiting:
    run: "exec sleep 600"
    ports: {http: {default: 18432, env: PORT}}
    health: {tcp: true, timeout: 20s}
groups: {infra: [static]}
pipelines:
  check: {needs: [infra], steps: [{run: "echo step-ready compose=$COMPOSE_PROJECT_NAME"}]}
  blocked: {needs: [waiting], steps: [{run: "echo never-step"}]}
`)
	if err := os.WriteFile(filepath.Join(e.project, "rocket.yaml"), manifest, 0o644); err != nil {
		t.Fatal(err)
	}
	var outcome app.JobOutcome
	e.json(&outcome, []string{"ROCKET_OWNER=agent:check"}, 0, "run", "check")
	if outcome.Status != domain.JobSucceeded || !httpOK(18431) || !strings.Contains(strings.Join(outcome.Tail, "\n"), "step-ready compose=rocket-rocket-fixture-dev") {
		t.Fatalf("pipeline prerequisite outcome: %+v", outcome)
	}
	if run := e.ps()["static"]; run.Owner != "agent:check" || run.State != domain.StateRunning {
		t.Fatalf("prerequisite did not remain running with job owner: %+v", run)
	}
	var job domain.Job
	e.json(&job, nil, 0, "run", "blocked", "--detach")
	var waiting domain.Run
	waitFor(t, 5*time.Second, "blocked prerequisite startup", func() bool {
		waiting = e.ps()["waiting"]
		return waiting.State == domain.StateStarting && waiting.PGID > 0
	})
	var canceled domain.Job
	e.json(&canceled, nil, 0, "job", "cancel", job.ID)
	if canceled.Status != domain.JobCanceled || canceled.Step != 0 || !groupGone(waiting.PGID) {
		t.Fatalf("prerequisite cancellation failed: %+v, prerequisite=%+v", canceled, waiting)
	}
	var logs app.JobLogsResult
	e.json(&logs, nil, 0, "job", job.ID, "logs")
	if strings.Contains(strings.Join(logs.Lines, "\n"), "never-step") {
		t.Fatalf("pipeline steps ran despite canceled prerequisite: %v", logs.Lines)
	}
	if !httpOK(18431) {
		t.Fatal("canceling another job stopped the ready prerequisite")
	}
	var down app.DownResult
	e.json(&down, nil, 0, "down", "--all")
	var ports app.PortsResult
	e.json(&ports, nil, 0, "ports")
	if len(ports.Ports) != 0 {
		t.Fatalf("test-owned leases remain: %+v", ports)
	}
}
