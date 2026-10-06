package app

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// memStore is an in-memory ports.Store.
type memStore struct {
	mu       sync.Mutex
	projects map[string]domain.ProjectRef
	runs     map[string]domain.Run
	leases   map[int]domain.Lease
	jobs     map[string]domain.Job
}

func newMemStore() *memStore {
	return &memStore{projects: map[string]domain.ProjectRef{}, runs: map[string]domain.Run{}, leases: map[int]domain.Lease{},
		jobs: map[string]domain.Job{}}
}

func (m *memStore) SaveJob(j domain.Job) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.jobs[j.ID] = j
	return nil
}
func (m *memStore) GetJob(id string) (domain.Job, bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	j, ok := m.jobs[id]
	return j, ok, nil
}
func (m *memStore) ListJobs(project string, limit int) ([]domain.Job, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := []domain.Job{}
	for _, j := range m.jobs {
		if project == "" || j.Project == project {
			out = append(out, j)
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if !out[i].StartedAt.Equal(out[j].StartedAt) {
			return out[i].StartedAt.After(out[j].StartedAt)
		}
		return out[i].ID > out[j].ID
	})
	if limit > 0 && len(out) > limit {
		out = out[:limit]
	}
	return out, nil
}
func (m *memStore) DeleteJob(id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.jobs, id)
	return nil
}

func runKey(p, s string) string { return p + "/" + s }

func (m *memStore) UpsertProject(r domain.ProjectRef) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.projects[r.Name] = r
	return nil
}
func (m *memStore) GetProject(name string) (domain.ProjectRef, bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	r, ok := m.projects[name]
	return r, ok, nil
}
func (m *memStore) ListProjects() ([]domain.ProjectRef, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []domain.ProjectRef
	for _, r := range m.projects {
		out = append(out, r)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}
func (m *memStore) DeleteProject(name string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.projects, name)
	return nil
}
func (m *memStore) SaveRun(r domain.Run) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.runs[runKey(r.Project, r.Service)] = r
	return nil
}
func (m *memStore) GetRun(p, s string) (domain.Run, bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	r, ok := m.runs[runKey(p, s)]
	return r, ok, nil
}
func (m *memStore) ListRuns() ([]domain.Run, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []domain.Run
	for _, r := range m.runs {
		out = append(out, r)
	}
	sort.Slice(out, func(i, j int) bool {
		return runKey(out[i].Project, out[i].Service) < runKey(out[j].Project, out[j].Service)
	})
	return out, nil
}
func (m *memStore) DeleteRun(p, s string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.runs, runKey(p, s))
	return nil
}
func (m *memStore) AcquireLease(l domain.Lease) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if cur, ok := m.leases[l.Port]; ok && (cur.Project != l.Project || cur.Service != l.Service) {
		return ports.ErrLeaseTaken
	}
	m.leases[l.Port] = l
	return nil
}
func (m *memStore) ReleaseLeases(p, s string) ([]domain.Lease, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []domain.Lease
	for port, l := range m.leases {
		if l.Project == p && l.Service == s {
			out = append(out, l)
			delete(m.leases, port)
		}
	}
	return out, nil
}
func (m *memStore) ListLeases() ([]domain.Lease, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []domain.Lease
	for _, l := range m.leases {
		out = append(out, l)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Port < out[j].Port })
	return out, nil
}

type fakeLoader struct{ projects map[string]*domain.Project }

func (f fakeLoader) Load(dir string) (*domain.Project, error) {
	p, ok := f.projects[dir]
	if !ok {
		return nil, fmt.Errorf("no rocket.yaml in %s", dir)
	}
	return p, nil
}

type fakeEnv struct{ values map[string]string }

func (f fakeEnv) Dotenv(string, []string) (map[string]string, error) { return f.values, nil }

type fakeProc struct {
	spec ports.ProcessSpec
	pid  int
	done chan int
	dead bool
}

type fakeRunner struct {
	mu       sync.Mutex
	next     int
	procs    map[int]*fakeProc // by pgid
	started  []ports.ProcessSpec
	stopped  []int
	exitNow  map[string]int // argv-substring -> immediate exit code
	alive    map[int]bool   // overrides for adopted pids
	startErr error
}

func newFakeRunner() *fakeRunner {
	return &fakeRunner{next: 1000, procs: map[int]*fakeProc{}, exitNow: map[string]int{}, alive: map[int]bool{}}
}

func (r *fakeRunner) Start(spec ports.ProcessSpec) (ports.ProcessHandle, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.startErr != nil {
		return ports.ProcessHandle{}, r.startErr
	}
	r.next++
	p := &fakeProc{spec: spec, pid: r.next, done: make(chan int, 1)}
	r.procs[p.pid] = p
	r.started = append(r.started, spec)
	for sub, code := range r.exitNow {
		if strings.Contains(strings.Join(spec.Argv, " "), sub) {
			p.dead = true
			p.done <- code
		}
	}
	return ports.ProcessHandle{PID: p.pid, PGID: p.pid, Done: p.done}, nil
}

func (r *fakeRunner) Stop(_ context.Context, pgid int, _ time.Duration) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.stopped = append(r.stopped, pgid)
	if p, ok := r.procs[pgid]; ok && !p.dead {
		p.dead = true
		p.done <- 143
	}
	r.alive[pgid] = false
	return nil
}

func (r *fakeRunner) Alive(pid, _ int) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	if v, ok := r.alive[pid]; ok {
		return v
	}
	p, ok := r.procs[pid]
	return ok && !p.dead
}

func (r *fakeRunner) exit(pid, code int) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if p, ok := r.procs[pid]; ok && !p.dead {
		p.dead = true
		p.done <- code
	}
}

func (r *fakeRunner) specFor(t *testing.T, cwdSuffix string) ports.ProcessSpec {
	t.Helper()
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, s := range r.started {
		if strings.HasSuffix(s.Dir, cwdSuffix) {
			return s
		}
	}
	t.Fatalf("no process started in %s", cwdSuffix)
	return ports.ProcessSpec{}
}

type composeCall struct {
	op      string
	target  ports.ComposeTarget
	service string
}

type fakeCompose struct {
	mu      sync.Mutex
	calls   []composeCall
	running map[string]bool // composeProject/service
}

func newFakeCompose() *fakeCompose { return &fakeCompose{running: map[string]bool{}} }

func (c *fakeCompose) NamedVolumes(context.Context, ports.ComposeTarget, string) ([]string, error) {
	return nil, nil
}
func (c *fakeCompose) VolumeExists(context.Context, ports.ComposeTarget, string) (bool, error) {
	return false, nil
}

func (c *fakeCompose) record(op string, t ports.ComposeTarget, svc string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.calls = append(c.calls, composeCall{op, t, svc})
}
func (c *fakeCompose) Up(_ context.Context, t ports.ComposeTarget, svc string, _ io.Writer) (string, error) {
	c.record("up", t, svc)
	c.mu.Lock()
	c.running[t.ProjectName+"/"+svc] = true
	c.mu.Unlock()
	return "cid-" + svc, nil
}
func (c *fakeCompose) Stop(_ context.Context, t ports.ComposeTarget, svc string, _ io.Writer) error {
	c.record("stop", t, svc)
	c.mu.Lock()
	c.running[t.ProjectName+"/"+svc] = false
	c.mu.Unlock()
	return nil
}
func (c *fakeCompose) Down(_ context.Context, t ports.ComposeTarget, _ io.Writer) error {
	c.record("down", t, "")
	return nil
}
func (c *fakeCompose) Running(_ context.Context, project, svc string) (bool, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.running[project+"/"+svc], nil
}
func (c *fakeCompose) Logs(context.Context, ports.ComposeTarget, string, int) ([]string, error) {
	return []string{"compose log"}, nil
}
func (c *fakeCompose) ops() []string {
	c.mu.Lock()
	defer c.mu.Unlock()
	var out []string
	for _, call := range c.calls {
		out = append(out, call.op+":"+call.service)
	}
	return out
}

type fakeTask struct{}

func (fakeTask) Argv(task string, args ...string) []string {
	return append([]string{"task", task}, args...)
}

// Taskfile pretends only /code/jobs has a Taskfile.
func (fakeTask) Taskfile(dir string) string {
	if dir == "/code/jobs" {
		return "/code/jobs/Taskfile.yml"
	}
	return ""
}

type fakeProbe struct {
	mu      sync.Mutex
	busy    map[int]*domain.PortHolder
	checked []int
}

func (f *fakeProbe) Free(port int) bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.checked = append(f.checked, port)
	_, busy := f.busy[port]
	return !busy
}
func (f *fakeProbe) Holder(port int) (*domain.PortHolder, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.busy[port], nil
}

type fakeHealth struct {
	mu      sync.Mutex
	failing map[int]bool
	checks  []domain.HealthCheck
}

func (f *fakeHealth) Check(_ context.Context, c domain.HealthCheck) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.checks = append(f.checks, c)
	if f.failing[c.Port] {
		return errors.New("connection refused")
	}
	return nil
}

type recBus struct {
	mu     sync.Mutex
	events []domain.Event
}

func (b *recBus) Publish(e domain.Event) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.events = append(b.events, e)
}
func (b *recBus) Subscribe(int) (<-chan domain.Event, func()) {
	ch := make(chan domain.Event)
	return ch, func() {}
}

type fileLogs struct{ dir string }

func (l fileLogs) Open(p, s string) (*os.File, string, error) {
	path := filepath.Join(l.dir, p+"-"+s+".log")
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
	return f, path, err
}
func (l fileLogs) OpenJob(p, id string) (*os.File, string, error) { return l.Open(p, "job-"+id) }
func (l fileLogs) TailJob(p, id string, n int) ([]string, error)  { return l.Tail(p, "job-"+id, n) }
func (l fileLogs) CloseJob(string, string)                        {}
func (l fileLogs) RemoveJob(p, id string) error {
	return os.Remove(filepath.Join(l.dir, p+"-job-"+id+".log"))
}

func (l fileLogs) Tail(p, s string, n int) ([]string, error) {
	data, err := os.ReadFile(filepath.Join(l.dir, p+"-"+s+".log"))
	if err != nil {
		return nil, err
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	return lines, nil
}

type clock struct {
	mu  sync.Mutex
	now time.Time
}

func (c *clock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}
func (c *clock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.now = c.now.Add(d)
}
