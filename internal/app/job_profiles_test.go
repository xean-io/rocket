package app

import (
	"context"
	"encoding/json"
	"errors"
	"slices"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func jobProfiles(t *testing.T, job domain.Job) []string {
	t.Helper()
	data, _ := json.Marshal(job)
	var wire struct {
		Profiles []string `json:"profiles"`
	}
	if err := json.Unmarshal(data, &wire); err != nil {
		t.Fatal(err)
	}
	return wire.Profiles
}

func TestJobProfilesSelectWildcardPrerequisitesAndPersist(t *testing.T) {
	for _, tc := range []struct {
		name, body string
		profiles   []string
		services   []string
	}{
		{"default env", `{}`, []string{"base"}, []string{"api", "ordinary", "required"}},
		{"requested deduplicated", `{"profiles":["reports","base","reports","extra"]}`, []string{"base", "extra", "reports"}, []string{"api", "ordinary", "reporting", "required"}},
		{"selected env", `{"env":"smoke","profiles":["extra"]}`, []string{"extra", "trends"}, []string{"api", "ordinary", "reporting", "required", "trends"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			h, p := profileHarness(t)
			p.Pipelines = map[string][]domain.Step{"check": {{Run: "job-step"}}}
			p.PipelineNeeds = map[string][]string{"check": {"all"}}
			h.runner.exitNow["job-step"] = 0
			var req JobRequest
			if err := json.Unmarshal([]byte(tc.body), &req); err != nil {
				t.Fatal(err)
			}
			req.Project, req.Kind, req.Name = p.Root, domain.JobPipeline, "check"
			started := h.startJob(t, req)
			finished := h.waitJob(t, started.ID)
			if finished.Status != domain.JobSucceeded || !slices.Equal(jobProfiles(t, started), tc.profiles) || !slices.Equal(jobProfiles(t, finished), tc.profiles) {
				t.Errorf("effective profiles not persisted: start=%+v, finish=%+v, profiles=%v", started, finished, jobProfiles(t, finished))
			}
			runs, _ := h.store.ListRuns()
			var services []string
			for _, run := range runs {
				services = append(services, run.Service)
				if run.Env != finished.Env {
					t.Errorf("prerequisite env %s != job env %s", run.Env, finished.Env)
				}
			}
			slices.Sort(services)
			if !slices.Equal(services, tc.services) {
				t.Fatalf("wildcard prerequisites = %v, want %v", services, tc.services)
			}
			if slices.Contains(tc.services, "reporting") {
				want := domain.MergeProfiles(tc.profiles, p.Services["reporting"].Profiles)
				if got := h.run(t, p.Name, "reporting").Profiles; !slices.Equal(got, want) {
					t.Fatalf("Compose prerequisite profiles = %v, want %v", got, want)
				}
			}
		})
	}
}

func TestDeployRejectsRequestedProfilesBeforePersistence(t *testing.T) {
	h := newHarness(t)
	var req JobRequest
	if err := json.Unmarshal([]byte(`{"project":"/code/jobs","kind":"deploy","name":"prod","yes":true,"profiles":["extra"]}`), &req); err != nil {
		t.Fatal(err)
	}
	_, err := h.app.StartJob(context.Background(), req)
	jobs, _ := h.store.ListJobs("", 0)
	if !errors.Is(err, ErrInvalid) || len(jobs) != 0 || len(h.argvs()) != 0 {
		t.Fatalf("unsupported deployment profiles accepted: error=%v, jobs=%v", err, jobs)
	}
}
