package app

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
	"github.com/xean-io/rocket/internal/ports"
)

func pipelineNeedsHarness(t *testing.T) (*harness, *domain.Project) {
	t.Helper()
	p, err := manifest.Parse([]byte(`version: 1
name: needs
envs: {dev: {}, smoke: {}}
services:
  db: {run: db-serve, cwd: db, ports: {main: {default: 6432, env: DB_PORT}}}
  api: {run: api-serve, cwd: api, depends_on: [db], ports: {http: {default: 8080, env: API_PORT}}}
groups: {infra: [db]}
pipelines:
  check: {needs: [infra, api], steps: [{run: job-step}]}
`), "/code/needs")
	if err != nil {
		t.Fatalf("parse pipeline needs: %v", err)
	}
	h := newHarness(t)
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	h.runner.exitNow["job-step"] = 0
	return h, p
}

func requestTTL(t *testing.T, req JobRequest, ttl string) JobRequest {
	t.Helper()
	data, err := json.Marshal(map[string]string{"ttl": ttl})
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &req); err != nil {
		t.Fatal(err)
	}
	return req
}

func jobExpiration(t *testing.T, job domain.Job) *time.Time {
	t.Helper()
	data, err := json.Marshal(job)
	if err != nil {
		t.Fatal(err)
	}
	var record struct {
		ExpiresAt *time.Time `json:"expires_at"`
	}
	if err := json.Unmarshal(data, &record); err != nil {
		t.Fatal(err)
	}
	return record.ExpiresAt
}

type needsBlockingHealth struct {
	port    int
	entered chan struct{}
	release chan struct{}
	once    sync.Once
}

func (b *needsBlockingHealth) Check(ctx context.Context, check domain.HealthCheck) error {
	if check.Port != b.port {
		return nil
	}
	b.once.Do(func() { close(b.entered) })
	select {
	case <-b.release:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func blockNeedsHealth(t *testing.T, h *harness, port int) (*needsBlockingHealth, func()) {
	t.Helper()
	b := &needsBlockingHealth{port: port, entered: make(chan struct{}), release: make(chan struct{})}
	h.app.d.Health = b
	var once sync.Once
	release := func() { once.Do(func() { close(b.release) }) }
	t.Cleanup(release)
	return b, release
}

func waitNeedsStartup(t *testing.T, entered <-chan struct{}) {
	t.Helper()
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("prerequisite startup never reached readiness")
	}
}

func assertNoJobSteps(t *testing.T, h *harness) {
	t.Helper()
	for _, argv := range h.argvs() {
		if strings.Contains(strings.Join(argv, " "), "job-step") {
			t.Fatalf("job step started before prerequisites completed: %v", argv)
		}
	}
}

func TestPipelineNeedsPersistBeforeAsyncStartupWithExactDeadline(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	block, release := blockNeedsHealth(t, h, 6432)
	started := h.clock.Now()
	req := requestTTL(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check", Env: "smoke", Owner: "agent:check"}, "30m")
	returned := make(chan domain.Job, 1)
	go func() { job, _ := h.app.StartJob(context.Background(), req); returned <- job }()
	var job domain.Job
	select {
	case job = <-returned:
	case <-time.After(time.Second):
		t.Fatal("StartJob blocked on prerequisite readiness")
	}
	if persisted, ok, err := h.store.GetJob(job.ID); err != nil || !ok || persisted.Status != domain.JobRunning {
		t.Fatalf("job must be persisted before async Up: %+v, %v, %v", persisted, ok, err)
	}
	waitNeedsStartup(t, block.entered)
	assertNoJobSteps(t, h)
	h.clock.Advance(7 * time.Minute)
	release()
	finished := h.waitJob(t, job.ID)
	if finished.Status != domain.JobSucceeded {
		t.Fatalf("job failed: %+v", finished)
	}
	deadline := jobExpiration(t, job)
	if deadline == nil || !deadline.Equal(started.Add(30*time.Minute)) {
		t.Fatalf("job deadline = %v", deadline)
	}
	for _, name := range []string{"db", "api"} {
		run := h.run(t, p.Name, name)
		if run.State != domain.StateRunning || run.Owner != "agent:check" || run.Env != "smoke" || run.ExpiresAt == nil || !run.ExpiresAt.Equal(*deadline) {
			t.Errorf("prerequisite %s did not inherit exact job identity/deadline: %+v", name, run)
		}
	}
}

func TestPipelineNeedsPreserveExistingOwnerAndDeadline(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	h.up(t, UpRequest{Project: p.Root, Services: []string{"db"}, Owner: "user", TTL: "1h"})
	existing := h.run(t, p.Name, "db")
	h.clock.Advance(time.Minute)
	job := h.startJob(t, requestTTL(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check", Owner: "agent:check"}, "30m"))
	if finished := h.waitJob(t, job.ID); finished.Status != domain.JobSucceeded {
		t.Fatalf("job failed: %+v", finished)
	}
	db := h.run(t, p.Name, "db")
	if db.PID != existing.PID || db.Owner != "user" || db.ExpiresAt == nil || !db.ExpiresAt.Equal(*existing.ExpiresAt) {
		t.Fatalf("existing ownership/deadline changed: %+v", db)
	}
	api := h.run(t, p.Name, "api")
	deadline := jobExpiration(t, job)
	if deadline == nil || api.ExpiresAt == nil || !api.ExpiresAt.Equal(*deadline) || api.Owner != "agent:check" {
		t.Fatalf("new prerequisite did not inherit job owner/deadline: %+v, %v", api, deadline)
	}
}

func TestPipelineNeedsFailurePreventsSteps(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	db := p.Services["db"]
	db.Ports[0].Env = ""
	p.Services["db"] = db
	h.probe.busy[6432] = &domain.PortHolder{PID: 77}
	job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check"})
	finished := h.waitJob(t, job.ID)
	if finished.Status != domain.JobFailed || !strings.Contains(finished.Error, "prerequisite") || finished.Step != 0 {
		t.Fatalf("failed prerequisite job: %+v", finished)
	}
	assertNoJobSteps(t, h)
}

func TestPipelineNeedsCancellationDuringReadiness(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	block, _ := blockNeedsHealth(t, h, 8080)
	job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check"})
	waitNeedsStartup(t, block.entered)
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	finished, err := h.app.CancelJob(ctx, job.ID)
	if err != nil || finished.Status != domain.JobCanceled {
		t.Fatalf("cancel prerequisites: %+v, %v", finished, err)
	}
	assertNoJobSteps(t, h)
	if run := h.run(t, p.Name, "api"); run.State.Active() || h.runner.Alive(run.PID, run.PGID) {
		t.Fatalf("interrupted startup remains active: %+v", run)
	}
	if run := h.run(t, p.Name, "db"); run.State != domain.StateRunning {
		t.Fatalf("previously ready prerequisite should remain running: %+v", run)
	}
}

func TestPipelineNeedsCancellationWhileWaitingProjectLock(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	gate := h.app.projectLock(p.Name)
	gate.Lock()
	defer gate.Unlock()
	job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check"})
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	finished, err := h.app.CancelJob(ctx, job.ID)
	if err != nil || finished.Status != domain.JobCanceled || len(h.argvs()) != 0 {
		t.Fatalf("queued startup must cancel before lock release: %+v, %v, %v", finished, err, h.argvs())
	}
}

func TestUpCancellationWhileWaitingProjectLock(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	gate := h.app.projectLock(p.Name)
	gate.Lock()
	var once sync.Once
	unlock := func() { once.Do(gate.Unlock) }
	defer unlock()
	ctx, cancel := context.WithCancel(context.Background())
	returned := make(chan error, 1)
	go func() {
		_, err := h.app.Up(ctx, UpRequest{Project: p.Root, Services: []string{"api"}})
		returned <- err
	}()
	cancel()
	select {
	case err := <-returned:
		if !errors.Is(err, context.Canceled) || len(h.argvs()) != 0 {
			t.Fatalf("canceled Up started prerequisites: %v, %v", err, h.argvs())
		}
	case <-time.After(200 * time.Millisecond):
		unlock()
		<-returned
		t.Fatal("canceled Up waited for the held project lock")
	}
}

func TestDownCancelsPipelineBeforePrerequisiteSnapshot(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	block, release := blockNeedsHealth(t, h, 8080)
	job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check"})
	waitNeedsStartup(t, block.entered)
	type result struct {
		down DownResult
		err  error
	}
	returned := make(chan result, 1)
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	go func() { down, err := h.app.Down(ctx, DownRequest{Project: p.Root}); returned <- result{down, err} }()
	select {
	case got := <-returned:
		if got.err != nil || !slices.Contains(got.down.CanceledJobs, job.ID) {
			t.Fatalf("down = %+v, %v", got.down, got.err)
		}
	case <-ctx.Done():
		release()
		<-returned
		t.Fatal("down deadlocked behind prerequisite startup")
	}
	assertNoJobSteps(t, h)
	runs, err := h.store.ListRuns()
	if err != nil {
		t.Fatal(err)
	}
	for _, run := range runs {
		if run.State.Active() || h.runner.Alive(run.PID, run.PGID) {
			t.Errorf("down left a prerequisite active: %+v", run)
		}
	}
	leases, err := h.store.ListLeases()
	if err != nil || len(leases) != 0 {
		t.Fatalf("leftover leases: %v, %v", leases, err)
	}
}

type needsBlockingCompose struct {
	*fakeCompose
	entered chan struct{}
}

func (b *needsBlockingCompose) Up(ctx context.Context, target ports.ComposeTarget, service string, out io.Writer) (string, error) {
	_, _ = b.fakeCompose.Up(ctx, target, service, out)
	close(b.entered)
	<-ctx.Done()
	return "", ctx.Err()
}

func TestPipelineNeedsCancellationStopsAttemptedComposeService(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	db := p.Services["db"]
	db.Kind, db.Run, db.Compose = domain.KindCompose, "", "database"
	p.Services["db"] = db
	compose := &needsBlockingCompose{fakeCompose: h.compose, entered: make(chan struct{})}
	h.app.d.Compose = compose
	job := h.startJob(t, JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check"})
	waitNeedsStartup(t, compose.entered)
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	finished, err := h.app.CancelJob(ctx, job.ID)
	if err != nil || finished.Status != domain.JobCanceled {
		t.Fatalf("cancel compose: %+v, %v", finished, err)
	}
	if ops := h.compose.ops(); !slices.Equal(ops, []string{"up:database", "stop:database"}) {
		t.Fatalf("cancellation must stop only attempted compose service: %v", ops)
	}
	if running, _ := h.compose.Running(context.Background(), domain.ComposeProjectName(p.Name, "dev"), "database"); running {
		t.Fatal("attempted container remained running")
	}
	assertNoJobSteps(t, h)
	for _, request := range []DownRequest{{Project: p.Root, Owner: "user"}, {Project: p.Root, Services: []string{"db"}}} {
		down, err := h.app.Down(context.Background(), request)
		if err != nil || len(down.ComposeDown) != 0 || len(down.Stopped) != 0 {
			t.Fatalf("selective/owner down must not remove a whole compose project: %+v, %v", down, err)
		}
	}
	down, err := h.app.Down(context.Background(), DownRequest{Project: p.Root})
	if err != nil || !slices.Equal(down.ComposeDown, []string{domain.ComposeProjectName(p.Name, "dev")}) {
		t.Fatalf("whole down must remove tracked failed compose attempts: %+v, %v", down, err)
	}
	again, err := h.app.Down(context.Background(), DownRequest{Project: p.Root})
	if err != nil || len(again.Stopped) != 0 || len(again.ComposeDown) != 0 {
		t.Fatalf("repeat whole down must remain idempotent: %+v, %v", again, err)
	}
}

func TestUpCancellationReleasesAttemptedServiceLeases(t *testing.T) {
	h, p := pipelineNeedsHarness(t)
	h.up(t, UpRequest{Project: p.Root, Services: []string{"db"}})
	block, _ := blockNeedsHealth(t, h, 8080)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	returned := make(chan UpResult, 1)
	go func() {
		result, _ := h.app.Up(ctx, UpRequest{Project: p.Root, Services: []string{"api"}})
		returned <- result
	}()
	waitNeedsStartup(t, block.entered)
	cancel()
	select {
	case result := <-returned:
		if !result.Failed() {
			t.Fatalf("canceled startup did not fail: %+v", result)
		}
	case <-time.After(time.Second):
		t.Fatal("canceled startup did not unwind")
	}
	leases, err := h.store.ListLeases()
	if err != nil || len(leases) != 1 || leases[0].Service != "db" {
		t.Fatalf("cancellation must release only attempted startup leases: %v, %v", leases, err)
	}
	api := h.run(t, p.Name, "api")
	if api.State.Active() || h.runner.Alive(api.PID, api.PGID) {
		t.Fatalf("orphan startup process: %+v", api)
	}
}

func TestPipelineNeedsRejectInvalidTTLAndTargetsBeforePersistence(t *testing.T) {
	for _, invalid := range []string{"not-duration", "0s", "-1s", "unknown-target"} {
		t.Run(invalid, func(t *testing.T) {
			h, p := pipelineNeedsHarness(t)
			req := JobRequest{Project: p.Root, Kind: domain.JobPipeline, Name: "check"}
			if invalid == "unknown-target" {
				p.PipelineNeeds["check"] = []string{"missing"}
			} else {
				req = requestTTL(t, req, invalid)
			}
			_, err := h.app.StartJob(context.Background(), req)
			jobs, _ := h.store.ListJobs("", 0)
			if !errors.Is(err, ErrInvalid) || len(jobs) != 0 || len(h.argvs()) != 0 {
				t.Fatalf("invalid prerequisite request caused side effects: error=%v, jobs=%v", err, jobs)
			}
		})
	}
}
