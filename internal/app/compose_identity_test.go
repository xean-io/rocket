package app

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func TestServicesForceComposeProjectIdentity(t *testing.T) {
	for _, kind := range []domain.ServiceKind{domain.KindRun, domain.KindTask} {
		for _, envName := range []string{"dev", "smoke"} {
			t.Run(string(kind)+"/"+envName, func(t *testing.T) {
				h := newHarness(t)
				p := &domain.Project{Name: "identity", Root: "/code/identity", DefaultEnv: "dev",
					Envs: map[string]domain.Environment{"dev": {Name: "dev"}, "smoke": {Name: "smoke"}},
					Services: map[string]domain.Service{"api": {Name: "api", Kind: kind, Run: "serve", Task: "api:dev",
						Env: map[string]string{"COMPOSE_PROJECT_NAME": "service-override"},
						Ports: []domain.PortSpec{{Name: "http", Default: 8080, Env: "COMPOSE_PROJECT_NAME", EnvBindings: map[string]domain.PortEnvBinding{
							"COMPOSE_PROJECT_NAME": {Template: "port-override-{port}"},
						}}}}},
				}
				h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
				h.app.d.BaseEnv = func() []string { return []string{"COMPOSE_PROJECT_NAME=inherited-override", "ROCKET_OWNER=agent:leak"} }
				h.app.d.Env = fakeEnv{values: map[string]string{"COMPOSE_PROJECT_NAME": "dotenv-override"}}
				res := h.up(t, UpRequest{Project: p.Root, Env: envName, Owner: "agent:job-owner"})
				if res.Failed() {
					t.Fatalf("up failed: %+v", res)
				}
				env := envMap(h.runner.specFor(t, p.Root).Env)
				if got, want := env["COMPOSE_PROJECT_NAME"], "rocket-identity-"+envName; got != want {
					t.Errorf("compose identity = %q, want %q", got, want)
				}
				if _, ok := env["ROCKET_OWNER"]; ok {
					t.Error("Rocket owner leaked into child environment")
				}
			})
		}
	}
}

func TestServicesForceComposeProjectIdentityAcrossOverrideLayers(t *testing.T) {
	for _, source := range []string{"unset", "inherited", "dotenv", "service"} {
		t.Run(source, func(t *testing.T) {
			h := newHarness(t)
			p := jobsProject()
			base := []string{"ROCKET_OWNER=agent:leak"}
			dotenv := map[string]string{}
			if source == "inherited" {
				base = append(base, "COMPOSE_PROJECT_NAME=foreign")
			}
			if source == "dotenv" {
				dotenv["COMPOSE_PROJECT_NAME"] = "foreign"
			}
			if source == "service" {
				api := p.Services["api"]
				api.Env = map[string]string{"COMPOSE_PROJECT_NAME": "foreign"}
				p.Services["api"] = api
			}
			h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
			h.app.d.BaseEnv = func() []string { return base }
			h.app.d.Env = fakeEnv{values: dotenv}
			h.up(t, UpRequest{Project: p.Root})
			if got := envMap(h.runner.specFor(t, p.Root).Env)["COMPOSE_PROJECT_NAME"]; got != "rocket-jobs-dev" {
				t.Fatalf("%s compose identity = %q", source, got)
			}
		})
	}
}

func TestJobsForceComposeProjectIdentityAndPersistEnvironment(t *testing.T) {
	for _, tt := range []struct {
		name      string
		kind      domain.JobKind
		jobName   string
		env       string
		wantEnv   string
		wantSteps int
	}{
		{"default pipeline", domain.JobPipeline, "ci", "", "dev", 3},
		{"selected pipeline", domain.JobPipeline, "ci", "stage", "stage", 3},
		{"default setup sequence", domain.JobSetup, "", "", "dev", 2},
		{"selected setup", domain.JobSetup, "doctor", "stage", "stage", 1},
		{"task fallback", domain.JobPipeline, "build", "prod", "prod", 1},
		{"deployment target", domain.JobDeploy, "prod", "", "prod", 1},
		{"matching deploy env", domain.JobDeploy, "prod", "prod", "prod", 1},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h := newHarness(t)
			h.app.d.BaseEnv = func() []string { return []string{"COMPOSE_PROJECT_NAME=inherited-override", "ROCKET_OWNER=agent:leak"} }
			h.app.d.Env = fakeEnv{values: map[string]string{"COMPOSE_PROJECT_NAME": "dotenv-override", "SHARED": "value"}}
			h.runner.exitNow[""] = 0
			req := JobRequest{Kind: tt.kind, Name: tt.jobName, Owner: "agent:owner", Yes: true, AllowAgentDeploy: true}
			if err := json.Unmarshal([]byte(`{"env":"`+tt.env+`"}`), &req); err != nil {
				t.Fatal(err)
			}
			job := h.startJob(t, req)
			if job.Env != tt.wantEnv {
				t.Errorf("returned env = %q, want %q", job.Env, tt.wantEnv)
			}
			finished := h.waitJob(t, job.ID)
			if finished.Env != tt.wantEnv || finished.Status != domain.JobSucceeded {
				t.Errorf("persisted job = %+v, want env %q and succeeded", finished, tt.wantEnv)
			}
			h.runner.mu.Lock()
			defer h.runner.mu.Unlock()
			if len(h.runner.started) != tt.wantSteps {
				t.Fatalf("started %d steps, want %d", len(h.runner.started), tt.wantSteps)
			}
			for i, spec := range h.runner.started {
				env := envMap(spec.Env)
				if got, want := env["COMPOSE_PROJECT_NAME"], "rocket-jobs-"+tt.wantEnv; got != want {
					t.Errorf("step %d compose identity = %q, want %q", i+1, got, want)
				}
				if _, ok := env["ROCKET_OWNER"]; ok {
					t.Errorf("step %d owner leaked", i+1)
				}
				if env["SHARED"] != "value" {
					t.Errorf("step %d lost dotenv value", i+1)
				}
			}
		})
	}
}

func TestJobsRejectInvalidEnvironmentBeforePersistence(t *testing.T) {
	for _, tt := range []struct {
		name    string
		kind    domain.JobKind
		jobName string
		env     string
	}{
		{"unknown pipeline env", domain.JobPipeline, "ci", "missing"},
		{"unknown setup env", domain.JobSetup, "doctor", "missing"},
		{"inconsistent deploy env", domain.JobDeploy, "prod", "stage"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			h := newHarness(t)
			req := JobRequest{Project: "/code/jobs", Kind: tt.kind, Name: tt.jobName, Yes: true}
			if err := json.Unmarshal([]byte(`{"env":"`+tt.env+`"}`), &req); err != nil {
				t.Fatal(err)
			}
			_, err := h.app.StartJob(context.Background(), req)
			if !errors.Is(err, ErrInvalid) || !strings.Contains(err.Error(), tt.env) {
				t.Errorf("error = %v, want invalid env %q", err, tt.env)
			}
			jobs, err := h.store.ListJobs("", 0)
			if err != nil || len(jobs) != 0 || len(h.argvs()) != 0 {
				t.Fatalf("invalid request had job side effects: jobs=%v, err=%v, argv=%v", jobs, err, h.argvs())
			}
		})
	}
}
