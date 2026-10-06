package domain

import (
	"regexp"
	"sort"
	"strings"
	"time"
)

// RunState is the lifecycle state of a service instance.
type RunState string

const (
	StateStarting RunState = "starting"
	StateRunning  RunState = "running"
	StateStopping RunState = "stopping"
	StateStopped  RunState = "stopped"
	StateExited   RunState = "exited"
	StateFailed   RunState = "failed"
	StateDead     RunState = "dead" // found gone during reconcile
)

// Active reports whether the run may still own processes or ports.
func (s RunState) Active() bool {
	return s == StateStarting || s == StateRunning || s == StateStopping
}

// Health is the last known readiness of a run.
type Health string

const (
	HealthUnknown   Health = "unknown"
	HealthHealthy   Health = "healthy"
	HealthUnhealthy Health = "unhealthy"
)

// Run is the current (or last) instance of a service.
type Run struct {
	Project        string         `json:"project"`
	Service        string         `json:"service"`
	Env            string         `json:"env"`
	Kind           ServiceKind    `json:"kind"`
	State          RunState       `json:"state"`
	Health         Health         `json:"health"`
	PID            int            `json:"pid,omitempty"`
	PGID           int            `json:"pgid,omitempty"`
	ComposeProject string         `json:"compose_project,omitempty"`
	Profiles       []string       `json:"profiles,omitempty"` // effective Compose launch profiles; retained for stop
	ContainerID    string         `json:"container_id,omitempty"`
	Owner          string         `json:"owner,omitempty"`
	ExpiresAt      *time.Time     `json:"expires_at,omitempty"`
	StartedAt      *time.Time     `json:"started_at,omitempty"`
	StoppedAt      *time.Time     `json:"stopped_at,omitempty"`
	ExitCode       *int           `json:"exit_code,omitempty"`
	Ports          map[string]int `json:"ports,omitempty"`
	LogPath        string         `json:"log_path,omitempty"`
	Error          string         `json:"error,omitempty"`
}

// Expired reports whether the run's TTL has elapsed at now.
func (r Run) Expired(now time.Time) bool {
	return r.ExpiresAt != nil && !now.Before(*r.ExpiresAt)
}

// Lease reserves a host port for one service port across all projects.
type Lease struct {
	Port      int       `json:"port"`
	Project   string    `json:"project"`
	Service   string    `json:"service"`
	PortName  string    `json:"port_name"`
	CreatedAt time.Time `json:"created_at"`
}

// PortHolder describes a foreign process listening on a port.
type PortHolder struct {
	PID     int    `json:"pid"`
	Command string `json:"command"`
	Cwd     string `json:"cwd,omitempty"`
}

// PortRemap records an automatic port reassignment.
type PortRemap struct {
	Name   string      `json:"name"`
	From   int         `json:"from"`
	To     int         `json:"to"`
	Env    string      `json:"env"`
	Holder *PortHolder `json:"holder,omitempty"`
	Reason string      `json:"reason"`
}

// Event types published on the daemon event stream.
const (
	EventServiceState = "service.state"
	EventLogLine      = "log.line"
	EventPortLeased   = "port.leased"
	EventPortReleased = "port.released"
	EventJobState     = "job.state"
	EventJobLog       = "job.log"
)

// Event is a daemon notification (SSE payload).
type Event struct {
	Type    string    `json:"type"`
	Time    time.Time `json:"time"`
	Project string    `json:"project,omitempty"`
	Service string    `json:"service,omitempty"`
	State   RunState  `json:"state,omitempty"`
	Line    string    `json:"line,omitempty"`
	Run     *Run      `json:"run,omitempty"`
	Lease   *Lease    `json:"lease,omitempty"`
	JobID   string    `json:"job_id,omitempty"`
	Status  JobStatus `json:"status,omitempty"` // job.state
	Job     *Job      `json:"job,omitempty"`    // job.state
}

// RemapCandidates yields ports to try when the desired one is busy: first a
// +100 jump (3000 -> 3100), then +1 steps from there.
func RemapCandidates(port int) []int {
	const attempts = 200
	out := make([]int, 0, attempts)
	for i := 0; i < attempts; i++ {
		c := port + 100 + i
		if c > 65535 {
			c = 1024 + (c - 65535)
		}
		out = append(out, c)
	}
	return out
}

var composeNameSanitizer = regexp.MustCompile(`[^a-z0-9_-]+`)

// ComposeProjectName forces a unique compose project per rocket project/env.
func ComposeProjectName(project, env string) string {
	name := strings.ToLower("rocket-" + project + "-" + env)
	return strings.Trim(composeNameSanitizer.ReplaceAllString(name, "-"), "-")
}

// BuildEnv merges env layers for a child process. Later layers win.
// ROCKET_* variables from the base environment never leak into children.
func BuildEnv(base []string, layers ...map[string]string) []string {
	m := map[string]string{}
	for _, kv := range base {
		k, v, ok := strings.Cut(kv, "=")
		if !ok || strings.HasPrefix(k, "ROCKET_") {
			continue
		}
		m[k] = v
	}
	for _, l := range layers {
		for k, v := range l {
			m[k] = v
		}
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	out := make([]string, 0, len(keys))
	for _, k := range keys {
		out = append(out, k+"="+m[k])
	}
	return out
}
