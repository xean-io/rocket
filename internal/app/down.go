package app

import (
	"context"
	"fmt"
	"io"
	"sort"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// DownRequest selects running services to stop.
type DownRequest struct {
	Project    string   `json:"project,omitempty"`    // path or name; ignored with everywhere
	Services   []string `json:"services,omitempty"`   // services or groups; empty = whole project
	Owner      string   `json:"owner,omitempty"`      // only runs started by this owner
	Everywhere bool     `json:"everywhere,omitempty"` // all projects
}

// DownResult lists what was stopped.
type DownResult struct {
	Stopped     []domain.Run `json:"stopped"`
	ComposeDown []string     `json:"compose_down,omitempty"` // compose projects taken down
	// CanceledJobs lists running jobs canceled by a whole-project, owner
	// or everywhere down.
	CanceledJobs []string `json:"canceled_jobs,omitempty"`
	Errors       []string `json:"errors,omitempty"`
}

// Down stops services in reverse dependency order.
func (a *App) Down(ctx context.Context, req DownRequest) (DownResult, error) {
	res := DownResult{Stopped: []domain.Run{}}
	projects := map[string]*domain.Project{}
	var targets map[string]bool
	jobProject := ""
	if !req.Everywhere {
		p, err := a.ResolveProject(req.Project)
		if err != nil {
			return res, err
		}
		projects[p.Name] = p
		jobProject = p.Name
		if len(req.Services) > 0 {
			names, err := p.ExpandTargets(req.Services)
			if err != nil {
				return res, fmt.Errorf("%w: %v", ErrInvalid, err)
			}
			targets = map[string]bool{}
			for _, n := range names {
				targets[n] = true
			}
		}
	} else if len(req.Services) > 0 {
		return res, fmt.Errorf("%w: services cannot be combined with --everywhere", ErrInvalid)
	}
	// Cancel prerequisite Up operations before taking the service snapshot or
	// waiting for their project gate. Their partial startups must unwind first.
	if len(req.Services) == 0 {
		res.CanceledJobs = a.cancelJobs(ctx, jobProject, req.Owner)
	}
	wholeProject := len(req.Services) == 0 && req.Owner == ""

	runs, err := a.d.Store.ListRuns()
	if err != nil {
		return res, err
	}
	byProject := map[string][]domain.Run{}
	for _, r := range runs {
		if !r.State.Active() && !(wholeProject && r.State == domain.StateFailed && r.Kind == domain.KindCompose && r.ComposeProject != "") {
			continue
		}
		if !req.Everywhere && projects[r.Project] == nil {
			continue
		}
		if targets != nil && !targets[r.Service] {
			continue
		}
		if req.Owner != "" && r.Owner != req.Owner {
			continue
		}
		byProject[r.Project] = append(byProject[r.Project], r)
	}
	names := make([]string, 0, len(byProject))
	for n := range byProject {
		names = append(names, n)
	}
	sort.Strings(names)

	for _, name := range names {
		p := projects[name]
		if p == nil {
			p, _ = a.ResolveProject(name)
		}
		stopped, composeDown, errs := a.stopProjectRuns(ctx, p, byProject[name], wholeProject)
		res.Stopped = append(res.Stopped, stopped...)
		res.ComposeDown = append(res.ComposeDown, composeDown...)
		res.Errors = append(res.Errors, errs...)
	}
	return res, nil
}

func (a *App) stopProjectRuns(ctx context.Context, p *domain.Project, runs []domain.Run, composeDown bool) ([]domain.Run, []string, []string) {
	if len(runs) == 0 {
		return nil, nil, nil
	}
	lock := a.projectLock(runs[0].Project)
	lock.Lock()
	defer lock.Unlock()

	byService := map[string]domain.Run{}
	var services []string
	for _, r := range runs {
		byService[r.Service] = r
		services = append(services, r.Service)
	}
	if p != nil {
		services = p.StopOrder(services)
	} else {
		sort.Strings(services)
	}
	var stopped []domain.Run
	var errs []string
	composeTargets := map[string]ports.ComposeTarget{}
	for _, s := range services {
		r := byService[s]
		out, target, err := a.stopRun(ctx, p, r)
		if err != nil {
			errs = append(errs, fmt.Sprintf("%s/%s: %v", r.Project, r.Service, err))
		}
		if r.Kind == domain.KindCompose {
			if existing, ok := composeTargets[target.ProjectName]; ok {
				target.Profiles = domain.MergeProfiles(existing.Profiles, target.Profiles)
			}
			composeTargets[target.ProjectName] = target
		}
		stopped = append(stopped, out)
	}
	var downed []string
	if composeDown {
		names := make([]string, 0, len(composeTargets))
		for n := range composeTargets {
			names = append(names, n)
		}
		sort.Strings(names)
		for _, n := range names {
			if err := a.d.Compose.Down(ctx, composeTargets[n], io.Discard); err != nil {
				errs = append(errs, fmt.Sprintf("compose down %s: %v", n, err))
				continue
			}
			downed = append(downed, n)
		}
	}
	return stopped, downed, errs
}

// stopRun stops one run and releases its leases. Caller holds the project lock.
func (a *App) stopRun(ctx context.Context, p *domain.Project, r domain.Run) (domain.Run, ports.ComposeTarget, error) {
	a.mu.Lock()
	r.State = domain.StateStopping
	_ = a.saveRun(r)
	a.mu.Unlock()

	var target ports.ComposeTarget
	var stopErr error
	switch r.Kind {
	case domain.KindCompose:
		target = a.composeTargetForRun(p, r)
		svcName := r.Service
		if p != nil {
			if svc, ok := p.Services[r.Service]; ok {
				svcName = svc.Compose
			}
		}
		out := io.Writer(io.Discard)
		if f, _, err := a.d.Logs.Open(r.Project, r.Service); err == nil {
			defer f.Close()
			out = f
		}
		stopErr = a.d.Compose.Stop(ctx, target, svcName, out)
	default:
		if r.PGID > 0 {
			stopErr = a.d.Runner.Stop(ctx, r.PGID, a.d.StopGrace)
		}
	}

	a.mu.Lock()
	defer a.mu.Unlock()
	if cur, ok, _ := a.d.Store.GetRun(r.Project, r.Service); ok && cur.PID == r.PID {
		r.ExitCode = cur.ExitCode
	}
	r.State = domain.StateStopped
	r.Health = domain.HealthUnknown
	t := a.now()
	r.StoppedAt = &t
	if stopErr != nil {
		r.Error = "stop: " + stopErr.Error()
	}
	_ = a.saveRun(r)
	delete(a.watches, key(r.Project, r.Service))
	a.releaseLeases(r.Project, r.Service)
	return r, target, stopErr
}

func (a *App) composeTargetForRun(p *domain.Project, r domain.Run) ports.ComposeTarget {
	if p != nil {
		if svc, ok := p.Services[r.Service]; ok {
			if _, ok := p.Envs[r.Env]; ok {
				env, err := a.serviceEnv(p, r.Env, svc, r.Ports)
				if err != nil {
					env = a.d.BaseEnv()
				}
				target := a.composeTarget(p, r.Env, svc, env, nil)
				if r.Profiles != nil {
					target.Profiles = domain.MergeProfiles(r.Profiles)
				}
				return target
			}
		}
	}
	return ports.ComposeTarget{ProjectName: r.ComposeProject, Env: a.d.BaseEnv(), Profiles: domain.MergeProfiles(r.Profiles)}
}
