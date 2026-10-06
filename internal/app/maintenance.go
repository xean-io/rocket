package app

import (
	"context"
	"fmt"
	"sort"

	"github.com/xean-io/rocket/internal/domain"
)

// ReconcileResult reports what the daemon found on start.
type ReconcileResult struct {
	Adopted        []domain.Run   `json:"adopted"`
	Dead           []domain.Run   `json:"dead"`
	ReleasedLeases []domain.Lease `json:"released_leases"`
	LostJobs       []string       `json:"lost_jobs"`
	ExpiredJobs    []string       `json:"expired_jobs,omitempty"`
}

// Reconcile compares persisted runs with reality: live processes/containers
// are adopted, vanished ones are marked dead and their leases released.
func (a *App) Reconcile(ctx context.Context) (ReconcileResult, error) {
	res := ReconcileResult{Adopted: []domain.Run{}, Dead: []domain.Run{}, ReleasedLeases: []domain.Lease{}, LostJobs: []string{}}
	expiredJobs, err := a.ExpireJobsTTL(ctx)
	if err != nil {
		return res, err
	}
	for _, job := range expiredJobs {
		res.ExpiredJobs = append(res.ExpiredJobs, job.ID)
	}
	runs, err := a.d.Store.ListRuns()
	if err != nil {
		return res, err
	}
	for _, r := range runs {
		if !r.State.Active() {
			continue
		}
		a.mu.Lock()
		_, supervised := a.watches[key(r.Project, r.Service)]
		a.mu.Unlock()
		if supervised {
			continue
		}
		live := a.isLive(ctx, r)
		a.mu.Lock()
		if live {
			if r.State != domain.StateRunning {
				r.State = domain.StateRunning
				_ = a.saveRun(r)
			}
			if r.Kind != domain.KindCompose {
				a.adopt(r)
			}
			res.Adopted = append(res.Adopted, r)
		} else {
			res.Dead = append(res.Dead, a.markDeadLocked(r))
		}
		a.mu.Unlock()
	}
	lost, expired, err := a.reconcileJobs(ctx)
	if err != nil {
		return res, err
	}
	res.LostJobs = append(res.LostJobs, lost...)
	res.ExpiredJobs = append(res.ExpiredJobs, expired...)
	released, err := a.releaseStaleLeases()
	res.ReleasedLeases = append(res.ReleasedLeases, released...)
	return res, err
}

func (a *App) releaseStaleLeases() ([]domain.Lease, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	leases, err := a.d.Store.ListLeases()
	if err != nil {
		return nil, err
	}
	var out []domain.Lease
	done := map[string]bool{}
	for _, l := range leases {
		k := key(l.Project, l.Service)
		if done[k] {
			continue
		}
		if r, ok, _ := a.d.Store.GetRun(l.Project, l.Service); ok && r.State.Active() {
			continue
		}
		done[k] = true
		released, _ := a.d.Store.ReleaseLeases(l.Project, l.Service)
		for _, rl := range released {
			a.publishLease(domain.EventPortReleased, rl)
		}
		out = append(out, released...)
	}
	return out, nil
}

// ExpireTTL stops every active run whose TTL elapsed.
func (a *App) ExpireTTL(ctx context.Context) ([]domain.Run, error) {
	runs, err := a.d.Store.ListRuns()
	if err != nil {
		return nil, err
	}
	now := a.now()
	byProject := map[string][]domain.Run{}
	for _, r := range runs {
		if r.State.Active() && r.State != domain.StateStarting && r.Expired(now) {
			byProject[r.Project] = append(byProject[r.Project], r)
		}
	}
	var out []domain.Run
	for name, rs := range byProject {
		p, _ := a.ResolveProject(name)
		stopped, _, _ := a.stopProjectRuns(ctx, p, rs, false)
		out = append(out, stopped...)
	}
	return out, nil
}

// GCAction describes one cleanup performed by GC.
type GCAction struct {
	Action  string `json:"action"` // marked_dead | released_lease | expired | expired_job | stopped_orphan | pruned | lost_job | pruned_job
	Project string `json:"project"`
	Service string `json:"service"`
	Port    int    `json:"port,omitempty"`
	Detail  string `json:"detail,omitempty"`
}

// GCResult lists cleanup actions.
type GCResult struct {
	Actions []GCAction `json:"actions"`
}

// GC reconciles state, expires TTLs, stops runs whose project is no longer
// registered, releases stale leases and prunes inactive run records. It only
// ever signals processes rocket itself started.
func (a *App) GC(ctx context.Context) (GCResult, error) {
	res := GCResult{Actions: []GCAction{}}
	rec, err := a.Reconcile(ctx)
	if err != nil {
		return res, err
	}
	for _, r := range rec.Dead {
		res.Actions = append(res.Actions, GCAction{Action: "marked_dead", Project: r.Project, Service: r.Service})
	}
	for _, id := range rec.LostJobs {
		res.Actions = append(res.Actions, GCAction{Action: "lost_job", Detail: id})
	}
	for _, id := range rec.ExpiredJobs {
		job, _ := a.GetJob(id)
		res.Actions = append(res.Actions, GCAction{Action: "expired_job", Project: job.Project, Service: job.Name, Detail: id})
	}
	for _, l := range rec.ReleasedLeases {
		res.Actions = append(res.Actions, GCAction{Action: "released_lease", Project: l.Project, Service: l.Service, Port: l.Port})
	}
	expired, err := a.ExpireTTL(ctx)
	if err != nil {
		return res, err
	}
	for _, r := range expired {
		res.Actions = append(res.Actions, GCAction{Action: "expired", Project: r.Project, Service: r.Service, Detail: "owner " + r.Owner})
	}

	runs, err := a.d.Store.ListRuns()
	if err != nil {
		return res, err
	}
	orphans := map[string][]domain.Run{}
	for _, r := range runs {
		if !r.State.Active() {
			continue
		}
		if _, ok, _ := a.d.Store.GetProject(r.Project); !ok {
			orphans[r.Project] = append(orphans[r.Project], r)
		}
	}
	for name, rs := range orphans {
		stopped, _, _ := a.stopProjectRuns(ctx, nil, rs, false)
		for _, r := range stopped {
			res.Actions = append(res.Actions, GCAction{Action: "stopped_orphan", Project: name, Service: r.Service,
				Detail: fmt.Sprintf("project %s is not registered", name)})
		}
	}

	runs, _ = a.d.Store.ListRuns()
	for _, r := range runs {
		if r.State.Active() {
			continue
		}
		if r.State == domain.StateFailed && r.Kind == domain.KindCompose && r.ComposeProject != "" {
			continue // whole-project Down still needs this container cleanup tracker
		}
		if err := a.d.Store.DeleteRun(r.Project, r.Service); err == nil {
			res.Actions = append(res.Actions, GCAction{Action: "pruned", Project: r.Project, Service: r.Service, Detail: string(r.State)})
		}
	}
	res.Actions = append(res.Actions, a.pruneJobs()...)
	sort.SliceStable(res.Actions, func(i, j int) bool { return res.Actions[i].Action < res.Actions[j].Action })
	return res, nil
}
