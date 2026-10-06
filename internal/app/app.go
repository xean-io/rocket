// Package app implements rocket's use cases on top of the ports interfaces.
package app

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// Sentinel errors; the API adapter maps them to HTTP status codes.
var (
	ErrNotFound = errors.New("not found")
	ErrInvalid  = errors.New("invalid request")
	ErrConflict = errors.New("conflict")
)

// Deps wires adapters into the application.
type Deps struct {
	Store     ports.Store
	Manifests ports.ManifestLoader
	Env       ports.EnvSource
	Runner    ports.ProcessRunner
	Compose   ports.ComposeDriver
	Tasks     ports.TaskDriver
	Probe     ports.PortProbe
	Health    ports.HealthProbe
	Bus       ports.EventBus
	Logs      ports.LogSink

	Now     func() time.Time
	BaseEnv func() []string
	NewID   func() string // job ids

	StopGrace      time.Duration // SIGTERM -> SIGKILL grace (default 10s)
	HealthInterval time.Duration // between readiness probes (default 250ms)
	PollInterval   time.Duration // liveness polling of adopted processes (default 2s)
	StartGrace     time.Duration // wait for early exit of probe-less processes (default 500ms)
}

// App is the rocket application core, hosted by the daemon.
type App struct {
	d Deps

	mu      sync.Mutex // guards store read-modify-write, watches and jobs
	watches map[string]*watch
	jobs    map[string]*jobRun

	locksMu sync.Mutex
	locks   map[string]*projectGate

	ctx    context.Context
	cancel context.CancelFunc
	wg     sync.WaitGroup
}

// watch tracks a process the daemon supervises.
type watch struct {
	pid    int
	exited chan struct{}
	code   int
}

// New builds the application with sane defaults for unset durations.
func New(d Deps) *App {
	if d.Now == nil {
		d.Now = time.Now
	}
	if d.BaseEnv == nil {
		d.BaseEnv = os.Environ
	}
	if d.NewID == nil {
		d.NewID = newJobID
	}
	if d.StopGrace == 0 {
		d.StopGrace = 10 * time.Second
	}
	if d.HealthInterval == 0 {
		d.HealthInterval = 250 * time.Millisecond
	}
	if d.PollInterval == 0 {
		d.PollInterval = 2 * time.Second
	}
	if d.StartGrace == 0 {
		d.StartGrace = 500 * time.Millisecond
	}
	ctx, cancel := context.WithCancel(context.Background())
	return &App{d: d, watches: map[string]*watch{}, jobs: map[string]*jobRun{}, locks: map[string]*projectGate{}, ctx: ctx, cancel: cancel}
}

// Close stops background watchers. Supervised processes keep running and
// are adopted by the next daemon through Reconcile.
func (a *App) Close() {
	a.cancel()
	a.wg.Wait()
}

func key(project, service string) string { return project + "/" + service }

// projectGate serializes project operations while allowing queued startup to
// be canceled without waiting for another operation to release the project.
type projectGate struct{ token chan struct{} }

func (g *projectGate) Lock()   { <-g.token }
func (g *projectGate) Unlock() { g.token <- struct{}{} }
func (g *projectGate) LockContext(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-g.token:
		if err := ctx.Err(); err != nil {
			g.Unlock()
			return err
		}
		return nil
	}
}

func (a *App) projectLock(name string) *projectGate {
	a.locksMu.Lock()
	defer a.locksMu.Unlock()
	l, ok := a.locks[name]
	if !ok {
		l = &projectGate{token: make(chan struct{}, 1)}
		l.Unlock()
		a.locks[name] = l
	}
	return l
}

func (a *App) now() time.Time { return a.d.Now().UTC() }

func (a *App) publishRun(r domain.Run) {
	if a.d.Bus == nil {
		return
	}
	run := r
	a.d.Bus.Publish(domain.Event{Type: domain.EventServiceState, Time: a.now(), Project: r.Project, Service: r.Service, State: r.State, Run: &run})
}

func (a *App) publishLease(typ string, l domain.Lease) {
	if a.d.Bus == nil {
		return
	}
	lease := l
	a.d.Bus.Publish(domain.Event{Type: typ, Time: a.now(), Project: l.Project, Service: l.Service, Lease: &lease})
}

func (a *App) saveRun(r domain.Run) error {
	if err := a.d.Store.SaveRun(r); err != nil {
		return err
	}
	a.publishRun(r)
	return nil
}

func (a *App) releaseLeases(project, service string) {
	released, _ := a.d.Store.ReleaseLeases(project, service)
	for _, l := range released {
		a.publishLease(domain.EventPortReleased, l)
	}
}

// ResolveProject loads a project by absolute path (registering it) or by
// registered name.
func (a *App) ResolveProject(ref string) (*domain.Project, error) {
	if ref == "" {
		return nil, fmt.Errorf("%w: project is required (run inside a directory with rocket.yaml or pass -p)", ErrInvalid)
	}
	if filepath.IsAbs(ref) {
		p, err := a.d.Manifests.Load(ref)
		if err != nil {
			return nil, fmt.Errorf("%w: %v", ErrInvalid, err)
		}
		if err := a.register(p); err != nil {
			return nil, err
		}
		return p, nil
	}
	ref0, ok, err := a.d.Store.GetProject(ref)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, fmt.Errorf("%w: project %q is not registered (rocket projects add <path>)", ErrNotFound, ref)
	}
	p, err := a.d.Manifests.Load(ref0.Path)
	if err != nil {
		return nil, fmt.Errorf("%w: %v", ErrInvalid, err)
	}
	return p, nil
}

func (a *App) register(p *domain.Project) error {
	existing, ok, err := a.d.Store.GetProject(p.Name)
	if err != nil {
		return err
	}
	if ok && existing.Path == p.Root {
		return nil
	}
	if ok {
		if _, err := a.d.Manifests.Load(existing.Path); err == nil {
			return fmt.Errorf("%w: project name %q is already registered at %s (rename it in rocket.yaml or `rocket projects rm %s`)",
				ErrConflict, p.Name, existing.Path, p.Name)
		}
	}
	return a.d.Store.UpsertProject(domain.ProjectRef{Name: p.Name, Path: p.Root, AddedAt: a.now()})
}

// AddProject registers the project at path.
func (a *App) AddProject(path string) (domain.ProjectRef, error) {
	if !filepath.IsAbs(path) {
		return domain.ProjectRef{}, fmt.Errorf("%w: project path must be absolute", ErrInvalid)
	}
	p, err := a.ResolveProject(path)
	if err != nil {
		return domain.ProjectRef{}, err
	}
	ref, _, err := a.d.Store.GetProject(p.Name)
	return ref, err
}

// ListProjects returns the registry.
func (a *App) ListProjects() ([]domain.ProjectRef, error) { return a.d.Store.ListProjects() }

// RemoveProject unregisters a project that has no active runs.
func (a *App) RemoveProject(name string) error {
	if _, ok, err := a.d.Store.GetProject(name); err != nil {
		return err
	} else if !ok {
		return fmt.Errorf("%w: project %q is not registered", ErrNotFound, name)
	}
	runs, err := a.d.Store.ListRuns()
	if err != nil {
		return err
	}
	for _, r := range runs {
		if r.Project == name && r.State.Active() {
			return fmt.Errorf("%w: project %q has running services; run `rocket down -p %s` first", ErrConflict, name, name)
		}
	}
	for _, r := range runs {
		if r.Project == name {
			_ = a.d.Store.DeleteRun(r.Project, r.Service)
		}
	}
	return a.d.Store.DeleteProject(name)
}
