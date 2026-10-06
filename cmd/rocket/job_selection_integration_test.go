//go:build integration && unix

package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
)

func TestJobSelectionMetadataAndDownGuidance(t *testing.T) {
	if testing.Short() {
		t.Skip("real daemon and subprocesses")
	}
	e := setup(t)
	data := []byte(`version: 1
name: rocket-fixture
default_env: dev
envs:
  stage: {deploy: {run: "echo harmless-deploy-$COMPOSE_PROJECT_NAME", confirm: true}}
  smoke: {profiles: [smoke]}
  dev: {profiles: [base]}
services:
  core: {run: "exec sleep 600", depends_on: [dependency]}
  dependency: {run: "exec sleep 600", profiles: [internal]}
  trends: {run: "exec sleep 600", profiles: [trends]}
  smoke: {run: "exec sleep 600", profiles: [smoke]}
groups: {all: ["*"]}
setup:
  doctor: {run: "echo doctor-$COMPOSE_PROJECT_NAME"}
  install: {run: "echo install-$COMPOSE_PROJECT_NAME"}
  migrate: {run: "echo migrate-$COMPOSE_PROJECT_NAME"}
pipelines:
  selected: {needs: [all], steps: [{run: "echo pipeline-$COMPOSE_PROJECT_NAME"}]}
  alpha: [{run: "echo alpha"}]
`)
	if err := os.WriteFile(filepath.Join(e.project, "rocket.yaml"), data, 0o644); err != nil {
		t.Fatal(err)
	}
	t.Run("status declares sorted available choices", func(t *testing.T) {
		e := *e
		e.t = t
		var wire struct {
			Project struct {
				Envs       []string `json:"envs"`
				Pipelines  []string `json:"pipelines"`
				DeployEnvs []string `json:"deploy_envs"`
			} `json:"project"`
		}
		e.json(&wire, nil, 0, "status")
		if !slices.Equal(wire.Project.Envs, []string{"dev", "smoke", "stage"}) || !slices.Equal(wire.Project.Pipelines, []string{"alpha", "selected"}) || !slices.Equal(wire.Project.DeployEnvs, []string{"stage"}) {
			t.Fatalf("declared choices = %+v", wire.Project)
		}
	})
	t.Run("pipeline env and repeatable profile flags select prerequisites", func(t *testing.T) {
		e := *e
		e.t = t
		var outcome app.JobOutcome
		e.json(&outcome, nil, 0, "run", "selected", "--env", "smoke", "--profile", "trends", "--profile", "extra", "--profile", "extra")
		if outcome.Status != domain.JobSucceeded || !tailHas(outcome.Tail, "pipeline-rocket-rocket-fixture-smoke") {
			t.Fatalf("selected job outcome = %+v", outcome)
		}
		var wire struct {
			Env      string   `json:"env"`
			Profiles []string `json:"profiles"`
		}
		e.json(&wire, nil, 0, "job", outcome.Job)
		if wire.Env != "smoke" || !slices.Equal(wire.Profiles, []string{"extra", "smoke", "trends"}) {
			t.Fatalf("persisted job selection = %+v", wire)
		}
		for name, run := range e.ps() {
			if run.State != domain.StateRunning || run.Env != "smoke" || run.Owner != "user" {
				t.Errorf("selected prerequisite %s = %+v", name, run)
			}
		}
		var down app.DownResult
		e.json(&down, nil, 0, "down", "--all")
		if len(down.Stopped) != 4 {
			t.Fatalf("stop-all omitted profile prerequisite: %+v", down)
		}
	})
	t.Run("setup commands share environment and profile flags", func(t *testing.T) {
		e := *e
		e.t = t
		for _, args := range [][]string{{"setup", "doctor"}, {"doctor"}, {"install"}, {"migrate"}} {
			var outcome app.JobOutcome
			e.json(&outcome, nil, 0, append(args, "--env", "smoke", "--profile", "extra", "--profile", "extra")...)
			if !tailHas(outcome.Tail, "rocket-rocket-fixture-smoke") {
				t.Errorf("setup environment %v: %+v", args, outcome)
			}
			var wire struct {
				Profiles []string `json:"profiles"`
			}
			e.json(&wire, nil, 0, "job", outcome.Job)
			if !slices.Equal(wire.Profiles, []string{"extra", "smoke"}) {
				t.Errorf("setup persisted profiles %v: %v", args, wire.Profiles)
			}
		}
	})
	t.Run("harmless deploy retains confirmation gate", func(t *testing.T) {
		e := *e
		e.t = t
		var body map[string]string
		e.json(&body, nil, 3, "deploy", "stage")
		if body["code"] != "confirmation_required" {
			t.Fatalf("deploy confirmation = %v", body)
		}
		var outcome app.JobOutcome
		e.json(&outcome, nil, 0, "deploy", "stage", "--yes")
		if !tailHas(outcome.Tail, "harmless-deploy-rocket-rocket-fixture-stage") {
			t.Fatalf("harmless deploy outcome = %+v", outcome)
		}
		var before, after app.JobsResult
		e.json(&before, nil, 0, "jobs")
		for _, flag := range []string{"--profile", "--env"} {
			if _, code := e.rocket(nil, "deploy", "stage", "--yes", flag, "extra"); code != 1 {
				t.Fatalf("unsupported deploy flag %s accepted: exit=%d", flag, code)
			}
		}
		e.json(&after, nil, 0, "jobs")
		if len(after.Jobs) != len(before.Jobs) {
			t.Fatalf("unsupported deploy flag submitted a job: before=%d, after=%d", len(before.Jobs), len(after.Jobs))
		}
	})
	t.Run("outside-project JSON guidance preserves error and exit", func(t *testing.T) {
		e := *e
		e.t = t
		for _, explicit := range []bool{false, true} {
			args := []string{"down", "--all", "--json"}
			if explicit {
				args = append(args, "-p", filepath.Join(e.home, "missing"))
			}
			out, code := e.rocketAt(e.home, nil, args...)
			var body map[string]string
			if err := json.Unmarshal([]byte(out), &body); err != nil || code != 1 || len(body) != 2 || body["code"] != "error" || !strings.Contains(body["error"], "no rocket.yaml found") || strings.Contains(body["error"], "--everywhere") == explicit {
				t.Errorf("outside guidance explicit=%v: exit=%d, body=%v, error=%v", explicit, code, body, err)
			}
		}
		bad := filepath.Join(e.home, "bad")
		if err := os.MkdirAll(bad, 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(bad, "rocket.yaml"), []byte("version: 99\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		out, code := e.rocketAt(e.home, nil, "down", "--all", "-p", bad, "--json")
		var body map[string]string
		if err := json.Unmarshal([]byte(out), &body); err != nil || code != 1 || len(body) != 2 || body["code"] != "invalid" || strings.Contains(body["error"], "--everywhere") {
			t.Errorf("malformed project guidance: exit=%d, body=%v, error=%v", code, body, err)
		}
	})
}
