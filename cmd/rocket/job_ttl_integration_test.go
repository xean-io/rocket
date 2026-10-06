//go:build integration && unix

package main

import (
	"bufio"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/daemon"
	"github.com/xean-io/rocket/internal/domain"
)

func (e *env) waitExpiredJob(id string) domain.Job {
	e.t.Helper()
	var job domain.Job
	waitFor(e.t, 8*time.Second, "job TTL expiry", func() bool {
		e.json(&job, nil, 0, "job", id)
		return job.Status.Terminal()
	})
	if job.Status != domain.JobCanceled || job.Error != "ttl expired" || job.ExpiresAt == nil || job.FinishedAt == nil {
		e.t.Fatalf("expired job: %+v", job)
	}
	return job
}

func TestJobTTLRealStepsStartupAndDaemonRestart(t *testing.T) {
	if testing.Short() {
		t.Skip("real daemon and subprocesses")
	}
	e := setup(t)
	manifest := []byte(`version: 1
name: rocket-fixture
services:
  waiting:
    run: "exec sleep 600"
    ports: {http: {default: 18432, env: PORT}}
    health: {tcp: true, timeout: 20s}
envs:
  dev: {}
  stage: {deploy: {run: "echo harmless-deploy", confirm: true}}
setup: {doctor: {run: "echo doctor-ok"}}
pipelines:
  slow: [{run: "exec sleep 600"}]
  trailing:
    - run: >-
        exec python3 -c 'import signal,sys,time; signal.signal(signal.SIGTERM, lambda *_:(sys.stdout.write("trailing-partial"),sys.stdout.flush(),sys.exit(0))); print("ready", flush=True); time.sleep(600)'
  blocked: {needs: [waiting], steps: [{run: "echo never-step"}]}
`)
	if err := os.WriteFile(filepath.Join(e.project, "rocket.yaml"), manifest, 0o644); err != nil {
		t.Fatal(err)
	}

	t.Run("step expiry drains partial output before one terminal event", func(t *testing.T) {
		var started domain.Job
		e.json(&started, nil, 0, "run", "trailing", "--ttl", "1s", "--detach")
		running := e.waitJobPGID(started.ID)
		info, err := daemon.ReadInfo(filepath.Join(e.home, "daemon.json"))
		if err != nil {
			t.Fatal(err)
		}
		req, _ := http.NewRequest(http.MethodGet, info.HTTP+"/v1/jobs/"+started.ID+"/logs?follow=true&tail=-1", nil)
		req.Header.Set("Authorization", "Bearer "+info.Token)
		client := http.Client{Timeout: 8 * time.Second}
		response, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		if response.StatusCode != http.StatusOK {
			t.Fatalf("follow HTTP status: %d", response.StatusCode)
		}
		var partial, terminal int
		scanner := bufio.NewScanner(response.Body)
		for scanner.Scan() {
			line := scanner.Text()
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			var event domain.Event
			if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &event); err != nil {
				t.Fatalf("interleaved SSE frame: %q, %v", line, err)
			}
			if terminal > 0 {
				t.Fatalf("event after terminal: %+v", event)
			}
			if event.Type == domain.EventJobLog && event.Line == "trailing-partial" {
				partial++
			}
			if event.Type == domain.EventJobState && event.Status.Terminal() {
				terminal++
				if partial != 1 || event.Status != domain.JobCanceled || event.Job == nil || event.Job.Error != "ttl expired" {
					t.Fatalf("terminal preceded trailing output or lost expiry reason: %+v, partial=%d", event, partial)
				}
			}
		}
		if err := scanner.Err(); err != nil || partial != 1 || terminal != 1 {
			t.Fatalf("stream result: partial=%d, terminal=%d, error=%v", partial, terminal, err)
		}
		e.waitExpiredJob(started.ID)
		if !groupGone(running.PGID) {
			t.Fatalf("expired step group %d remains", running.PGID)
		}
	})

	t.Run("startup expiry shares deadline and releases attempted listener lease", func(t *testing.T) {
		var job domain.Job
		e.json(&job, nil, 0, "run", "blocked", "--ttl", "1s", "--detach")
		var waiting domain.Run
		waitFor(t, 3*time.Second, "prerequisite startup", func() bool {
			waiting = e.ps()["waiting"]
			return waiting.State == domain.StateStarting && waiting.PGID > 0
		})
		if job.ExpiresAt == nil || waiting.ExpiresAt == nil || !waiting.ExpiresAt.Equal(*job.ExpiresAt) {
			t.Fatalf("prerequisite deadline differs: job=%+v, run=%+v", job, waiting)
		}
		finished := e.waitExpiredJob(job.ID)
		if finished.Step != 0 || !groupGone(waiting.PGID) {
			t.Fatalf("startup expiry leaked process or ran steps: %+v, run=%+v", finished, waiting)
		}
		var logs app.JobLogsResult
		e.json(&logs, nil, 0, "job", job.ID, "logs")
		if tailHas(logs.Lines, "never-step") {
			t.Fatalf("steps ran after failed startup: %v", logs.Lines)
		}
		var ports app.PortsResult
		e.json(&ports, nil, 0, "ports")
		if len(ports.Ports) != 0 {
			t.Fatalf("expired startup retained lease: %+v", ports)
		}
	})

	t.Run("restart cancels expired no PID job and retains adopted deadline", func(t *testing.T) {
		var startup, step domain.Job
		e.json(&startup, nil, 0, "run", "blocked", "--ttl", "1s", "--detach")
		e.json(&step, nil, 0, "run", "slow", "--ttl", "4s", "--detach")
		running := e.waitJobPGID(step.ID)
		if _, code := e.rocket(nil, "daemon", "stop"); code != 0 {
			t.Fatalf("stop test daemon: exit %d", code)
		}
		if groupGone(running.PGID) {
			t.Fatal("daemon shutdown stopped retained job")
		}
		waitFor(t, 3*time.Second, "persisted startup deadline", func() bool { return time.Now().After(*startup.ExpiresAt) })
		var status DaemonStatus
		e.json(&status, nil, 0, "daemon", "start")
		finished := e.waitExpiredJob(startup.ID)
		if finished.Step != 0 {
			t.Fatalf("expired no PID job ran a step: %+v", finished)
		}
		var adopted domain.Job
		e.json(&adopted, nil, 0, "job", step.ID)
		if adopted.Status != domain.JobRunning || adopted.ExpiresAt == nil || !adopted.ExpiresAt.Equal(*step.ExpiresAt) {
			t.Fatalf("restart renewed deadline or failed adoption: before=%+v, after=%+v", step, adopted)
		}
		e.waitExpiredJob(step.ID)
		if !groupGone(running.PGID) {
			t.Fatal("adopted expired job group remains")
		}
	})

	t.Run("blocking run and shared job flags preserve exit codes", func(t *testing.T) {
		var outcome app.JobOutcome
		e.json(&outcome, nil, 2, "run", "slow", "--ttl", "100ms")
		if outcome.Status != domain.JobCanceled || outcome.Error != "ttl expired" {
			t.Fatalf("blocking expiry outcome: %+v", outcome)
		}
		for _, args := range [][]string{{"doctor", "--ttl", "1s"}, {"deploy", "stage", "--yes", "--ttl", "1s"}} {
			e.json(&outcome, nil, 0, args...)
			var job domain.Job
			e.json(&job, nil, 0, "job", outcome.Job)
			if job.ExpiresAt == nil || job.ExpiresAt.Sub(job.StartedAt) != time.Second {
				t.Fatalf("shared TTL flag not persisted for %v: %+v", args, job)
			}
		}
		var body map[string]string
		e.json(&body, nil, 1, "run", "slow", "--ttl", "0s")
		if body["code"] != "invalid" {
			t.Fatalf("invalid TTL error shape changed: %v", body)
		}
	})
}
