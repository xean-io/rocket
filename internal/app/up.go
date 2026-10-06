package app

import (
	"context"
	"errors"
	"fmt"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// Up actions reported per service.
const (
	ActionStarted        = "started"
	ActionAlreadyRunning = "already_running"
	ActionFailed         = "failed"
	ActionSkipped        = "skipped"
)

// UpRequest starts services (and their dependencies) of one project.
type UpRequest struct {
	Project  string   `json:"project"`            // absolute path or registered name
	Services []string `json:"services,omitempty"` // services or groups; empty = all
	Env      string   `json:"env,omitempty"`      // defaults to the project's default env
	Profiles []string `json:"profiles,omitempty"` // additional profiles enabled for startup wildcard selection
	Owner    string   `json:"owner,omitempty"`    // defaults to "user"
	TTL      string   `json:"ttl,omitempty"`      // Go duration, e.g. "30m"
}

// ServiceResult is the outcome for one service of an up/restart.
type ServiceResult struct {
	Service   string             `json:"service"`
	Action    string             `json:"action"`
	State     domain.RunState    `json:"state"`
	Health    domain.Health      `json:"health,omitempty"`
	PID       int                `json:"pid,omitempty"`
	Ports     map[string]int     `json:"ports,omitempty"`
	Remaps    []domain.PortRemap `json:"remaps,omitempty"`
	Owner     string             `json:"owner,omitempty"`
	ExpiresAt *time.Time         `json:"expires_at,omitempty"`
	LogPath   string             `json:"log_path,omitempty"`
	Error     string             `json:"error,omitempty"`
}

// UpResult lists per-service outcomes in start order.
type UpResult struct {
	Project  string          `json:"project"`
	Env      string          `json:"env"`
	Services []ServiceResult `json:"services"`
	Hints    []string        `json:"hints,omitempty"`
}

// Failed reports whether any service failed or was skipped.
func (r UpResult) Failed() bool {
	for _, s := range r.Services {
		if s.Action == ActionFailed || s.Action == ActionSkipped {
			return true
		}
	}
	return false
}

// Up starts the requested services in dependency order. It is idempotent:
// services that are already running are reported, not restarted.
func (a *App) Up(ctx context.Context, req UpRequest) (UpResult, error) {
	return a.up(ctx, req, nil)
}

// up accepts an absolute deadline for pipeline prerequisites, so time spent
// waiting for earlier services does not extend later services' TTLs.
func (a *App) up(ctx context.Context, req UpRequest, deadline *time.Time) (UpResult, error) {
	if err := ctx.Err(); err != nil {
		return UpResult{}, err
	}
	p, err := a.ResolveProject(req.Project)
	if err != nil {
		return UpResult{}, err
	}
	envName, targets, err := startupTargets(p, req)
	if err != nil {
		return UpResult{}, err
	}
	owner := req.Owner
	if owner == "" {
		owner = domain.DefaultOwner
	}
	expires := deadline
	ttl, err := parseServiceTTL(req.TTL)
	if err != nil {
		return UpResult{}, err
	}
	if ttl > 0 {
		t := a.now().Add(ttl)
		expires = &t
	}
	order, err := p.StartOrder(targets)
	if err != nil {
		return UpResult{}, fmt.Errorf("%w: %v", ErrInvalid, err)
	}

	lock := a.projectLock(p.Name)
	if err := lock.LockContext(ctx); err != nil {
		return UpResult{}, err
	}
	defer lock.Unlock()
	// A project has one live run per service. Check the complete dependency
	// closure before starting anything so no stale environment is reused.
	for _, name := range order {
		if run, ok, _ := a.d.Store.GetRun(p.Name, name); ok && run.Env != envName && a.isServiceLive(ctx, run, p.Services[name]) {
			return UpResult{}, fmt.Errorf("%w: service %q is already running in env %q, requested %q; stop it or restart with --env %s", ErrInvalid, name, run.Env, envName, envName)
		}
	}

	res := UpResult{Project: p.Name, Env: envName, Services: []ServiceResult{}}
	createdVolumes := map[string]bool{}
	failed := map[string]bool{}
	for _, name := range order {
		if err := ctx.Err(); err != nil {
			return res, err
		}
		svc := p.Services[name]
		if dep := firstFailed(svc.DependsOn, failed); dep != "" {
			failed[name] = true
			res.Services = append(res.Services, ServiceResult{Service: name, Action: ActionSkipped, State: domain.StateStopped,
				Error: fmt.Sprintf("dependency %q failed", dep)})
			continue
		}
		if run, ok, _ := a.d.Store.GetRun(p.Name, name); ok && a.isServiceLive(ctx, run, svc) {
			res.Services = append(res.Services, resultFromRun(run, ActionAlreadyRunning, nil))
			continue
		}
		sr := a.startService(ctx, p, envName, svc, req.Profiles, owner, expires, createdVolumes)
		res.Hints = composeVolumeHints(p, envName, createdVolumes)
		if sr.Action == ActionFailed {
			failed[name] = true
		}
		res.Services = append(res.Services, sr)
	}
	return res, nil
}

// Restart stops then starts the requested services.
func (a *App) Restart(ctx context.Context, req UpRequest) (UpResult, error) {
	p, err := a.ResolveProject(req.Project)
	if err != nil {
		return UpResult{}, err
	}
	envName, targets, err := startupTargets(p, req)
	if err != nil {
		return UpResult{}, err
	}
	if _, err := parseServiceTTL(req.TTL); err != nil {
		return UpResult{}, err
	}
	req.Env = envName
	if len(targets) == 0 {
		return a.Up(ctx, req)
	}
	req.Services = targets
	if _, err := a.Down(ctx, DownRequest{Project: req.Project, Services: targets}); err != nil {
		return UpResult{}, err
	}
	return a.Up(ctx, req)
}

func parseServiceTTL(value string) (time.Duration, error) {
	if value == "" {
		return 0, nil
	}
	ttl, err := time.ParseDuration(value)
	if err != nil || ttl <= 0 {
		return 0, fmt.Errorf("%w: invalid ttl %q (use a Go duration like 30m)", ErrInvalid, value)
	}
	return ttl, nil
}

func startupTargets(p *domain.Project, req UpRequest) (string, []string, error) {
	envName := req.Env
	if envName == "" {
		envName = p.DefaultEnv
	}
	env, ok := p.Envs[envName]
	if !ok {
		return "", nil, fmt.Errorf("%w: unknown env %q in project %s", ErrInvalid, envName, p.Name)
	}
	targets, err := p.ExpandStartupTargets(req.Services, domain.MergeProfiles(env.Profiles, req.Profiles))
	if err != nil {
		return "", nil, fmt.Errorf("%w: %v", ErrInvalid, err)
	}
	return envName, targets, nil
}

func firstFailed(deps []string, failed map[string]bool) string {
	for _, d := range deps {
		if failed[d] {
			return d
		}
	}
	return ""
}

func resultFromRun(r domain.Run, action string, remaps []domain.PortRemap) ServiceResult {
	return ServiceResult{
		Service: r.Service, Action: action, State: r.State, Health: r.Health, PID: r.PID, Ports: r.Ports,
		Remaps: remaps, Owner: r.Owner, ExpiresAt: r.ExpiresAt, LogPath: r.LogPath, Error: r.Error,
	}
}

// isLive checks whether an active run still has its process/container.
func (a *App) isLive(ctx context.Context, r domain.Run) bool {
	if !r.State.Active() {
		return false
	}
	if r.Kind == domain.KindCompose {
		name := r.Service
		if p, err := a.ResolveProject(r.Project); err == nil {
			if svc, ok := p.Services[r.Service]; ok && svc.Compose != "" {
				name = svc.Compose
			}
		}
		ok, err := a.d.Compose.Running(ctx, r.ComposeProject, name)
		return err == nil && ok
	}
	return r.PID > 0 && a.d.Runner.Alive(r.PID, r.PGID)
}

// isServiceLive uses the loaded Compose service name, which may differ from
// Rocket's service name. The run identity itself is left unchanged.
func (a *App) isServiceLive(ctx context.Context, r domain.Run, svc domain.Service) bool {
	if r.Kind == domain.KindCompose && svc.Compose != "" {
		if !r.State.Active() {
			return false
		}
		ok, err := a.d.Compose.Running(ctx, r.ComposeProject, svc.Compose)
		return err == nil && ok
	}
	return a.isLive(ctx, r)
}

func (a *App) startService(ctx context.Context, p *domain.Project, envName string, svc domain.Service, profiles []string, owner string, expires *time.Time, createdVolumes map[string]bool) ServiceResult {
	now := a.now()
	run := domain.Run{
		Project: p.Name, Service: svc.Name, Env: envName, Kind: svc.Kind, State: domain.StateStarting,
		Health: domain.HealthUnknown, Owner: owner, ExpiresAt: expires, StartedAt: &now,
	}
	fail := func(remaps []domain.PortRemap, err error) ServiceResult {
		run.State = domain.StateFailed
		run.Health = domain.HealthUnhealthy
		run.Error = err.Error()
		stopped := a.now()
		run.StoppedAt = &stopped
		a.mu.Lock()
		_ = a.saveRun(run)
		a.mu.Unlock()
		a.releaseLeases(p.Name, svc.Name)
		return resultFromRun(run, ActionFailed, remaps)
	}
	if err := ctx.Err(); err != nil {
		return fail(nil, err)
	}

	portsMap, remaps, err := a.leasePorts(p.Name, svc)
	run.Ports = portsMap
	if err != nil {
		return fail(remaps, err)
	}
	svc.Env, err = a.resolvePortReferences(ctx, p, envName, svc.Env)
	if err != nil {
		return fail(remaps, err)
	}
	env, err := a.serviceEnv(p, envName, svc, portsMap)
	if err != nil {
		return fail(remaps, err)
	}
	logf, logPath, err := a.d.Logs.Open(p.Name, svc.Name)
	if err != nil {
		return fail(remaps, fmt.Errorf("open log: %w", err))
	}
	defer logf.Close()
	run.LogPath = logPath
	fmt.Fprintf(logf, "=== rocket: starting %s (%s, env %s, owner %s) at %s ===\n", svc.Name, svc.Kind, envName, owner, now.Format(time.RFC3339))

	var w *watch
	var attemptedCompose *ports.ComposeTarget
	stopCanceledCompose := func(err error) error {
		if attemptedCompose != nil && ctx.Err() != nil {
			cleanup, cancel := context.WithTimeout(context.Background(), a.d.StopGrace+5*time.Second)
			defer cancel()
			if stopErr := a.d.Compose.Stop(cleanup, *attemptedCompose, svc.Compose, logf); stopErr != nil {
				return fmt.Errorf("%w; stop canceled compose service: %v", err, stopErr)
			}
		}
		return err
	}
	if err := ctx.Err(); err != nil {
		return fail(remaps, err)
	}
	switch svc.Kind {
	case domain.KindCompose:
		target := a.composeTarget(p, envName, svc, env, profiles)
		run.ComposeProject = target.ProjectName
		run.Profiles = target.Profiles
		a.mu.Lock()
		_ = a.saveRun(run)
		a.mu.Unlock()
		missing := a.missingComposeVolumes(ctx, target, svc.Compose)
		if err := ctx.Err(); err != nil {
			return fail(remaps, err)
		}
		attemptedCompose = &target
		cid, err := a.d.Compose.Up(ctx, target, svc.Compose, logf)
		a.confirmComposeVolumes(ctx, target, missing, createdVolumes)
		if err != nil {
			return fail(remaps, stopCanceledCompose(fmt.Errorf("docker compose up %s: %w", svc.Compose, err)))
		}
		run.ContainerID = cid
	default:
		argv := []string{"/bin/sh", "-c", svc.Run}
		if svc.Kind == domain.KindTask {
			argv = a.d.Tasks.Argv(svc.Task)
		}
		h, err := a.d.Runner.Start(ports.ProcessSpec{Argv: argv, Dir: filepath.Join(p.Root, svc.Cwd), Env: env, Output: logf})
		if err != nil {
			return fail(remaps, fmt.Errorf("start %s: %w", svc.Name, err))
		}
		run.PID, run.PGID = h.PID, h.PGID
		a.mu.Lock()
		_ = a.saveRun(run)
		w = a.watchProcess(p.Name, svc.Name, h)
		a.mu.Unlock()
	}

	if err := a.waitHealthy(ctx, svc, portsMap, w); err != nil {
		if w != nil {
			_ = a.d.Runner.Stop(context.Background(), run.PGID, a.d.StopGrace)
		}
		return fail(remaps, stopCanceledCompose(err))
	}
	if err := ctx.Err(); err != nil {
		if w != nil {
			_ = a.d.Runner.Stop(context.Background(), run.PGID, a.d.StopGrace)
		}
		return fail(remaps, stopCanceledCompose(err))
	}

	a.mu.Lock()
	defer a.mu.Unlock()
	if w != nil && isClosed(w.exited) {
		code := w.code
		run.ExitCode = &code
		return a.failLocked(p.Name, svc.Name, run, remaps, fmt.Errorf("process exited with code %d right after start", code))
	}
	run.State = domain.StateRunning
	run.Health = domain.HealthHealthy
	_ = a.saveRun(run)
	return resultFromRun(run, ActionStarted, remaps)
}

func (a *App) failLocked(project, service string, run domain.Run, remaps []domain.PortRemap, err error) ServiceResult {
	run.State = domain.StateFailed
	run.Health = domain.HealthUnhealthy
	run.Error = err.Error()
	t := a.now()
	run.StoppedAt = &t
	_ = a.saveRun(run)
	a.releaseLeases(project, service)
	return resultFromRun(run, ActionFailed, remaps)
}

func isClosed(ch chan struct{}) bool {
	select {
	case <-ch:
		return true
	default:
		return false
	}
}

func (a *App) resolvePortReferences(ctx context.Context, p *domain.Project, envName string, source map[string]string) (map[string]string, error) {
	resolved := make(map[string]string, len(source))
	for variable, value := range source {
		for _, ref := range domain.PortReferences(value) {
			run, ok, err := a.d.Store.GetRun(p.Name, ref.Service)
			if err != nil {
				return nil, fmt.Errorf("resolve env %s reference %s: %w", variable, ref.Token, err)
			}
			if !ok || run.State != domain.StateRunning || !a.isServiceLive(ctx, run, p.Services[ref.Service]) {
				return nil, fmt.Errorf("resolve env %s reference %s: provider is not running", variable, ref.Token)
			}
			if run.Env != envName {
				return nil, fmt.Errorf("resolve env %s reference %s: provider is running in env %q, requested %q", variable, ref.Token, run.Env, envName)
			}
			port := run.Ports[ref.Port]
			if port <= 0 {
				return nil, fmt.Errorf("resolve env %s reference %s: provider has no resolved port", variable, ref.Token)
			}
			value = ref.Replace(value, strconv.Itoa(port))
		}
		resolved[variable] = value
	}
	return resolved, nil
}

func (a *App) serviceEnv(p *domain.Project, envName string, svc domain.Service, portsMap map[string]int) ([]string, error) {
	files := append(append([]string{}, p.Dotenv...), svc.Dotenv...)
	dotenv, err := a.d.Env.Dotenv(p.Root, files)
	if err != nil {
		return nil, fmt.Errorf("read dotenv: %w", err)
	}
	merged := domain.BuildEnv(a.d.BaseEnv(), dotenv, svc.Env)
	values := map[string]string{}
	for _, item := range merged {
		key, value, _ := strings.Cut(item, "=")
		values[key] = value
	}
	portVars := map[string]string{}
	for _, spec := range svc.Ports {
		port := strconv.Itoa(portsMap[spec.Name])
		if spec.Env != "" && len(spec.EnvBindings) == 0 {
			portVars[spec.Env] = port
		}
		for variable, binding := range spec.EnvBindings {
			if !binding.Default || values[variable] == "" {
				portVars[variable] = strings.ReplaceAll(binding.Template, "{port}", port)
			}
		}
	}
	identity := map[string]string{"COMPOSE_PROJECT_NAME": domain.ComposeProjectName(p.Name, envName)}
	return domain.BuildEnv(nil, values, portVars, identity), nil
}

func (a *App) composeTarget(p *domain.Project, envName string, svc domain.Service, env []string, profiles []string) ports.ComposeTarget {
	e := p.Envs[envName]
	t := ports.ComposeTarget{ProjectName: domain.ComposeProjectName(p.Name, envName), Dir: p.Root, Env: env}
	for _, f := range e.Compose {
		if !filepath.IsAbs(f) {
			f = filepath.Join(p.Root, f)
		}
		t.Files = append(t.Files, f)
	}
	t.Profiles = domain.MergeProfiles(e.Profiles, profiles, svc.Profiles)
	return t
}

// leasePorts reserves every port of svc, remapping busy ones when the port
// declares an env var.
func (a *App) leasePorts(project string, svc domain.Service) (map[string]int, []domain.PortRemap, error) {
	out := map[string]int{}
	var remaps []domain.PortRemap
	for _, spec := range svc.Ports {
		reason, holder := a.portBusy(spec.Default, project, svc.Name)
		if reason == "" {
			if err := a.lease(spec.Default, project, svc.Name, spec.Name); err == nil {
				out[spec.Name] = spec.Default
				continue
			}
			reason = "leased concurrently"
		}
		if spec.Env == "" {
			return out, remaps, fmt.Errorf("port %d (%s) for %s/%s is busy: %s; it declares no env var so rocket cannot remap it — free the port or add `env:` to the port in rocket.yaml",
				spec.Default, spec.Name, project, svc.Name, reason)
		}
		chosen := 0
		for _, c := range domain.RemapCandidates(spec.Default) {
			if r, _ := a.portBusy(c, project, svc.Name); r != "" {
				continue
			}
			if err := a.lease(c, project, svc.Name, spec.Name); err == nil {
				chosen = c
				break
			}
		}
		if chosen == 0 {
			return out, remaps, fmt.Errorf("port %d (%s) for %s/%s is busy (%s) and no free port was found to remap", spec.Default, spec.Name, project, svc.Name, reason)
		}
		out[spec.Name] = chosen
		remaps = append(remaps, domain.PortRemap{Name: spec.Name, From: spec.Default, To: chosen, Env: spec.Env, Holder: holder, Reason: reason})
	}
	return out, remaps, nil
}

func (a *App) lease(port int, project, service, name string) error {
	l := domain.Lease{Port: port, Project: project, Service: service, PortName: name, CreatedAt: a.now()}
	if err := a.d.Store.AcquireLease(l); err != nil {
		return err
	}
	a.publishLease(domain.EventPortLeased, l)
	return nil
}

// portBusy returns a human reason when port is unavailable to project/service.
func (a *App) portBusy(port int, project, service string) (string, *domain.PortHolder) {
	leases, _ := a.d.Store.ListLeases()
	for _, l := range leases {
		if l.Port != port || (l.Project == project && l.Service == service) {
			continue
		}
		if run, ok, _ := a.d.Store.GetRun(l.Project, l.Service); ok && run.State.Active() {
			return fmt.Sprintf("leased by %s/%s", l.Project, l.Service), nil
		}
		// stale lease left by a run that is no longer active
		a.releaseLeases(l.Project, l.Service)
	}
	if a.d.Probe.Free(port) {
		return "", nil
	}
	holder, _ := a.d.Probe.Holder(port)
	if holder != nil {
		desc := fmt.Sprintf("in use by %s (pid %d)", holder.Command, holder.PID)
		if holder.Cwd != "" {
			desc += " in " + holder.Cwd
		}
		return desc, holder
	}
	return "in use by an unknown process", nil
}

// watchProcess must be called with a.mu held.
func (a *App) watchProcess(project, service string, h ports.ProcessHandle) *watch {
	w := &watch{pid: h.PID, exited: make(chan struct{})}
	a.watches[key(project, service)] = w
	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		select {
		case code := <-h.Done:
			w.code = code
			close(w.exited)
			a.onExit(project, service, h.PID, code)
		case <-a.ctx.Done():
		}
	}()
	return w
}

// adopt polls a process started by a previous daemon. Must hold a.mu.
func (a *App) adopt(r domain.Run) {
	w := &watch{pid: r.PID, exited: make(chan struct{})}
	a.watches[key(r.Project, r.Service)] = w
	a.wg.Add(1)
	go func() {
		defer a.wg.Done()
		t := time.NewTicker(a.d.PollInterval)
		defer t.Stop()
		for {
			select {
			case <-a.ctx.Done():
				return
			case <-t.C:
				if !a.d.Runner.Alive(r.PID, r.PGID) {
					w.code = -1
					close(w.exited)
					a.onExit(r.Project, r.Service, r.PID, -1)
					return
				}
			}
		}
	}()
}

func (a *App) onExit(project, service string, pid, code int) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if w, ok := a.watches[key(project, service)]; ok && w.pid == pid {
		delete(a.watches, key(project, service))
	}
	run, ok, _ := a.d.Store.GetRun(project, service)
	if !ok || run.PID != pid {
		return
	}
	c := code
	switch run.State {
	case domain.StateStarting:
		return // Up owns the outcome and observes the exit itself
	case domain.StateRunning:
		run.State = domain.StateExited
		run.Health = domain.HealthUnknown
		t := a.now()
		run.StoppedAt = &t
		run.ExitCode = &c
		_ = a.saveRun(run)
		a.releaseLeases(project, service)
	default:
		if run.ExitCode == nil {
			run.ExitCode = &c
			_ = a.d.Store.SaveRun(run)
		}
	}
}

func (a *App) waitHealthy(ctx context.Context, svc domain.Service, portsMap map[string]int, w *watch) error {
	var exited <-chan struct{}
	if w != nil {
		exited = w.exited
	}
	check := domain.HealthCheckFor(svc, portsMap)
	if check == nil {
		if w == nil {
			return nil
		}
		select {
		case <-exited:
			return fmt.Errorf("process exited with code %d right after start", w.code)
		case <-time.After(a.d.StartGrace):
			return nil
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	timeout := svc.HealthTimeout()
	hctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	var lastErr error
	for {
		if exited != nil && isClosed(w.exited) {
			return fmt.Errorf("process exited with code %d before becoming healthy", w.code)
		}
		pctx, pcancel := context.WithTimeout(hctx, 2*time.Second)
		lastErr = a.d.Health.Check(pctx, *check)
		pcancel()
		if lastErr == nil {
			return nil
		}
		select {
		case <-hctx.Done():
			if errors.Is(ctx.Err(), context.Canceled) {
				return ctx.Err()
			}
			return fmt.Errorf("not healthy after %s (%s probe on port %d): %v", timeout, check.Kind, check.Port, lastErr)
		case <-exited:
		case <-time.After(a.d.HealthInterval):
		}
	}
}
