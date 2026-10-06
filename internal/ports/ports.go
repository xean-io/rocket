// Package ports declares the interfaces the application core depends on.
// Adapters in internal/adapters implement them.
package ports

import (
	"context"
	"errors"
	"io"
	"os"
	"time"

	"github.com/xean-io/rocket/internal/domain"
)

// ErrUnsupported is returned by adapters that are not implemented on the
// current platform.
var ErrUnsupported = errors.New("not supported on this platform")

// ErrLeaseTaken is returned when a port is leased by another service.
var ErrLeaseTaken = errors.New("port already leased")

// ManifestLoader loads a project from its root directory.
type ManifestLoader interface {
	Load(dir string) (*domain.Project, error)
}

// EnvSource reads dotenv files and the daemon's base environment.
type EnvSource interface {
	Dotenv(root string, files []string) (map[string]string, error)
}

// Store persists the project registry, runs and port leases.
type Store interface {
	UpsertProject(domain.ProjectRef) error
	GetProject(name string) (domain.ProjectRef, bool, error)
	ListProjects() ([]domain.ProjectRef, error)
	DeleteProject(name string) error

	SaveRun(domain.Run) error
	GetRun(project, service string) (domain.Run, bool, error)
	ListRuns() ([]domain.Run, error)
	DeleteRun(project, service string) error

	// AcquireLease stores a lease; it returns ErrLeaseTaken when the port is
	// leased by a different project/service.
	AcquireLease(domain.Lease) error
	ReleaseLeases(project, service string) ([]domain.Lease, error)
	ListLeases() ([]domain.Lease, error)

	SaveJob(domain.Job) error
	GetJob(id string) (domain.Job, bool, error)
	// ListJobs returns jobs newest first; an empty project means every
	// project and limit <= 0 means no limit.
	ListJobs(project string, limit int) ([]domain.Job, error)
	DeleteJob(id string) error
}

// ProcessSpec describes a local process to start in its own process group.
type ProcessSpec struct {
	Argv   []string
	Dir    string
	Env    []string
	Output *os.File // receives stdout and stderr
}

// ProcessHandle identifies a started process. Done yields the exit code once.
type ProcessHandle struct {
	PID  int
	PGID int
	Done <-chan int
}

// ProcessRunner starts and stops process groups.
type ProcessRunner interface {
	Start(ProcessSpec) (ProcessHandle, error)
	// Stop sends SIGTERM to the group, waits up to grace, then SIGKILL.
	Stop(ctx context.Context, pgid int, grace time.Duration) error
	// Alive reports whether pid is running and still leads/belongs to pgid.
	Alive(pid, pgid int) bool
}

// ComposeTarget addresses one compose project (rocket project + env).
type ComposeTarget struct {
	ProjectName string
	Dir         string
	Files       []string
	Profiles    []string
	Env         []string
}

// ComposeDriver wraps `docker compose`.
type ComposeDriver interface {
	// NamedVolumes returns nonexternal named volumes mounted by one service,
	// after Compose has normalized names and interpolated the target environment.
	NamedVolumes(ctx context.Context, t ComposeTarget, service string) ([]string, error)
	// VolumeExists uses a successful Docker inventory to establish exact-name
	// presence. Errors provide no evidence that a volume is absent.
	VolumeExists(ctx context.Context, t ComposeTarget, name string) (bool, error)
	Up(ctx context.Context, t ComposeTarget, service string, out io.Writer) (containerID string, err error)
	Stop(ctx context.Context, t ComposeTarget, service string, out io.Writer) error
	Down(ctx context.Context, t ComposeTarget, out io.Writer) error
	Running(ctx context.Context, composeProject, service string) (bool, error)
	Logs(ctx context.Context, t ComposeTarget, service string, tail int) ([]string, error)
}

// TaskDriver builds argv for go-task invocations.
type TaskDriver interface {
	Argv(task string, args ...string) []string
	// Taskfile returns the Taskfile path in dir, or "" when there is none.
	Taskfile(dir string) string
}

// PortProbe inspects the host's TCP ports.
type PortProbe interface {
	Free(port int) bool
	Holder(port int) (*domain.PortHolder, error)
}

// HealthProbe performs a single readiness probe.
type HealthProbe interface {
	Check(ctx context.Context, c domain.HealthCheck) error
}

// EventBus fans out daemon events to subscribers.
type EventBus interface {
	Publish(domain.Event)
	Subscribe(buffer int) (<-chan domain.Event, func())
}

// LogSink owns per-service log files and their in-memory tail.
type LogSink interface {
	// Open returns an append-mode file for the child's stdout/stderr and
	// starts following it for tail and event publishing.
	Open(project, service string) (*os.File, string, error)
	Tail(project, service string, n int) ([]string, error)

	// OpenJob is Open for a job log; new lines are published as job.log.
	OpenJob(project, jobID string) (*os.File, string, error)
	TailJob(project, jobID string, n int) ([]string, error)
	// CloseJob publishes the remaining lines and stops following the log.
	CloseJob(project, jobID string)
	RemoveJob(project, jobID string) error
}
