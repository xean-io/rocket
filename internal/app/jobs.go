package app

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// ErrConfirmation means a deploy needs explicit human confirmation.
var ErrConfirmation = errors.New("confirmation required")

// SetupSequence is what `rocket setup` runs when no name is given.
var SetupSequence = []string{"doctor", "install", "migrate"}

// KeepJobs is how many finished jobs per project GC keeps.
const KeepJobs = 100

// JobRequest starts a pipeline, setup action or deploy.
type JobRequest struct {
	Project  string         `json:"project"`
	Kind     domain.JobKind `json:"kind"`               // setup | pipeline | deploy
	Name     string         `json:"name,omitempty"`     // pipeline/task, setup name ("" = doctor→install→migrate) or deploy env
	Env      string         `json:"env,omitempty"`      // setup/pipeline env; defaults to default_env; deploy must match name
	Profiles []string       `json:"profiles,omitempty"` // requested prerequisite startup profiles; setup/pipeline only
	TTL      string         `json:"ttl,omitempty"`      // positive Go duration; deadline starts at job creation
	Args     []string       `json:"args,omitempty"`     // appended to the last step
	Owner    string         `json:"owner,omitempty"`
	// Yes confirms a deploy (--yes).
	Yes bool `json:"yes,omitempty"`
	// AllowAgentDeploy reports ROCKET_ALLOW_DEPLOY=1 in the caller's
	// environment; agents need it in addition to Yes.
	AllowAgentDeploy bool `json:"allow_agent_deploy,omitempty"`
}

// JobsRequest lists jobs of one project (or all).
type JobsRequest struct {
	Project     string `json:"project,omitempty"`
	AllProjects bool   `json:"all_projects,omitempty"`
	Limit       int    `json:"limit,omitempty"` // default 50
}

// JobsResult lists jobs newest first.
type JobsResult struct {
	Jobs []domain.Job `json:"jobs"`
}

// JobLogsResult carries the tail of a job log.
type JobLogsResult struct {
	Job     string   `json:"job"`
	Project string   `json:"project"`
	Lines   []string `json:"lines"`
}

// JobOutcome is the final summary returned by blocking job commands.
type JobOutcome struct {
	Job        string           `json:"job"`
	Project    string           `json:"project"`
	Kind       domain.JobKind   `json:"kind"`
	Name       string           `json:"name"`
	Status     domain.JobStatus `json:"status"`
	ExitCode   *int             `json:"exit_code"`
	DurationMS int64            `json:"duration_ms"`
	LogPath    string           `json:"log_path"`
	Tail       []string         `json:"tail"`
	Error      string           `json:"error,omitempty"`
}

// Outcome builds the blocking-command summary of a job.
func Outcome(j domain.Job, tail []string) JobOutcome {
	if tail == nil {
		tail = []string{}
	}
	return JobOutcome{Job: j.ID, Project: j.Project, Kind: j.Kind, Name: j.Name, Status: j.Status, ExitCode: j.ExitCode,
		DurationMS: j.DurationMS, LogPath: j.LogPath, Tail: tail, Error: j.Error}
}

// jobRun is the daemon-side handle of a supervised job. Guarded by App.mu.
type jobRun struct {
	canceled bool
	reason   string
	cancel   context.CancelFunc
	pgid     int
	done     chan struct{}
	stopOnce sync.Once
	stopErr  error
}

func newJobID() string {
	b := make([]byte, 5)
	_, _ = rand.Read(b)
	return "j" + hex.EncodeToString(b)
}

// StartJob validates the request, persists the job and runs its steps in
// the background. It returns as soon as the job is running.
func (a *App) StartJob(ctx context.Context, req JobRequest) (domain.Job, error) {
	p, err := a.ResolveProject(req.Project)
	if err != nil {
		return domain.Job{}, err
	}
	owner := req.Owner
	if owner == "" {
		owner = domain.DefaultOwner
	}
	name, env, steps, err := a.resolveJob(p, req, owner)
	if err != nil {
		return domain.Job{}, err
	}
	var profiles []string
	if req.Kind != domain.JobDeploy {
		profiles = domain.MergeProfiles(p.Envs[env].Profiles, req.Profiles)
	}
	var needs []string
	if req.Kind == domain.JobPipeline {
		needs = append([]string(nil), p.PipelineNeeds[name]...)
		if len(needs) > 0 {
			if _, err := p.ExpandTargets(needs); err != nil {
				return domain.Job{}, fmt.Errorf("%w: pipeline %q prerequisites: %v", ErrInvalid, name, err)
			}
		}
	}
	startedAt := a.now()
	var expires *time.Time
	if req.TTL != "" {
		ttl, err := time.ParseDuration(req.TTL)
		if err != nil || ttl <= 0 {
			return domain.Job{}, fmt.Errorf("%w: invalid ttl %q (use a positive Go duration like 30m)", ErrInvalid, req.TTL)
		}
		deadline := startedAt.Add(ttl)
		expires = &deadline
	}
	dotenv, err := a.d.Env.Dotenv(p.Root, p.Dotenv)
	if err != nil {
		return domain.Job{}, fmt.Errorf("read dotenv: %w", err)
	}
	identity := map[string]string{"COMPOSE_PROJECT_NAME": domain.ComposeProjectName(p.Name, env)}
	childEnv := domain.BuildEnv(a.d.BaseEnv(), dotenv, identity)

	id := a.d.NewID()
	logf, logPath, err := a.d.Logs.OpenJob(p.Name, id)
	if err != nil {
		return domain.Job{}, fmt.Errorf("open job log: %w", err)
	}
	job := domain.Job{
		ID: id, Project: p.Name, Name: name, Kind: req.Kind, Env: env, Profiles: profiles, Owner: owner, Steps: append([]domain.Step{}, steps...), Args: append([]string(nil), req.Args...),
		Status: domain.JobRunning, StartedAt: startedAt, ExpiresAt: expires, LogPath: logPath,
	}
	fmt.Fprintf(logf, "=== rocket: job %s (%s %s, owner %s) started at %s ===\n",
		id, job.Kind, job.Name, owner, job.StartedAt.Format(time.RFC3339))
	jobCtx, cancel := context.WithCancel(a.ctx)
	jr := &jobRun{done: make(chan struct{}), cancel: cancel}
	a.mu.Lock()
	if err := a.saveJob(job); err != nil {
		a.mu.Unlock()
		logf.Close()
		cancel()
		a.d.Logs.CloseJob(p.Name, id)
		return domain.Job{}, err
	}
	a.jobs[id] = jr
	a.mu.Unlock()

	a.wg.Add(1)
	go a.execJob(jobCtx, p, job, needs, childEnv, logf, jr)
	a.watchJobTTL(job, jr)
	return job, nil
}

func (a *App) resolveJob(p *domain.Project, req JobRequest, owner string) (name, env string, steps []domain.Step, err error) {
	env = req.Env
	if req.Kind == domain.JobDeploy {
		if len(req.Profiles) > 0 {
			return "", "", nil, fmt.Errorf("%w: profiles select setup/pipeline prerequisites and are not supported for deploy jobs", ErrInvalid)
		}
		if env != "" && env != req.Name {
			return "", "", nil, fmt.Errorf("%w: deploy env %q must match target %q", ErrInvalid, env, req.Name)
		}
		env = req.Name
	} else if env == "" {
		env = p.DefaultEnv
	}
	if _, ok := p.Envs[env]; !ok {
		return "", "", nil, fmt.Errorf("%w: unknown env %q in project %s", ErrInvalid, env, p.Name)
	}
	switch req.Kind {
	case domain.JobSetup:
		if req.Name == "" {
			for _, n := range SetupSequence {
				if s, ok := p.Setup[n]; ok {
					steps = append(steps, s)
				}
			}
			if len(steps) == 0 {
				return "", "", nil, fmt.Errorf("%w: project %s defines none of setup.%s in rocket.yaml",
					ErrInvalid, p.Name, strings.Join(SetupSequence, "/"))
			}
			return "setup", env, steps, nil
		}
		s, ok := p.Setup[req.Name]
		if !ok {
			return "", "", nil, fmt.Errorf("%w: project %s has no setup.%s (defined: %s)", ErrInvalid, p.Name, req.Name, keys(p.Setup))
		}
		return req.Name, env, []domain.Step{s}, nil
	case domain.JobPipeline:
		if req.Name == "" {
			return "", "", nil, fmt.Errorf("%w: pipeline name is required", ErrInvalid)
		}
		if s, ok := p.Pipelines[req.Name]; ok {
			return req.Name, env, s, nil
		}
		if a.d.Tasks.Taskfile(p.Root) == "" {
			return "", "", nil, fmt.Errorf("%w: project %s has no pipeline %q and no Taskfile to fall back to (pipelines: %s)",
				ErrInvalid, p.Name, req.Name, keys(p.Pipelines))
		}
		return req.Name, env, []domain.Step{{Task: req.Name}}, nil
	case domain.JobDeploy:
		e, ok := p.Envs[req.Name]
		if !ok {
			return "", "", nil, fmt.Errorf("%w: unknown env %q in project %s", ErrInvalid, req.Name, p.Name)
		}
		if e.Deploy == nil {
			return "", "", nil, fmt.Errorf("%w: env %q of project %s has no deploy", ErrInvalid, req.Name, p.Name)
		}
		if e.Deploy.Confirm && !req.Yes {
			return "", "", nil, fmt.Errorf("%w: deploying %s to %s needs --yes", ErrConfirmation, p.Name, req.Name)
		}
		if domain.IsAgentOwner(owner) && !(req.Yes && req.AllowAgentDeploy) {
			return "", "", nil, fmt.Errorf("%w: owner %s is an agent; agents may deploy only with --yes and ROCKET_ALLOW_DEPLOY=1 set by a human",
				ErrConfirmation, owner)
		}
		return req.Name, req.Name, []domain.Step{e.Deploy.Step()}, nil
	default:
		return "", "", nil, fmt.Errorf("%w: unknown job kind %q (setup, pipeline or deploy)", ErrInvalid, req.Kind)
	}
}

func keys[V any](m map[string]V) string {
	if len(m) == 0 {
		return "none"
	}
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return strings.Join(out, ", ")
}

func (a *App) stepArgv(s domain.Step, args []string) []string {
	if s.Task != "" {
		return a.d.Tasks.Argv(s.Task, args...)
	}
	if len(args) == 0 {
		return []string{"/bin/sh", "-c", s.Run}
	}
	// "$@" keeps every argument intact without shell quoting.
	return append([]string{"/bin/sh", "-c", s.Run + ` "$@"`, "rocket"}, args...)
}

func (a *App) execJob(ctx context.Context, p *domain.Project, job domain.Job, needs []string, env []string, logf *os.File, jr *jobRun) {
	defer a.wg.Done()
	defer close(jr.done)
	defer jr.cancel()

	status := domain.JobSucceeded
	var code *int
	var failure string
	if len(needs) > 0 {
		fmt.Fprintf(logf, "=== rocket: prerequisites: %s ===\n", strings.Join(needs, ", "))
		result, err := a.up(ctx, UpRequest{Project: p.Root, Services: needs, Env: job.Env, Profiles: job.Profiles, Owner: job.Owner}, job.ExpiresAt)
		if a.ctx.Err() != nil {
			logf.Close()
			return
		}
		if err != nil {
			status, failure = domain.JobFailed, fmt.Sprintf("prerequisite startup: %v", err)
		} else if result.Failed() {
			var problems []string
			for _, service := range result.Services {
				if service.Action == ActionFailed || service.Action == ActionSkipped {
					problems = append(problems, fmt.Sprintf("%s: %s", service.Service, service.Error))
				}
			}
			status, failure = domain.JobFailed, "prerequisite startup failed: "+strings.Join(problems, "; ")
		}
		a.mu.Lock()
		a.expireJobRunLocked(job, jr)
		if jr.canceled {
			status, failure = domain.JobCanceled, jr.reason
		}
		a.mu.Unlock()
	}
	for i, step := range job.Steps {
		if status != domain.JobSucceeded {
			break
		}
		var args []string
		if i == len(job.Steps)-1 {
			args = job.Args
		}
		fmt.Fprintf(logf, "=== rocket: step %d/%d: %s ===\n", i+1, len(job.Steps), step.Describe())

		// Start under the lock so a concurrent cancel either sees this
		// step's pgid or prevents it from starting.
		a.mu.Lock()
		a.expireJobRunLocked(job, jr)
		if jr.canceled {
			failure = jr.reason
			a.mu.Unlock()
			status = domain.JobCanceled
			break
		}
		h, err := a.d.Runner.Start(ports.ProcessSpec{Argv: a.stepArgv(step, args), Dir: p.Root, Env: env, Output: logf})
		if err != nil {
			a.mu.Unlock()
			status, failure = domain.JobFailed, fmt.Sprintf("step %d (%s): %v", i+1, step.Describe(), err)
			fmt.Fprintf(logf, "rocket: %s\n", failure)
			break
		}
		jr.pgid = h.PGID
		job.Step, job.PID, job.PGID = i+1, h.PID, h.PGID
		_ = a.saveJob(job)
		a.mu.Unlock()

		var c int
		select {
		case c = <-h.Done:
		case <-ctx.Done():
			if a.ctx.Err() != nil {
				logf.Close()
				return
			}
			_ = a.stopJobProcess(jr, h.PGID)
			select {
			case c = <-h.Done:
			case <-a.ctx.Done():
				logf.Close()
				return
			}
		case <-a.ctx.Done():
			// Daemon shutdown: the step keeps running and the next daemon
			// marks the job lost once its process group is gone.
			logf.Close()
			return
		}
		code = &c
		a.mu.Lock()
		jr.pgid = 0
		a.expireJobRunLocked(job, jr)
		canceled := jr.canceled
		reason := jr.reason
		a.mu.Unlock()
		if canceled {
			status, failure = domain.JobCanceled, reason
			break
		}
		if c != 0 {
			status, failure = domain.JobFailed, fmt.Sprintf("step %d (%s) exited with code %d", i+1, step.Describe(), c)
			break
		}
	}
	a.mu.Lock()
	a.expireJobRunLocked(job, jr)
	if jr.canceled {
		status, failure = domain.JobCanceled, jr.reason
	}
	a.mu.Unlock()
	fmt.Fprintf(logf, "\n=== rocket: job %s %s ===\n", job.ID, status)
	logf.Close()
	// Flush remaining job.log events before the terminal job.state.
	a.d.Logs.CloseJob(job.Project, job.ID)

	a.mu.Lock()
	defer a.mu.Unlock()
	delete(a.jobs, job.ID)
	a.expireJobRunLocked(job, jr)
	if jr.canceled {
		status, failure = domain.JobCanceled, jr.reason
	}
	a.finishJobLocked(job, status, code, failure)
}

// Check the persisted deadline at transitions as well as on the timer, so
// delayed scheduling cannot start another step or record a late success.
func (a *App) expireJobRunLocked(job domain.Job, jr *jobRun) {
	if !jr.canceled && job.Expired(a.now()) {
		jr.canceled, jr.reason = true, "ttl expired"
		if jr.cancel != nil {
			jr.cancel()
		}
	}
}

func (a *App) finishJobLocked(job domain.Job, status domain.JobStatus, code *int, failure string) domain.Job {
	if current, ok, _ := a.d.Store.GetJob(job.ID); ok && current.Status.Terminal() {
		return current // a concurrent cancellation/reconciliation already settled it
	}
	t := a.now()
	if status != domain.JobCanceled && job.Expired(t) {
		status, failure = domain.JobCanceled, "ttl expired"
	}
	job.Status, job.ExitCode, job.Error = status, code, failure
	job.FinishedAt = &t
	job.DurationMS = t.Sub(job.StartedAt).Milliseconds()
	_ = a.saveJob(job)
	return job
}

func (a *App) saveJob(j domain.Job) error {
	if err := a.d.Store.SaveJob(j); err != nil {
		return err
	}
	if a.d.Bus != nil {
		job := j
		a.d.Bus.Publish(domain.Event{Type: domain.EventJobState, Time: a.now(), Project: j.Project, JobID: j.ID, Status: j.Status, Job: &job})
	}
	return nil
}

// GetJob returns one job; running jobs report their elapsed duration.
func (a *App) GetJob(id string) (domain.Job, error) {
	j, ok, err := a.d.Store.GetJob(id)
	if err != nil {
		return j, err
	}
	if !ok {
		return j, fmt.Errorf("%w: job %q", ErrNotFound, id)
	}
	if j.Status == domain.JobRunning {
		j.DurationMS = a.now().Sub(j.StartedAt).Milliseconds()
	}
	return j, nil
}

// WaitJob blocks until the job is terminal or ctx ends.
func (a *App) WaitJob(ctx context.Context, id string) (domain.Job, error) {
	for {
		j, err := a.GetJob(id)
		if err != nil || j.Status.Terminal() {
			return j, err
		}
		a.mu.Lock()
		var done chan struct{}
		if jr := a.jobs[id]; jr != nil {
			done = jr.done
		}
		a.mu.Unlock()
		select {
		case <-done:
		case <-time.After(250 * time.Millisecond):
		case <-ctx.Done():
			return j, ctx.Err()
		}
	}
}

// ListJobs returns jobs newest first.
func (a *App) ListJobs(req JobsRequest) (JobsResult, error) {
	project := ""
	if !req.AllProjects && req.Project != "" {
		p, err := a.ResolveProject(req.Project)
		if err != nil {
			return JobsResult{}, err
		}
		project = p.Name
	}
	limit := req.Limit
	if limit <= 0 {
		limit = 50
	}
	jobs, err := a.d.Store.ListJobs(project, limit)
	if err != nil {
		return JobsResult{}, err
	}
	now := a.now()
	for i := range jobs {
		if jobs[i].Status == domain.JobRunning {
			jobs[i].DurationMS = now.Sub(jobs[i].StartedAt).Milliseconds()
		}
	}
	return JobsResult{Jobs: jobs}, nil
}

// JobLogs returns the last n lines of a job log (default 100).
func (a *App) JobLogs(id string, n int) (JobLogsResult, error) {
	j, err := a.GetJob(id)
	if err != nil {
		return JobLogsResult{}, err
	}
	if n <= 0 {
		n = 100
	}
	res := JobLogsResult{Job: j.ID, Project: j.Project, Lines: []string{}}
	if lines, err := a.d.Logs.TailJob(j.Project, j.ID, n); err == nil {
		res.Lines = append(res.Lines, lines...)
	}
	return res, nil
}

// CancelJob stops the job's current process group and waits for the job to
// settle. Cancelling a finished job is a no-op.
func (a *App) CancelJob(ctx context.Context, id string) (domain.Job, error) {
	return a.cancelJob(ctx, id, "canceled")
}

func (a *App) cancelJob(ctx context.Context, id, reason string) (domain.Job, error) {
	j, err := a.GetJob(id)
	if err != nil || j.Status.Terminal() {
		return j, err
	}
	a.mu.Lock()
	if current, ok, _ := a.d.Store.GetJob(id); ok {
		j = current
		if j.Status.Terminal() {
			a.mu.Unlock()
			return j, nil
		}
	}
	if j.Expired(a.now()) {
		reason = "ttl expired"
	}
	jr := a.jobs[id]
	if jr == nil {
		jr = &jobRun{canceled: true, reason: reason, pgid: j.PGID, done: make(chan struct{})}
		a.jobs[id] = jr
		a.mu.Unlock()
		if j.PGID > 0 && a.d.Runner.Alive(j.PID, j.PGID) {
			if err := a.stopJobProcess(jr, j.PGID); err != nil {
				a.mu.Lock()
				delete(a.jobs, id)
				a.mu.Unlock()
				close(jr.done)
				return j, err
			}
		}
		a.d.Logs.CloseJob(j.Project, j.ID)
		a.mu.Lock()
		delete(a.jobs, id)
		final := a.finishJobLocked(j, domain.JobCanceled, nil, reason)
		a.mu.Unlock()
		close(jr.done)
		return final, nil
	}
	if !jr.canceled {
		jr.canceled, jr.reason = true, reason
	}
	if jr.cancel != nil {
		jr.cancel()
	}
	pgid := jr.pgid
	a.mu.Unlock()
	if pgid > 0 {
		if err := a.stopJobProcess(jr, pgid); err != nil {
			return j, err
		}
	}
	select {
	case <-jr.done:
	case <-ctx.Done():
		return j, ctx.Err()
	case <-time.After(a.d.StopGrace + a.d.PollInterval + 5*time.Second):
		return j, errors.New("timed out waiting for job cancellation")
	}
	return a.GetJob(id)
}

// stopJobProcess is shared by cancellation and the step loop. The cleanup
// context is independent of the expired job/request context and signals once.
func (a *App) stopJobProcess(jr *jobRun, pgid int) error {
	jr.stopOnce.Do(func() {
		cleanup, cancel := context.WithTimeout(context.Background(), a.d.StopGrace+5*time.Second)
		defer cancel()
		jr.stopErr = a.d.Runner.Stop(cleanup, pgid, a.d.StopGrace)
	})
	return jr.stopErr
}

func (a *App) watchJobTTL(job domain.Job, jr *jobRun) {
	if job.ExpiresAt == nil {
		return
	}
	remaining := job.ExpiresAt.Sub(a.now())
	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		timer := time.NewTimer(max(remaining, 0))
		defer timer.Stop()
		select {
		case <-a.ctx.Done():
			return // daemon shutdown retains child jobs for adoption
		case <-jr.done:
			return
		case <-timer.C:
			if a.ctx.Err() != nil {
				return
			}
			cleanup, cancel := context.WithTimeout(context.Background(), a.d.StopGrace+a.d.PollInterval+5*time.Second)
			defer cancel()
			_, _ = a.cancelJob(cleanup, job.ID, "ttl expired")
		}
	}()
}

// ExpireJobsTTL cancels expired jobs without acquiring any project gate.
func (a *App) ExpireJobsTTL(ctx context.Context) ([]domain.Job, error) {
	jobs, err := a.d.Store.ListJobs("", 0)
	if err != nil {
		return nil, err
	}
	var expired []domain.Job
	for _, job := range jobs {
		if job.Status != domain.JobRunning || !job.Expired(a.now()) {
			continue
		}
		final, err := a.cancelJob(ctx, job.ID, "ttl expired")
		if err != nil {
			return expired, err
		}
		if final.Status == domain.JobCanceled && final.Error == "ttl expired" {
			expired = append(expired, final)
		}
	}
	return expired, nil
}

// cancelJobs cancels running jobs of project ("" = all) and owner ("" = any).
func (a *App) cancelJobs(ctx context.Context, project, owner string) []string {
	jobs, err := a.d.Store.ListJobs(project, 0)
	if err != nil {
		return nil
	}
	var out []string
	for _, j := range jobs {
		if j.Status != domain.JobRunning || (owner != "" && j.Owner != owner) {
			continue
		}
		if _, err := a.CancelJob(ctx, j.ID); err == nil {
			out = append(out, j.ID)
		}
	}
	return out
}

// reconcileJobs adopts running jobs whose process group is still alive and
// marks the rest lost.
func (a *App) reconcileJobs(ctx context.Context) (lost, expired []string, err error) {
	jobs, err := a.d.Store.ListJobs("", 0)
	if err != nil {
		return nil, nil, err
	}
	lost = []string{}
	for _, j := range jobs {
		if j.Status != domain.JobRunning {
			continue
		}
		if j.Expired(a.now()) {
			if _, err := a.cancelJob(ctx, j.ID, "ttl expired"); err != nil {
				return lost, expired, err
			}
			expired = append(expired, j.ID)
			continue
		}
		a.mu.Lock()
		if _, supervised := a.jobs[j.ID]; supervised {
			a.mu.Unlock()
			continue
		}
		if j.PID > 0 && a.d.Runner.Alive(j.PID, j.PGID) {
			a.adoptJobLocked(j)
		} else {
			final := a.finishJobLocked(j, domain.JobLost, nil, "its process was gone when the daemon (re)started; exit code unknown")
			if final.Status == domain.JobLost {
				lost = append(lost, j.ID)
			} else if final.Status == domain.JobCanceled && final.Error == "ttl expired" {
				expired = append(expired, j.ID)
			}
		}
		a.mu.Unlock()
	}
	return lost, expired, nil
}

// adoptJobLocked polls a job step started by a previous daemon. Its exit code
// cannot be observed, so the job ends as lost (or canceled). Must hold a.mu.
func (a *App) adoptJobLocked(j domain.Job) {
	jr := &jobRun{pgid: j.PGID, done: make(chan struct{})}
	a.jobs[j.ID] = jr
	a.watchJobTTL(j, jr)
	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		defer close(jr.done)
		t := time.NewTicker(a.d.PollInterval)
		defer t.Stop()
		for {
			select {
			case <-a.ctx.Done():
				return
			case <-t.C:
				if a.d.Runner.Alive(j.PID, j.PGID) {
					continue
				}
				a.d.Logs.CloseJob(j.Project, j.ID)
				a.mu.Lock()
				delete(a.jobs, j.ID)
				if jr.canceled {
					a.finishJobLocked(j, domain.JobCanceled, nil, jr.reason)
				} else {
					a.finishJobLocked(j, domain.JobLost, nil,
						"the daemon restarted while this job ran; exit code unknown and later steps did not run")
				}
				a.mu.Unlock()
				return
			}
		}
	}()
}

// pruneJobs deletes finished jobs beyond the newest KeepJobs per project.
func (a *App) pruneJobs() []GCAction {
	jobs, err := a.d.Store.ListJobs("", 0)
	if err != nil {
		return nil
	}
	var out []GCAction
	seen := map[string]int{}
	for _, j := range jobs {
		if !j.Status.Terminal() {
			continue
		}
		seen[j.Project]++
		if seen[j.Project] <= KeepJobs {
			continue
		}
		_ = a.d.Logs.RemoveJob(j.Project, j.ID)
		if err := a.d.Store.DeleteJob(j.ID); err == nil {
			out = append(out, GCAction{Action: "pruned_job", Project: j.Project, Service: j.Name, Detail: j.ID})
		}
	}
	return out
}
