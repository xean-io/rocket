package app

import (
	"context"
	"errors"
	"slices"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
)

func jobsProject() *domain.Project {
	return &domain.Project{
		Name: "jobs", Root: "/code/jobs", DefaultEnv: "dev", Dotenv: []string{".env"},
		Envs: map[string]domain.Environment{
			"dev":   {Name: "dev"},
			"stage": {Name: "stage", Deploy: &domain.Deploy{Run: "deploy-stage"}},
			"prod":  {Name: "prod", Deploy: &domain.Deploy{Task: "release:prod", Confirm: true}},
		},
		Setup: map[string]domain.Step{"doctor": {Run: "doctor-ok"}, "migrate": {Task: "db:migrate"}, "seed": {Run: "seed-it"}},
		Services: map[string]domain.Service{
			"api": {Name: "api", Kind: domain.KindRun, Run: "api-serve", Ports: []domain.PortSpec{{Name: "http", Default: 4000, Env: "PORT"}}},
		},
		Pipelines: map[string][]domain.Step{
			"ci":     {{Run: "step-lint"}, {Run: "step-unit"}, {Task: "e2e"}},
			"broken": {{Run: "step-ok"}, {Run: "step-boom"}, {Run: "step-never"}},
			"slow":   {{Run: "step-slow"}},
		},
	}
}

func (h *harness) startJob(t *testing.T, req JobRequest) domain.Job {
	t.Helper()
	if req.Project == "" {
		req.Project = "/code/jobs"
	}
	job, err := h.app.StartJob(context.Background(), req)
	if err != nil {
		t.Fatalf("start job: %v", err)
	}
	if job.ID == "" || job.Status != domain.JobRunning {
		t.Fatalf("started job %+v", job)
	}
	return job
}

func (h *harness) waitJob(t *testing.T, id string) domain.Job {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	j, err := h.app.WaitJob(ctx, id)
	if err != nil {
		t.Fatalf("wait job %s: %v", id, err)
	}
	return j
}

func (h *harness) waitJobPGID(t *testing.T, id string) domain.Job {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if j, err := h.app.GetJob(id); err == nil && j.PGID > 0 {
			return j
		}
		time.Sleep(2 * time.Millisecond)
	}
	t.Fatalf("job %s never started a process", id)
	return domain.Job{}
}

func (h *harness) argvs() [][]string {
	h.runner.mu.Lock()
	defer h.runner.mu.Unlock()
	var out [][]string
	for _, s := range h.runner.started {
		out = append(out, s.Argv)
	}
	return out
}

func TestRunPipelineRunsStepsInOrderWithProjectEnv(t *testing.T) {
	h := newHarness(t)
	h.runner.exitNow["step-lint"] = 0
	h.runner.exitNow["step-unit"] = 0
	h.runner.exitNow["task e2e"] = 0

	job := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "ci", Args: []string{"-run", "X"}, Owner: "agent:a"})
	got := h.waitJob(t, job.ID)

	if got.Status != domain.JobSucceeded || got.ExitCode == nil || *got.ExitCode != 0 || got.Step != 3 {
		t.Fatalf("job %+v", got)
	}
	if got.Owner != "agent:a" || got.Kind != domain.JobPipeline || got.Name != "ci" || got.LogPath == "" || got.FinishedAt == nil {
		t.Fatalf("job metadata %+v", got)
	}
	argvs := h.argvs()
	want := [][]string{
		{"/bin/sh", "-c", "step-lint"},
		{"/bin/sh", "-c", "step-unit"},
		{"task", "e2e", "-run", "X"}, // args go to the last step only
	}
	if !slices.EqualFunc(argvs, want, slices.Equal) {
		t.Fatalf("argv %q", argvs)
	}
	spec := h.runner.started[0]
	env := envMap(spec.Env)
	if spec.Dir != "/code/jobs" || env["FROM_DOTENV"] != "1" || env["ROCKET_OWNER"] != "" {
		t.Fatalf("spec dir %s env %v", spec.Dir, env)
	}
	var states []domain.JobStatus
	for _, e := range h.bus.events {
		if e.Type == domain.EventJobState && e.JobID == job.ID {
			states = append(states, e.Status)
		}
	}
	if len(states) < 2 || states[0] != domain.JobRunning || states[len(states)-1] != domain.JobSucceeded {
		t.Fatalf("job.state events %v", states)
	}
}

func TestRunPipelineStopsOnFirstFailure(t *testing.T) {
	h := newHarness(t)
	h.runner.exitNow["step-ok"] = 0
	h.runner.exitNow["step-boom"] = 3

	got := h.waitJob(t, h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "broken"}).ID)
	if got.Status != domain.JobFailed || *got.ExitCode != 3 || got.Step != 2 {
		t.Fatalf("job %+v", got)
	}
	if n := len(h.argvs()); n != 2 {
		t.Fatalf("started %d steps, want 2 (stop on first failure)", n)
	}
}

func TestRunArgsAppendToShellStep(t *testing.T) {
	h := newHarness(t)
	h.runner.exitNow["step-slow"] = 0
	h.waitJob(t, h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "slow", Args: []string{"a", "b c"}}).ID)
	want := []string{"/bin/sh", "-c", `step-slow "$@"`, "rocket", "a", "b c"}
	if got := h.argvs()[0]; !slices.Equal(got, want) {
		t.Fatalf("argv %q", got)
	}
}

func TestRunFallsBackToTaskfileTask(t *testing.T) {
	h := newHarness(t)
	h.runner.exitNow["task build"] = 0
	got := h.waitJob(t, h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "build"}).ID)
	if got.Status != domain.JobSucceeded || len(got.Steps) != 1 || got.Steps[0].Task != "build" {
		t.Fatalf("job %+v", got)
	}

	_, err := h.app.StartJob(context.Background(), JobRequest{Project: "/code/nuvara", Kind: domain.JobPipeline, Name: "build"})
	if !errors.Is(err, ErrInvalid) {
		t.Fatalf("no Taskfile: want ErrInvalid, got %v", err)
	}
}

func TestSetupRunsDoctorInstallMigrateSkippingUndefined(t *testing.T) {
	h := newHarness(t)
	h.runner.exitNow["doctor-ok"] = 0
	h.runner.exitNow["task db:migrate"] = 0
	h.runner.exitNow["seed-it"] = 0

	got := h.waitJob(t, h.startJob(t, JobRequest{Kind: domain.JobSetup}).ID)
	if got.Status != domain.JobSucceeded || got.Name != "setup" || len(got.Steps) != 2 ||
		got.Steps[0].Run != "doctor-ok" || got.Steps[1].Task != "db:migrate" {
		t.Fatalf("setup job %+v", got)
	}

	seed := h.waitJob(t, h.startJob(t, JobRequest{Kind: domain.JobSetup, Name: "seed"}).ID)
	if seed.Name != "seed" || len(seed.Steps) != 1 || seed.Status != domain.JobSucceeded {
		t.Fatalf("seed job %+v", seed)
	}

	for _, tc := range []struct{ project, name string }{{"/code/jobs", "install"}, {"/code/nuvara", ""}} {
		if _, err := h.app.StartJob(context.Background(), JobRequest{Project: tc.project, Kind: domain.JobSetup, Name: tc.name}); !errors.Is(err, ErrInvalid) {
			t.Fatalf("%s setup %q: want ErrInvalid, got %v", tc.project, tc.name, err)
		}
	}
}

func TestDeployGate(t *testing.T) {
	tests := []struct {
		name    string
		req     JobRequest
		wantErr error
	}{
		{"env without deploy", JobRequest{Name: "dev"}, ErrInvalid},
		{"unknown env", JobRequest{Name: "nope"}, ErrInvalid},
		{"confirm env without --yes", JobRequest{Name: "prod"}, ErrConfirmation},
		{"confirm env with --yes", JobRequest{Name: "prod", Yes: true}, nil},
		{"no-confirm env by user", JobRequest{Name: "stage"}, nil},
		{"agent without flags", JobRequest{Name: "stage", Owner: "agent:x"}, ErrConfirmation},
		{"agent with --yes only", JobRequest{Name: "stage", Owner: "agent:x", Yes: true}, ErrConfirmation},
		{"agent with allow only", JobRequest{Name: "stage", Owner: "agent:x", AllowAgentDeploy: true}, ErrConfirmation},
		{"agent with --yes and allow", JobRequest{Name: "prod", Owner: "agent:x", Yes: true, AllowAgentDeploy: true}, nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			h := newHarness(t)
			h.runner.exitNow["deploy-stage"] = 0
			h.runner.exitNow["task release:prod"] = 0
			tt.req.Project = "/code/jobs"
			tt.req.Kind = domain.JobDeploy
			job, err := h.app.StartJob(context.Background(), tt.req)
			if tt.wantErr != nil {
				if !errors.Is(err, tt.wantErr) {
					t.Fatalf("want %v, got %v", tt.wantErr, err)
				}
				if n := len(h.argvs()); n != 0 {
					t.Fatalf("refused deploy started %d processes", n)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			got := h.waitJob(t, job.ID)
			if got.Kind != domain.JobDeploy || got.Name != tt.req.Name || got.Env != tt.req.Name || got.Status != domain.JobSucceeded {
				t.Fatalf("deploy job %+v", got)
			}
		})
	}
}

func TestCancelJobStopsProcessGroup(t *testing.T) {
	h := newHarness(t)
	job := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "slow"})
	running := h.waitJobPGID(t, job.ID)

	got, err := h.app.CancelJob(context.Background(), job.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Status != domain.JobCanceled || got.ExitCode == nil || *got.ExitCode != 143 {
		t.Fatalf("canceled job %+v", got)
	}
	if !slices.Contains(h.runner.stopped, running.PGID) {
		t.Fatalf("pgid %d not stopped (%v)", running.PGID, h.runner.stopped)
	}
	if _, err := h.app.CancelJob(context.Background(), "nope"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("unknown job: %v", err)
	}
}

func TestReconcileMarksVanishedJobsLost(t *testing.T) {
	h := newHarness(t)
	start := h.clock.Now()
	_ = h.store.SaveJob(domain.Job{ID: "jdead", Project: "jobs", Name: "slow", Kind: domain.JobPipeline, Status: domain.JobRunning, PID: 5555, PGID: 5555, StartedAt: start})
	_ = h.store.SaveJob(domain.Job{ID: "jlive", Project: "jobs", Name: "slow", Kind: domain.JobPipeline, Status: domain.JobRunning, PID: 6666, PGID: 6666, StartedAt: start})
	h.runner.mu.Lock()
	h.runner.alive[6666] = true
	h.runner.mu.Unlock()

	rec, err := h.app.Reconcile(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(rec.LostJobs, []string{"jdead"}) {
		t.Fatalf("lost jobs %v", rec.LostJobs)
	}
	if j, _ := h.app.GetJob("jdead"); j.Status != domain.JobLost || j.FinishedAt == nil {
		t.Fatalf("jdead %+v", j)
	}
	if j, _ := h.app.GetJob("jlive"); j.Status != domain.JobRunning {
		t.Fatalf("jlive should be adopted, got %+v", j)
	}
	h.runner.mu.Lock()
	h.runner.alive[6666] = false
	h.runner.mu.Unlock()
	if j := h.waitJob(t, "jlive"); j.Status != domain.JobLost {
		t.Fatalf("jlive after exit %+v", j)
	}
}

func TestDownByOwnerCancelsThatOwnersJobs(t *testing.T) {
	h := newHarness(t)
	agentJob := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "slow", Owner: "agent:z"})
	h.clock.Advance(time.Second)
	userJob := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "slow"})
	h.waitJobPGID(t, agentJob.ID)
	h.waitJobPGID(t, userJob.ID)

	res, err := h.app.Down(context.Background(), DownRequest{Project: "/code/jobs", Owner: "agent:z"})
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(res.CanceledJobs, []string{agentJob.ID}) {
		t.Fatalf("canceled %v", res.CanceledJobs)
	}
	if j, _ := h.app.GetJob(userJob.ID); j.Status != domain.JobRunning {
		t.Fatalf("user job must survive: %+v", j)
	}
	list, err := h.app.ListJobs(JobsRequest{Project: "/code/jobs"})
	if err != nil {
		t.Fatal(err)
	}
	if len(list.Jobs) != 2 || list.Jobs[0].ID != userJob.ID {
		t.Fatalf("jobs newest first: %+v", list.Jobs)
	}
	if _, err := h.app.CancelJob(context.Background(), userJob.ID); err != nil {
		t.Fatal(err)
	}
}

func TestSummaryReportsServicesJobsAndConflicts(t *testing.T) {
	h := newHarness(t)
	h.probe.busy[3002] = &domain.PortHolder{PID: 42, Command: "node"}
	h.probe.busy[3000] = &domain.PortHolder{PID: 77, Command: "bun"}
	h.up(t, UpRequest{Services: []string{"api"}})

	sum, err := h.app.Summary(context.Background(), "/code/nuvara")
	if err != nil {
		t.Fatal(err)
	}
	if sum.Project == nil || sum.Project.Name != "nuvara" || len(sum.Services) != 5 || sum.Jobs == nil {
		t.Fatalf("summary %+v", sum)
	}
	var remapped, busy *Conflict
	for i, c := range sum.Conflicts {
		switch {
		case c.Kind == ConflictPortRemapped && c.Service == "api":
			remapped = &sum.Conflicts[i]
		case c.Kind == ConflictPortBusy && c.Service == "web":
			busy = &sum.Conflicts[i]
		}
	}
	if remapped == nil || remapped.Port != 3102 || remapped.Default != 3002 {
		t.Fatalf("remapped conflict %+v in %+v", remapped, sum.Conflicts)
	}
	if busy == nil || busy.Port != 3000 || !busy.Remappable || busy.Holder == nil || busy.Holder.PID != 77 {
		t.Fatalf("busy conflict %+v in %+v", busy, sum.Conflicts)
	}
}

func TestGetJobUnknown(t *testing.T) {
	h := newHarness(t)
	if _, err := h.app.GetJob("missing"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("got %v", err)
	}
}
