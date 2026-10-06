package app

import (
	"context"
	"fmt"
	"sort"

	"github.com/xean-io/rocket/internal/domain"
)

// StatusRequest selects the services shown by `rocket ps`.
type StatusRequest struct {
	Project     string `json:"project,omitempty"`
	AllProjects bool   `json:"all_projects,omitempty"`
}

// StatusResult lists runs; declared-but-never-started services appear as
// "stopped" when a single project is requested.
type StatusResult struct {
	Services []domain.Run `json:"services"`
}

// Status returns current service states.
func (a *App) Status(ctx context.Context, req StatusRequest) (StatusResult, error) {
	a.refreshLiveness()
	runs, err := a.d.Store.ListRuns()
	if err != nil {
		return StatusResult{}, err
	}
	out := []domain.Run{}
	if req.AllProjects || req.Project == "" {
		out = append(out, runs...)
	} else {
		p, err := a.ResolveProject(req.Project)
		if err != nil {
			return StatusResult{}, err
		}
		have := map[string]bool{}
		for _, r := range runs {
			if r.Project == p.Name {
				out = append(out, r)
				have[r.Service] = true
			}
		}
		for _, name := range p.ServiceNames() {
			if !have[name] {
				out = append(out, domain.Run{Project: p.Name, Service: name, Env: p.DefaultEnv, Kind: p.Services[name].Kind,
					State: domain.StateStopped, Health: domain.HealthUnknown})
			}
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Project != out[j].Project {
			return out[i].Project < out[j].Project
		}
		return out[i].Service < out[j].Service
	})
	return StatusResult{Services: out}, nil
}

// refreshLiveness marks unsupervised local processes that vanished as dead.
func (a *App) refreshLiveness() {
	a.mu.Lock()
	defer a.mu.Unlock()
	runs, _ := a.d.Store.ListRuns()
	for _, r := range runs {
		if !r.State.Active() || r.Kind == domain.KindCompose || r.State == domain.StateStarting {
			continue
		}
		if _, supervised := a.watches[key(r.Project, r.Service)]; supervised {
			continue
		}
		if r.PID > 0 && a.d.Runner.Alive(r.PID, r.PGID) {
			continue
		}
		a.markDeadLocked(r)
	}
}

func (a *App) markDeadLocked(r domain.Run) domain.Run {
	r.State = domain.StateDead
	r.Health = domain.HealthUnknown
	t := a.now()
	r.StoppedAt = &t
	_ = a.saveRun(r)
	a.releaseLeases(r.Project, r.Service)
	return r
}

// LogsRequest asks for the tail of a service log.
type LogsRequest struct {
	Project string `json:"project"`
	Service string `json:"service"`
	Tail    int    `json:"tail"`
}

// LogsResult carries log lines, oldest first.
type LogsResult struct {
	Project string   `json:"project"`
	Service string   `json:"service"`
	Lines   []string `json:"lines"`
}

// Logs returns the last lines of a service's output.
func (a *App) Logs(ctx context.Context, req LogsRequest) (LogsResult, error) {
	p, err := a.ResolveProject(req.Project)
	if err != nil {
		return LogsResult{}, err
	}
	svc, ok := p.Services[req.Service]
	if !ok {
		return LogsResult{}, fmt.Errorf("%w: unknown service %q in project %s", ErrNotFound, req.Service, p.Name)
	}
	tail := req.Tail
	if tail <= 0 {
		tail = 100
	}
	res := LogsResult{Project: p.Name, Service: svc.Name, Lines: []string{}}
	if svc.Kind == domain.KindCompose {
		env := p.DefaultEnv
		var runProfiles []string
		if r, ok, _ := a.d.Store.GetRun(p.Name, svc.Name); ok && r.Env != "" {
			env = r.Env
			runProfiles = r.Profiles
		}
		target := a.composeTarget(p, env, svc, a.d.BaseEnv(), nil)
		if runProfiles != nil {
			target.Profiles = domain.MergeProfiles(runProfiles)
		}
		if lines, err := a.d.Compose.Logs(ctx, target, svc.Compose, tail); err == nil {
			res.Lines = append(res.Lines, lines...)
			return res, nil
		}
	}
	lines, err := a.d.Logs.Tail(p.Name, svc.Name, tail)
	if err != nil {
		return res, nil // no log yet
	}
	res.Lines = append(res.Lines, lines...)
	return res, nil
}

// PortInfo is one row of the global port map.
type PortInfo struct {
	domain.Lease
	Owner string          `json:"owner,omitempty"`
	State domain.RunState `json:"state,omitempty"`
	PID   int             `json:"pid,omitempty"`
	Env   string          `json:"env,omitempty"`
}

// PortsResult is the global port map across projects.
type PortsResult struct {
	Ports []PortInfo `json:"ports"`
}

// Ports lists every leased port with its owning run.
func (a *App) Ports() (PortsResult, error) {
	leases, err := a.d.Store.ListLeases()
	if err != nil {
		return PortsResult{}, err
	}
	out := PortsResult{Ports: []PortInfo{}}
	for _, l := range leases {
		info := PortInfo{Lease: l}
		if r, ok, _ := a.d.Store.GetRun(l.Project, l.Service); ok {
			info.Owner, info.State, info.PID, info.Env = r.Owner, r.State, r.PID, r.Env
		}
		out.Ports = append(out.Ports, info)
	}
	sort.Slice(out.Ports, func(i, j int) bool { return out.Ports[i].Port < out.Ports[j].Port })
	return out, nil
}
