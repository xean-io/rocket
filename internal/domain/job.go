package domain

import (
	"strings"
	"time"
)

// JobKind tells why a one-shot job runs.
type JobKind string

const (
	JobSetup    JobKind = "setup"
	JobPipeline JobKind = "pipeline"
	JobDeploy   JobKind = "deploy"
)

// JobStatus is the lifecycle state of a job.
type JobStatus string

const (
	JobRunning   JobStatus = "running"
	JobSucceeded JobStatus = "succeeded"
	JobFailed    JobStatus = "failed"
	JobCanceled  JobStatus = "canceled"
	JobLost      JobStatus = "lost" // its process vanished while no daemon watched it
)

// Terminal reports whether the job has finished one way or another.
func (s JobStatus) Terminal() bool { return s != JobRunning }

// Job is one execution of a pipeline, setup action or deploy, supervised by
// the daemon. Steps run sequentially and stop at the first failure.
type Job struct {
	ID         string     `json:"id"`
	Project    string     `json:"project"`
	Name       string     `json:"name"` // pipeline/task name, setup name or deploy env
	Kind       JobKind    `json:"kind"`
	Env        string     `json:"env,omitempty"`      // selected environment; deploy target for deploy jobs
	Profiles   []string   `json:"profiles,omitempty"` // effective env/request startup selection for prerequisites
	Owner      string     `json:"owner"`
	Steps      []Step     `json:"steps"`
	Args       []string   `json:"args,omitempty"` // appended to the last step
	Status     JobStatus  `json:"status"`
	Step       int        `json:"step,omitempty"` // 1-based index of the current/last step
	PID        int        `json:"pid,omitempty"`
	PGID       int        `json:"pgid,omitempty"`
	ExitCode   *int       `json:"exit_code,omitempty"`
	StartedAt  time.Time  `json:"started_at"`
	ExpiresAt  *time.Time `json:"expires_at,omitempty"`
	FinishedAt *time.Time `json:"finished_at,omitempty"`
	DurationMS int64      `json:"duration_ms"`
	LogPath    string     `json:"log_path,omitempty"`
	Error      string     `json:"error,omitempty"`
}

// Expired reports whether the persisted deadline has elapsed.
func (j Job) Expired(now time.Time) bool {
	return j.ExpiresAt != nil && !now.Before(*j.ExpiresAt)
}

// Describe renders a step for logs and CLI output.
func (s Step) Describe() string {
	if s.Task != "" {
		return "task " + s.Task
	}
	return "run " + s.Run
}

// IsAgentOwner reports whether owner identifies an AI agent ("agent:<id>").
func IsAgentOwner(owner string) bool { return strings.HasPrefix(owner, "agent:") }
