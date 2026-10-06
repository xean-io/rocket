package app

import (
	"context"
	"slices"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

func assertJobExpired(t *testing.T, h *harness, id string) domain.Job {
	t.Helper()
	j, err := h.app.GetJob(id)
	if err != nil || j.Status != domain.JobCanceled || j.Error != "ttl expired" || j.FinishedAt == nil {
		t.Fatalf("expired job = %+v, %v", j, err)
	}
	h.bus.mu.Lock()
	defer h.bus.mu.Unlock()
	terminals := 0
	for _, event := range h.bus.events {
		if event.JobID == id && event.Type == domain.EventJobState && event.Status.Terminal() {
			terminals++
		}
	}
	if terminals != 1 {
		t.Fatalf("terminal events = %d, want one", terminals)
	}
	return j
}

func TestGCExpiresJobStepWithReasonAndIdentity(t *testing.T) {
	h := newHarness(t)
	j := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "slow", TTL: "1h"})
	running := h.waitJobPGID(t, j.ID)
	if got := jobExpiration(t, running); got == nil || !got.Equal(h.clock.Now().Add(time.Hour)) {
		t.Fatalf("deadline = %v", got)
	}
	h.clock.Advance(time.Hour)
	res, err := h.app.GC(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	assertJobExpired(t, h, j.ID)
	found := false
	for _, action := range res.Actions {
		if action.Action == "expired_job" && action.Detail == j.ID && action.Project == "jobs" {
			found = true
		}
	}
	if !found {
		t.Fatalf("GC omitted expired_job identity: %+v", res)
	}
	h.runner.mu.Lock()
	stops := slices.Clone(h.runner.stopped)
	h.runner.mu.Unlock()
	if count := countInt(stops, running.PGID); count != 1 {
		t.Fatalf("step stopped %d times: %v", count, stops)
	}
}

func countInt(values []int, wanted int) int {
	count := 0
	for _, value := range values {
		if value == wanted {
			count++
		}
	}
	return count
}

func TestReconcileExpiredPersistedJobsBeforeLostOrAdopted(t *testing.T) {
	for _, live := range []bool{false, true} {
		t.Run(map[bool]string{false: "no PID", true: "live process"}[live], func(t *testing.T) {
			h := newHarness(t)
			deadline := h.clock.Now().Add(-time.Second)
			j := domain.Job{ID: "jexpired", Project: "jobs", Name: "slow", Kind: domain.JobPipeline, Status: domain.JobRunning, StartedAt: deadline.Add(-time.Hour), ExpiresAt: &deadline}
			if live {
				j.PID, j.PGID = 6666, 6666
				h.runner.alive[6666] = true
			}
			if err := h.store.SaveJob(j); err != nil {
				t.Fatal(err)
			}
			res, err := h.app.Reconcile(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if len(res.LostJobs) != 0 {
				t.Fatalf("expired job marked lost: %+v", res)
			}
			assertJobExpired(t, h, j.ID)
			if live && h.runner.Alive(j.PID, j.PGID) {
				t.Fatal("expired adopted process remains alive")
			}
		})
	}
}

func TestAdoptedJobRetainsDeadline(t *testing.T) {
	h := newHarness(t)
	deadline := h.clock.Now().Add(time.Hour)
	j := domain.Job{ID: "jadopted", Project: "jobs", Name: "slow", Kind: domain.JobPipeline, Status: domain.JobRunning, PID: 6666, PGID: 6666, StartedAt: h.clock.Now(), ExpiresAt: &deadline}
	h.runner.alive[6666] = true
	if err := h.store.SaveJob(j); err != nil {
		t.Fatal(err)
	}
	if _, err := h.app.Reconcile(context.Background()); err != nil {
		t.Fatal(err)
	}
	if got, _ := h.app.GetJob(j.ID); got.Status != domain.JobRunning || got.ExpiresAt == nil || !got.ExpiresAt.Equal(deadline) {
		t.Fatalf("adopted deadline changed: %+v", got)
	}
	h.clock.Advance(time.Hour)
	if _, err := h.app.GC(context.Background()); err != nil {
		t.Fatal(err)
	}
	assertJobExpired(t, h, j.ID)
}

func TestJobTTLTimerExpiresWithoutMaintenance(t *testing.T) {
	h := newHarness(t)
	h.app.d.Now = time.Now
	j := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: "slow", TTL: "25ms"})
	h.waitJobPGID(t, j.ID)
	got := h.waitJob(t, j.ID)
	if got.Status != domain.JobCanceled || got.Error != "ttl expired" {
		t.Fatalf("independent timer did not expire job: %+v", got)
	}
}

func TestJobTTLInterruptsBlockedPrerequisiteAndProjectGate(t *testing.T) {
	for _, mode := range []string{"readiness", "project gate", "compose"} {
		t.Run(mode, func(t *testing.T) {
			h, p := pipelineNeedsHarness(t)
			var entered <-chan struct{}
			switch mode {
			case "readiness":
				blocking, _ := blockNeedsHealth(t, h, 6432)
				entered = blocking.entered
			case "project gate":
				gate := h.app.projectLock(p.Name)
				gate.Lock()
				t.Cleanup(gate.Unlock)
			case "compose":
				svc := p.Services["db"]
				svc.Kind, svc.Compose, svc.Run = domain.KindCompose, "database", ""
				p.Services["db"] = svc
				blocking := &needsBlockingCompose{fakeCompose: h.compose, entered: make(chan struct{})}
				h.app.d.Compose = blocking
				entered = blocking.entered
			}
			job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check", TTL: "1h"})
			if entered != nil {
				waitNeedsStartup(t, entered)
			}
			h.clock.Advance(time.Hour)
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			if _, err := h.app.GC(ctx); err != nil {
				t.Fatalf("expiry blocked by %s: %v", mode, err)
			}
			j := assertJobExpired(t, h, job.ID)
			if j.Step != 0 {
				t.Fatalf("steps ran after startup expiry: %+v", j)
			}
		})
	}
}

type deadlineHealth struct{ advance func() }

func (h deadlineHealth) Check(context.Context, domain.HealthCheck) error {
	h.advance()
	return nil
}

type deadlineRunner struct {
	*fakeRunner
	advance func()
}

type deadlineLiveness struct {
	*fakeRunner
	advance func()
}

func (r deadlineLiveness) Alive(int, int) bool {
	r.advance()
	return false
}

func TestReconcileDeadlineCrossingReportsExpiredNotLost(t *testing.T) {
	h := newHarness(t)
	deadline := h.clock.Now().Add(time.Hour)
	job := domain.Job{ID: "jcrossing", Project: "jobs", Name: "slow", Status: domain.JobRunning, PID: 6666, PGID: 6666, StartedAt: h.clock.Now(), ExpiresAt: &deadline}
	if err := h.store.SaveJob(job); err != nil {
		t.Fatal(err)
	}
	h.app.d.Runner = deadlineLiveness{fakeRunner: h.runner, advance: func() { h.clock.Advance(time.Hour) }}
	result, err := h.app.Reconcile(context.Background())
	if err != nil || len(result.LostJobs) != 0 || !slices.Equal(result.ExpiredJobs, []string{job.ID}) {
		t.Fatalf("deadline crossing metadata: %+v, %v", result, err)
	}
	assertJobExpired(t, h, job.ID)
}

func (r deadlineRunner) Start(spec ports.ProcessSpec) (ports.ProcessHandle, error) {
	h, err := r.fakeRunner.Start(spec)
	r.advance()
	return h, err
}

func TestJobDeadlineCheckedAtTransitions(t *testing.T) {
	t.Run("before first step after readiness", func(t *testing.T) {
		h, p := pipelineNeedsHarness(t)
		h.app.d.Health = deadlineHealth{advance: func() { h.clock.Advance(time.Hour) }}
		job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check", TTL: "1h"})
		h.waitJob(t, job.ID)
		finished := assertJobExpired(t, h, job.ID)
		if finished.Step != 0 {
			t.Fatalf("expired startup ran a step: %+v", finished)
		}
		assertNoJobSteps(t, h)
	})
	for _, pipeline := range []string{"ci", "slow"} {
		t.Run(pipeline, func(t *testing.T) {
			h := newHarness(t)
			for _, command := range []string{"step-lint", "step-unit", "task e2e", "step-slow"} {
				h.runner.exitNow[command] = 0
			}
			h.app.d.Runner = deadlineRunner{fakeRunner: h.runner, advance: func() { h.clock.Advance(time.Hour) }}
			job := h.startJob(t, JobRequest{Kind: domain.JobPipeline, Name: pipeline, TTL: "1h"})
			h.waitJob(t, job.ID)
			finished := assertJobExpired(t, h, job.ID)
			if finished.Step != 1 || len(h.argvs()) != 1 {
				t.Fatalf("steps continued after deadline: %+v, %v", finished, h.argvs())
			}
		})
	}
}

func TestGCRetainsFailedComposePrerequisiteUntilWholeDown(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	db := p.Services["db"]
	db.Kind, db.Run, db.Compose = domain.KindCompose, "", "database"
	p.Services["db"] = db
	compose := &needsBlockingCompose{fakeCompose: h.compose, entered: make(chan struct{})}
	h.app.d.Compose = compose
	job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check", TTL: "1h"})
	waitNeedsStartup(t, compose.entered)
	h.clock.Advance(time.Hour)
	if _, err := h.app.GC(context.Background()); err != nil {
		t.Fatal(err)
	}
	assertJobExpired(t, h, job.ID)
	down, err := h.app.Down(context.Background(), DownRequest{Project: p.Root})
	if err != nil || !slices.Equal(down.ComposeDown, []string{domain.ComposeProjectName(p.Name, "dev")}) {
		t.Fatalf("GC discarded the failed Compose cleanup record: %+v, %v", down, err)
	}
	if _, err := h.app.GC(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, exists, _ := h.store.GetRun(p.Name, "db"); exists {
		t.Fatal("successfully stopped Compose record was not pruned")
	}
}
