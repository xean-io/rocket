package sqlite

import (
	"encoding/json"
	"errors"
	"path/filepath"
	"slices"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

func openTemp(t *testing.T) *Store {
	t.Helper()
	s, err := Open(filepath.Join(t.TempDir(), "state.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func TestProjectsRoundTrip(t *testing.T) {
	s := openTemp(t)
	now := time.Now().UTC().Truncate(time.Second)
	if err := s.UpsertProject(domain.ProjectRef{Name: "nuvara", Path: "/a", AddedAt: now}); err != nil {
		t.Fatal(err)
	}
	if err := s.UpsertProject(domain.ProjectRef{Name: "nuvara", Path: "/b", AddedAt: now}); err != nil {
		t.Fatal(err)
	}
	got, ok, err := s.GetProject("nuvara")
	if err != nil || !ok || got.Path != "/b" || !got.AddedAt.Equal(now) {
		t.Fatalf("got %+v ok=%v err=%v", got, ok, err)
	}
	list, _ := s.ListProjects()
	if len(list) != 1 {
		t.Fatalf("list %+v", list)
	}
	_ = s.DeleteProject("nuvara")
	if _, ok, _ := s.GetProject("nuvara"); ok {
		t.Fatal("not deleted")
	}
}

func TestRunsRoundTrip(t *testing.T) {
	s := openTemp(t)
	exp := time.Now().UTC().Add(time.Minute).Truncate(time.Millisecond)
	r := domain.Run{Project: "p", Service: "api", Kind: domain.KindRun, State: domain.StateRunning, PID: 42, PGID: 42,
		Owner: "agent:x", ExpiresAt: &exp, Ports: map[string]int{"http": 3100}}
	if err := s.SaveRun(r); err != nil {
		t.Fatal(err)
	}
	r.State = domain.StateStopped
	_ = s.SaveRun(r)
	got, ok, err := s.GetRun("p", "api")
	if err != nil || !ok || got.State != domain.StateStopped || got.Ports["http"] != 3100 || !got.ExpiresAt.Equal(exp) {
		t.Fatalf("got %+v", got)
	}
	runs, _ := s.ListRuns()
	if len(runs) != 1 {
		t.Fatalf("runs %+v", runs)
	}
	_ = s.DeleteRun("p", "api")
	if _, ok, _ := s.GetRun("p", "api"); ok {
		t.Fatal("not deleted")
	}
}

func TestComposeRunProfilesRoundTrip(t *testing.T) {
	s := openTemp(t)
	r := domain.Run{Project: "profiles", Service: "reporting", Kind: domain.KindCompose, State: domain.StateRunning,
		ComposeProject: "rocket-profiles-dev", Profiles: []string{"base", "extra", "reports"}}
	if err := s.SaveRun(r); err != nil {
		t.Fatal(err)
	}
	got, ok, err := s.GetRun(r.Project, r.Service)
	if err != nil || !ok || !slices.Equal(got.Profiles, r.Profiles) {
		t.Fatalf("persisted profiles = %+v, ok=%v, error=%v", got, ok, err)
	}
}

func TestLeaseConflicts(t *testing.T) {
	s := openTemp(t)
	l := domain.Lease{Port: 3000, Project: "a", Service: "web", PortName: "http", CreatedAt: time.Now()}
	if err := s.AcquireLease(l); err != nil {
		t.Fatal(err)
	}
	if err := s.AcquireLease(l); err != nil {
		t.Fatalf("re-acquire by same service: %v", err)
	}
	err := s.AcquireLease(domain.Lease{Port: 3000, Project: "b", Service: "web", PortName: "http"})
	if !errors.Is(err, ports.ErrLeaseTaken) {
		t.Fatalf("err %v", err)
	}
	released, _ := s.ReleaseLeases("a", "web")
	if len(released) != 1 || released[0].Port != 3000 {
		t.Fatalf("released %+v", released)
	}
	if leases, _ := s.ListLeases(); len(leases) != 0 {
		t.Fatalf("leases %+v", leases)
	}
}

func TestJobsRoundTripAndOrdering(t *testing.T) {
	s := openTemp(t)
	base := time.Date(2026, 1, 1, 10, 0, 0, 0, time.UTC)
	code := 3
	deadline := base.Add(30 * time.Minute)
	jobs := []domain.Job{
		{ID: "j1", Project: "a", Name: "ci", Kind: domain.JobPipeline, Env: "dev", Owner: "user", Status: domain.JobFailed,
			Steps: []domain.Step{{Task: "lint"}, {Run: "go test ./..."}}, Args: []string{"-v"}, ExitCode: &code,
			StartedAt: base, ExpiresAt: &deadline, LogPath: "/l/j1.log"},
		// 500ms later: RFC3339Nano would sort "10:00:00.5Z" before "10:00:00Z"
		{ID: "j2", Project: "a", Name: "setup", Kind: domain.JobSetup, Owner: "agent:x", Status: domain.JobRunning, StartedAt: base.Add(500 * time.Millisecond)},
		{ID: "j3", Project: "b", Name: "prod", Kind: domain.JobDeploy, Env: "prod", Owner: "user", Status: domain.JobSucceeded, StartedAt: base.Add(time.Second)},
	}
	for _, j := range jobs {
		if err := s.SaveJob(j); err != nil {
			t.Fatal(err)
		}
	}
	got, ok, err := s.GetJob("j1")
	if err != nil || !ok || got.Env != "dev" || got.ExpiresAt == nil || !got.ExpiresAt.Equal(deadline) || got.ExitCode == nil || *got.ExitCode != 3 || len(got.Steps) != 2 || got.Args[0] != "-v" || !got.StartedAt.Equal(base) {
		t.Fatalf("get j1 %+v ok=%v err=%v", got, ok, err)
	}
	all, _ := s.ListJobs("", 0)
	if len(all) != 3 || all[0].ID != "j3" || all[1].ID != "j2" || all[2].ID != "j1" {
		t.Fatalf("all newest first: %+v", all)
	}
	a, _ := s.ListJobs("a", 1)
	if len(a) != 1 || a[0].ID != "j2" {
		t.Fatalf("project a limit 1: %+v", a)
	}
	jobs[1].Status = domain.JobSucceeded
	_ = s.SaveJob(jobs[1])
	if got, _, _ := s.GetJob("j2"); got.Status != domain.JobSucceeded {
		t.Fatalf("update %+v", got)
	}
	if err := s.DeleteJob("j2"); err != nil {
		t.Fatal(err)
	}
	if _, ok, _ := s.GetJob("j2"); ok {
		t.Fatal("j2 not deleted")
	}
}

func TestJobProfilesPersistAcrossReopen(t *testing.T) {
	path := filepath.Join(t.TempDir(), "profiles.db")
	store, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	var job domain.Job
	if err := json.Unmarshal([]byte(`{"id":"jprofile","project":"fixture","env":"smoke","profiles":["base","extra"],"status":"succeeded"}`), &job); err != nil {
		t.Fatal(err)
	}
	if err := store.SaveJob(job); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}
	store, err = Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	got, ok, err := store.GetJob(job.ID)
	if err != nil || !ok {
		t.Fatalf("reopened job: %+v, %v", got, err)
	}
	data, _ := json.Marshal(got)
	var wire struct {
		Profiles []string `json:"profiles"`
	}
	if err := json.Unmarshal(data, &wire); err != nil || !slices.Equal(wire.Profiles, []string{"base", "extra"}) {
		t.Fatalf("reopened profiles = %v, error=%v", wire.Profiles, err)
	}
}
