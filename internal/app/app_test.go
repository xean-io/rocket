package app

import (
	"context"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
)

type harness struct {
	app     *App
	store   *memStore
	runner  *fakeRunner
	compose *fakeCompose
	probe   *fakeProbe
	health  *fakeHealth
	bus     *recBus
	clock   *clock
}

func nuvara() *domain.Project {
	return &domain.Project{
		Name: "nuvara", Root: "/code/nuvara", DefaultEnv: "dev", Dotenv: []string{".env"},
		Envs: map[string]domain.Environment{"dev": {Name: "dev", Compose: []string{"docker-compose.yml"}}},
		Services: map[string]domain.Service{
			"postgres": {Name: "postgres", Kind: domain.KindCompose, Compose: "postgres", Profiles: []string{"deps"},
				Ports: []domain.PortSpec{{Name: "main", Default: 5435, Env: "POSTGRES_PORT"}}},
			"api": {Name: "api", Kind: domain.KindRun, Run: "bun run dev", Cwd: "apps/api", DependsOn: []string{"postgres"},
				Ports: []domain.PortSpec{{Name: "http", Default: 3002, Env: "PORT"}}, Health: &domain.HealthSpec{HTTP: "/health"}},
			"web": {Name: "web", Kind: domain.KindRun, Run: "bun run dev", Cwd: "apps/web", DependsOn: []string{"api"},
				Ports: []domain.PortSpec{{Name: "http", Default: 3000, Env: "PORT"}}},
			"causation": {Name: "causation", Kind: domain.KindTask, Task: "dev:causation",
				Ports: []domain.PortSpec{{Name: "http", Default: 3003}}},
			"worker": {Name: "worker", Kind: domain.KindRun, Run: "sleep 1000", DependsOn: []string{"causation"}},
		},
		Groups: map[string][]string{"core": {"api", "web"}, "all": {"*"}},
	}
}

func otherProject() *domain.Project {
	return &domain.Project{
		Name: "shop", Root: "/code/shop", DefaultEnv: "dev",
		Envs: map[string]domain.Environment{"dev": {Name: "dev"}},
		Services: map[string]domain.Service{
			"control-plane": {Name: "control-plane", Kind: domain.KindRun, Run: "pnpm dev",
				Ports: []domain.PortSpec{{Name: "http", Default: 3000, Env: "PORT"}}},
		},
	}
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	h := &harness{
		store: newMemStore(), runner: newFakeRunner(), compose: newFakeCompose(),
		probe: &fakeProbe{busy: map[int]*domain.PortHolder{}}, health: &fakeHealth{failing: map[int]bool{}},
		bus: &recBus{}, clock: &clock{now: time.Date(2026, 1, 1, 10, 0, 0, 0, time.UTC)},
	}
	h.app = New(Deps{
		Store:          h.store,
		Manifests:      fakeLoader{projects: map[string]*domain.Project{"/code/nuvara": nuvara(), "/code/shop": otherProject(), "/code/jobs": jobsProject()}},
		Env:            fakeEnv{values: map[string]string{"FROM_DOTENV": "1"}},
		Runner:         h.runner,
		Compose:        h.compose,
		Tasks:          fakeTask{},
		Probe:          h.probe,
		Health:         h.health,
		Bus:            h.bus,
		Logs:           fileLogs{dir: t.TempDir()},
		Now:            h.clock.Now,
		BaseEnv:        func() []string { return []string{"PATH=/usr/bin", "ROCKET_OWNER=agent:leak"} },
		HealthInterval: time.Millisecond,
		StopGrace:      time.Millisecond,
		PollInterval:   5 * time.Millisecond,
		StartGrace:     5 * time.Millisecond,
	})
	t.Cleanup(h.app.Close)
	return h
}

func (h *harness) up(t *testing.T, req UpRequest) UpResult {
	t.Helper()
	if req.Project == "" {
		req.Project = "/code/nuvara"
	}
	res, err := h.app.Up(context.Background(), req)
	if err != nil {
		t.Fatalf("up: %v", err)
	}
	return res
}

func actions(res UpResult) map[string]string {
	out := map[string]string{}
	for _, s := range res.Services {
		out[s.Service] = s.Action
	}
	return out
}

func envMap(env []string) map[string]string {
	m := map[string]string{}
	for _, kv := range env {
		k, v, _ := strings.Cut(kv, "=")
		m[k] = v
	}
	return m
}

func (h *harness) run(t *testing.T, project, svc string) domain.Run {
	t.Helper()
	r, ok, _ := h.store.GetRun(project, svc)
	if !ok {
		t.Fatalf("no run for %s/%s", project, svc)
	}
	return r
}

func eventually(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("condition not met in time")
}

func TestUpStartsDependenciesInOrder(t *testing.T) {
	h := newHarness(t)
	res := h.up(t, UpRequest{Services: []string{"web"}})

	var order []string
	for _, s := range res.Services {
		order = append(order, s.Service)
		if s.Action != ActionStarted {
			t.Fatalf("%s: action %s err %s", s.Service, s.Action, s.Error)
		}
	}
	if !slices.Equal(order, []string{"postgres", "api", "web"}) {
		t.Fatalf("order %v", order)
	}
	if res.Env != "dev" || res.Project != "nuvara" {
		t.Fatalf("result header %+v", res)
	}

	up := h.compose.calls[0]
	if up.target.ProjectName != "rocket-nuvara-dev" || up.service != "postgres" ||
		!slices.Equal(up.target.Profiles, []string{"deps"}) ||
		!slices.Equal(up.target.Files, []string{"/code/nuvara/docker-compose.yml"}) {
		t.Fatalf("compose target %+v", up)
	}
	if envMap(up.target.Env)["POSTGRES_PORT"] != "5435" {
		t.Fatal("compose port var not injected")
	}

	api := h.runner.specFor(t, "apps/api")
	if !slices.Equal(api.Argv, []string{"/bin/sh", "-c", "bun run dev"}) {
		t.Fatalf("argv %v", api.Argv)
	}
	env := envMap(api.Env)
	if env["PORT"] != "3002" || env["FROM_DOTENV"] != "1" || env["PATH"] != "/usr/bin" {
		t.Fatalf("api env %v", env)
	}
	if _, leaked := env["ROCKET_OWNER"]; leaked {
		t.Fatal("ROCKET_* leaked into child env")
	}

	r := h.run(t, "nuvara", "api")
	if r.State != domain.StateRunning || r.Health != domain.HealthHealthy || r.Owner != domain.DefaultOwner || r.Ports["http"] != 3002 {
		t.Fatalf("api run %+v", r)
	}
	if h.health.checks[0] != (domain.HealthCheck{Kind: "http", Port: 3002, Path: "/health"}) {
		t.Fatalf("health check %+v", h.health.checks[0])
	}
	leases, _ := h.store.ListLeases()
	if len(leases) != 3 {
		t.Fatalf("leases %+v", leases)
	}
}

func TestUpIsIdempotent(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"core"}})
	started := len(h.runner.started)

	res := h.up(t, UpRequest{Services: []string{"core"}})
	for _, s := range res.Services {
		if s.Action != ActionAlreadyRunning {
			t.Fatalf("%s: %s", s.Service, s.Action)
		}
	}
	if len(h.runner.started) != started {
		t.Fatal("services were started twice")
	}
}

func TestUpRemapsBusyPortAndInjectsEnv(t *testing.T) {
	h := newHarness(t)
	h.probe.busy[3000] = &domain.PortHolder{PID: 77, Command: "node", Cwd: "/code/elsewhere"}

	res := h.up(t, UpRequest{Services: []string{"web"}})
	web := res.Services[len(res.Services)-1]
	if web.Action != ActionStarted || web.Ports["http"] != 3100 {
		t.Fatalf("web result %+v", web)
	}
	if len(web.Remaps) != 1 || web.Remaps[0].From != 3000 || web.Remaps[0].To != 3100 ||
		web.Remaps[0].Env != "PORT" || web.Remaps[0].Holder.PID != 77 {
		t.Fatalf("remaps %+v", web.Remaps)
	}
	if envMap(h.runner.specFor(t, "apps/web").Env)["PORT"] != "3100" {
		t.Fatal("remapped port not injected")
	}
}

func TestUpRemapsPortLeasedByAnotherProject(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Project: "/code/shop"})
	res := h.up(t, UpRequest{Services: []string{"web"}})
	web := res.Services[len(res.Services)-1]
	if web.Ports["http"] != 3100 || !strings.Contains(web.Remaps[0].Reason, "shop/control-plane") {
		t.Fatalf("web %+v", web)
	}
}

func TestUpFailsBusyPortWithoutEnvAndSkipsDependents(t *testing.T) {
	h := newHarness(t)
	h.probe.busy[3003] = &domain.PortHolder{PID: 9, Command: "python3"}

	res := h.up(t, UpRequest{Services: []string{"worker"}})
	acts := actions(res)
	if acts["causation"] != ActionFailed || acts["worker"] != ActionSkipped {
		t.Fatalf("actions %v", acts)
	}
	msg := res.Services[0].Error
	for _, want := range []string{"3003", "python3", "pid 9", "env"} {
		if !strings.Contains(msg, want) {
			t.Fatalf("error %q missing %q", msg, want)
		}
	}
	if len(h.runner.started) != 0 {
		t.Fatal("nothing should have started")
	}
	if !res.Failed() {
		t.Fatal("result should report failure")
	}
}

func TestUpFailsWhenProcessExitsBeforeHealthy(t *testing.T) {
	h := newHarness(t)
	h.runner.exitNow["bun run dev"] = 1
	h.health.failing[3002] = true

	res := h.up(t, UpRequest{Services: []string{"api"}})
	acts := actions(res)
	if acts["api"] != ActionFailed {
		t.Fatalf("actions %v", acts)
	}
	if !strings.Contains(res.Services[1].Error, "exited with code 1") {
		t.Fatalf("error %q", res.Services[1].Error)
	}
	if r := h.run(t, "nuvara", "api"); r.State != domain.StateFailed {
		t.Fatalf("state %s", r.State)
	}
	leases, _ := h.store.ListLeases()
	for _, l := range leases {
		if l.Service == "api" {
			t.Fatal("failed service kept its lease")
		}
	}
}

func TestProcessExitIsRecorded(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"causation"}})
	r := h.run(t, "nuvara", "causation")
	h.runner.exit(r.PID, 2)
	eventually(t, func() bool { return h.run(t, "nuvara", "causation").State == domain.StateExited })
	r = h.run(t, "nuvara", "causation")
	if r.ExitCode == nil || *r.ExitCode != 2 {
		t.Fatalf("exit code %v", r.ExitCode)
	}
	leases, _ := h.store.ListLeases()
	if len(leases) != 0 {
		t.Fatalf("leases not released: %+v", leases)
	}
}

func TestDownByOwnerOnlyStopsThatOwner(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"postgres"}, Owner: "user"})
	h.up(t, UpRequest{Services: []string{"api"}, Owner: "agent:a1"})

	if got := h.run(t, "nuvara", "postgres").Owner; got != "user" {
		t.Fatalf("postgres owner changed to %s", got)
	}
	res, err := h.app.Down(context.Background(), DownRequest{Project: "/code/nuvara", Owner: "agent:a1"})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Stopped) != 1 || res.Stopped[0].Service != "api" {
		t.Fatalf("stopped %+v", res.Stopped)
	}
	if h.run(t, "nuvara", "postgres").State != domain.StateRunning {
		t.Fatal("postgres should keep running")
	}
	if h.run(t, "nuvara", "api").State != domain.StateStopped {
		t.Fatal("api should be stopped")
	}
}

func TestDownProjectStopsInReverseOrderAndDownsCompose(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"web"}})
	res, err := h.app.Down(context.Background(), DownRequest{Project: "/code/nuvara"})
	if err != nil {
		t.Fatal(err)
	}
	var order []string
	for _, r := range res.Stopped {
		order = append(order, r.Service)
	}
	if !slices.Equal(order, []string{"web", "api", "postgres"}) {
		t.Fatalf("stop order %v", order)
	}
	if !slices.Equal(h.compose.ops(), []string{"up:postgres", "stop:postgres", "down:"}) {
		t.Fatalf("compose ops %v", h.compose.ops())
	}
	if leases, _ := h.store.ListLeases(); len(leases) != 0 {
		t.Fatalf("leases left %+v", leases)
	}
}

func TestDownEverywhere(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"api"}})
	h.up(t, UpRequest{Project: "/code/shop"})
	res, err := h.app.Down(context.Background(), DownRequest{Everywhere: true})
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Stopped) != 3 {
		t.Fatalf("stopped %+v", res.Stopped)
	}
}

func TestTTLExpiry(t *testing.T) {
	h := newHarness(t)
	res := h.up(t, UpRequest{Services: []string{"causation"}, Owner: "agent:t1", TTL: "1m"})
	if res.Services[0].ExpiresAt == nil {
		t.Fatal("expires_at missing")
	}
	if expired, _ := h.app.ExpireTTL(context.Background()); len(expired) != 0 {
		t.Fatal("expired too early")
	}
	h.clock.Advance(2 * time.Minute)
	expired, err := h.app.ExpireTTL(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(expired) != 1 || h.run(t, "nuvara", "causation").State != domain.StateStopped {
		t.Fatalf("expired %+v", expired)
	}
}

func TestInvalidTTLRejected(t *testing.T) {
	h := newHarness(t)
	_, err := h.app.Up(context.Background(), UpRequest{Project: "/code/nuvara", TTL: "soon"})
	if err == nil || !strings.Contains(err.Error(), "ttl") {
		t.Fatalf("err %v", err)
	}
}

func TestRestart(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"causation"}})
	first := h.run(t, "nuvara", "causation").PID
	res, err := h.app.Restart(context.Background(), UpRequest{Project: "/code/nuvara", Services: []string{"causation"}})
	if err != nil {
		t.Fatal(err)
	}
	if res.Services[0].Action != ActionStarted || h.run(t, "nuvara", "causation").PID == first {
		t.Fatalf("restart result %+v", res.Services)
	}
}

func TestReconcile(t *testing.T) {
	h := newHarness(t)
	now := h.clock.Now()
	for _, r := range []domain.Run{
		{Project: "nuvara", Service: "api", Kind: domain.KindRun, State: domain.StateRunning, PID: 10, PGID: 10, StartedAt: &now},
		{Project: "nuvara", Service: "web", Kind: domain.KindRun, State: domain.StateRunning, PID: 11, PGID: 11, StartedAt: &now},
		{Project: "nuvara", Service: "postgres", Kind: domain.KindCompose, State: domain.StateRunning, ComposeProject: "rocket-nuvara-dev"},
		{Project: "nuvara", Service: "worker", Kind: domain.KindRun, State: domain.StateStopped},
	} {
		_ = h.store.SaveRun(r)
	}
	_ = h.store.AcquireLease(domain.Lease{Port: 3000, Project: "nuvara", Service: "web", PortName: "http"})
	_ = h.store.AcquireLease(domain.Lease{Port: 4000, Project: "nuvara", Service: "worker", PortName: "x"})
	h.runner.alive[10] = true

	res, err := h.app.Reconcile(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Adopted) != 1 || res.Adopted[0].Service != "api" {
		t.Fatalf("adopted %+v", res.Adopted)
	}
	if len(res.Dead) != 2 {
		t.Fatalf("dead %+v", res.Dead)
	}
	if h.run(t, "nuvara", "web").State != domain.StateDead || h.run(t, "nuvara", "postgres").State != domain.StateDead {
		t.Fatal("dead runs not marked")
	}
	if leases, _ := h.store.ListLeases(); len(leases) != 0 {
		t.Fatalf("stale leases kept: %+v", leases)
	}

	// The adopted process is polled; when it dies the run is updated.
	h.runner.mu.Lock()
	h.runner.alive[10] = false
	h.runner.mu.Unlock()
	eventually(t, func() bool { return h.run(t, "nuvara", "api").State == domain.StateExited })
}

func TestStatusListsDeclaredServices(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"causation"}})
	res, err := h.app.Status(context.Background(), StatusRequest{Project: "/code/nuvara"})
	if err != nil {
		t.Fatal(err)
	}
	states := map[string]domain.RunState{}
	for _, r := range res.Services {
		states[r.Service] = r.State
	}
	if len(states) != 5 || states["causation"] != domain.StateRunning || states["web"] != domain.StateStopped {
		t.Fatalf("states %v", states)
	}
}

func TestGCReleasesStaleLeasesAndPrunes(t *testing.T) {
	h := newHarness(t)
	_ = h.store.SaveRun(domain.Run{Project: "nuvara", Service: "web", Kind: domain.KindRun, State: domain.StateRunning, PID: 55, PGID: 55})
	_ = h.store.AcquireLease(domain.Lease{Port: 3000, Project: "nuvara", Service: "web", PortName: "http"})
	_ = h.store.AcquireLease(domain.Lease{Port: 9999, Project: "ghost", Service: "x", PortName: "http"})

	res, err := h.app.GC(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Actions) < 2 {
		t.Fatalf("actions %+v", res.Actions)
	}
	if leases, _ := h.store.ListLeases(); len(leases) != 0 {
		t.Fatalf("leases %+v", leases)
	}
	if _, ok, _ := h.store.GetRun("nuvara", "web"); ok {
		t.Fatal("dead run record should be pruned")
	}
}

func TestProjectsRegistry(t *testing.T) {
	h := newHarness(t)
	ref, err := h.app.AddProject("/code/nuvara")
	if err != nil || ref.Name != "nuvara" {
		t.Fatalf("add %v %+v", err, ref)
	}
	if _, err := h.app.Up(context.Background(), UpRequest{Project: "nuvara", Services: []string{"causation"}}); err != nil {
		t.Fatalf("up by name: %v", err)
	}
	if err := h.app.RemoveProject("nuvara"); err == nil {
		t.Fatal("removing a project with running services must fail")
	}
	if _, err := h.app.Up(context.Background(), UpRequest{Project: "unknown"}); err == nil {
		t.Fatal("unknown project should fail")
	}
}
