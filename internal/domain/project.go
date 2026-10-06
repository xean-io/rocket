// Package domain holds rocket's core model: projects, services, runs and
// port leases. It is pure: no IO, no clocks, no processes.
package domain

import "time"

// ServiceKind tells which adapter supervises a service.
type ServiceKind string

const (
	KindCompose ServiceKind = "compose"
	KindTask    ServiceKind = "task"
	KindRun     ServiceKind = "run"
)

// DefaultOwner is used when neither --owner nor ROCKET_OWNER is set.
const DefaultOwner = "user"

// DefaultHealthTimeout bounds how long `up` waits for a service to be healthy.
const DefaultHealthTimeout = 60 * time.Second

// PortSpec is a named port a service listens on.
type PortSpec struct {
	Name        string                    `json:"name"`
	Default     int                       `json:"default"`
	Env         string                    `json:"env,omitempty"` // scalar or first unconditional binding, for remap reporting
	EnvBindings map[string]PortEnvBinding `json:"-"`
	Probe       *bool                     `json:"probe,omitempty"` // nil defaults to enabled
}

// ProbeEnabled reports whether Rocket should use this port for readiness.
// Disabled ports still reserve a lease and permit remapping.
func (p PortSpec) ProbeEnabled() bool { return p.Probe == nil || *p.Probe }

// PortEnvBinding injects the resolved port into a template. Default bindings
// preserve nonempty values from the inherited, dotenv or service environment.
type PortEnvBinding struct {
	Template string
	Default  bool
}

// HealthSpec describes how to decide a service is ready.
type HealthSpec struct {
	HTTP    string        `json:"http,omitempty"` // path probed on the health port
	TCP     bool          `json:"tcp,omitempty"`
	Port    string        `json:"port,omitempty"` // port name; defaults to the first eligible port
	Timeout time.Duration `json:"timeout,omitempty"`
}

// Service is one supervised unit of a project.
type Service struct {
	Name      string            `json:"name"`
	Kind      ServiceKind       `json:"kind"`
	Compose   string            `json:"compose,omitempty"`
	Task      string            `json:"task,omitempty"`
	Run       string            `json:"run,omitempty"`
	Cwd       string            `json:"cwd,omitempty"`
	Env       map[string]string `json:"env,omitempty"`
	Dotenv    []string          `json:"dotenv,omitempty"`
	Profiles  []string          `json:"profiles,omitempty"`
	DependsOn []string          `json:"depends_on,omitempty"`
	Ports     []PortSpec        `json:"ports,omitempty"` // sorted by name
	Health    *HealthSpec       `json:"health,omitempty"`
}

// Step is one unit of a pipeline or setup action.
type Step struct {
	Task string `json:"task,omitempty"`
	Run  string `json:"run,omitempty"`
}

// Deploy is an environment's deploy action (exactly one of Task or Run).
type Deploy struct {
	Task    string `json:"task,omitempty"`
	Run     string `json:"run,omitempty"`
	Confirm bool   `json:"confirm"`
}

// Step returns the deploy action as a job step.
func (d Deploy) Step() Step { return Step{Task: d.Task, Run: d.Run} }

// Environment selects compose files/profiles (dev, smoke…) or a deploy target.
type Environment struct {
	Name     string   `json:"name"`
	Compose  []string `json:"compose,omitempty"`
	Profiles []string `json:"profiles,omitempty"`
	Deploy   *Deploy  `json:"deploy,omitempty"`
}

// Project is a loaded rocket.yaml.
type Project struct {
	Name          string                 `json:"name"`
	Root          string                 `json:"root"`
	Dotenv        []string               `json:"dotenv,omitempty"`
	DefaultEnv    string                 `json:"default_env"`
	Setup         map[string]Step        `json:"setup,omitempty"`
	Envs          map[string]Environment `json:"envs,omitempty"`
	Services      map[string]Service     `json:"services"`
	Groups        map[string][]string    `json:"groups,omitempty"`
	Pipelines     map[string][]Step      `json:"pipelines,omitempty"`
	PipelineNeeds map[string][]string    `json:"-"` // prerequisite service/group targets per pipeline
}

// ProjectRef is an entry of the global project registry.
type ProjectRef struct {
	Name    string    `json:"name"`
	Path    string    `json:"path"`
	AddedAt time.Time `json:"added_at"`
}

// HealthCheck is a resolved, single probe target.
type HealthCheck struct {
	Kind string `json:"kind"` // "tcp" | "http"
	Port int    `json:"port"`
	Path string `json:"path,omitempty"`
}

// HealthCheckFor resolves the probe for a service given its leased ports.
// It returns nil when the service has nothing to probe.
func HealthCheckFor(svc Service, ports map[string]int) *HealthCheck {
	name := ""
	target := ""
	if svc.Health != nil && svc.Health.Port != "" {
		target = svc.Health.Port
	}
	for _, spec := range svc.Ports {
		if spec.ProbeEnabled() && (target == "" || spec.Name == target) {
			name = spec.Name
			break
		}
	}
	if name == "" {
		return nil
	}
	port := ports[name]
	if port == 0 {
		return nil
	}
	if svc.Health != nil && svc.Health.HTTP != "" {
		return &HealthCheck{Kind: "http", Port: port, Path: svc.Health.HTTP}
	}
	if svc.Kind == KindCompose && (svc.Health == nil || !svc.Health.TCP) {
		return nil // compose health is covered by `up --wait`
	}
	return &HealthCheck{Kind: "tcp", Port: port}
}

// HealthTimeout returns the configured or default readiness timeout.
func (s Service) HealthTimeout() time.Duration {
	if s.Health != nil && s.Health.Timeout > 0 {
		return s.Health.Timeout
	}
	return DefaultHealthTimeout
}
