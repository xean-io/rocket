//go:build integration && unix

package main

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/agentdocs"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/daemon"
	"github.com/xean-io/rocket/internal/domain"
)

func (e *env) waitJobPGID(id string) domain.Job {
	e.t.Helper()
	var j domain.Job
	waitFor(e.t, 10*time.Second, "job process", func() bool {
		e.json(&j, nil, 0, "job", id)
		return j.PGID > 0
	})
	return j
}

func groupGone(pgid int) bool {
	return errors.Is(syscall.Kill(-pgid, 0), syscall.ESRCH)
}

func tailHas(lines []string, s string) bool {
	return strings.Contains(strings.Join(lines, "\n"), s)
}

func copyTree(t *testing.T, src, dst string) {
	t.Helper()
	err := filepath.WalkDir(src, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(src, path)
		if d.IsDir() {
			return os.MkdirAll(filepath.Join(dst, rel), 0o755)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		return os.WriteFile(filepath.Join(dst, rel), data, 0o644)
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestJobsAndAI(t *testing.T) {
	e := setup(t)

	t.Run("pipeline success returns the outcome", func(t *testing.T) {
		var out app.JobOutcome
		e.json(&out, nil, 0, "run", "check")
		if out.Status != domain.JobSucceeded || out.ExitCode == nil || *out.ExitCode != 0 || out.Kind != domain.JobPipeline {
			t.Fatalf("outcome %+v", out)
		}
		if !tailHas(out.Tail, "step-one") || !tailHas(out.Tail, "step-two greeting=hello") {
			t.Fatalf("tail %q", out.Tail)
		}
		if _, err := os.Stat(out.LogPath); err != nil {
			t.Fatalf("log %s: %v", out.LogPath, err)
		}
	})

	t.Run("pipeline failure exits 2 and stops at the failing step", func(t *testing.T) {
		var out app.JobOutcome
		e.json(&out, nil, 2, "run", "broken")
		if out.Status != domain.JobFailed || out.ExitCode == nil || *out.ExitCode != 3 {
			t.Fatalf("outcome %+v", out)
		}
		if !tailHas(out.Tail, "failing") || tailHas(out.Tail, "never-runs") {
			t.Fatalf("tail %q", out.Tail)
		}
	})

	t.Run("args after -- reach the last step", func(t *testing.T) {
		stdout, code := e.rocket(nil, "run", "echo-args", "--json", "--", "a", "b c")
		var out app.JobOutcome
		if err := json.Unmarshal([]byte(stdout), &out); err != nil || code != 0 {
			t.Fatalf("exit %d err %v\n%s", code, err, stdout)
		}
		if !tailHas(out.Tail, "args: a b c") {
			t.Fatalf("tail %q", out.Tail)
		}
	})

	t.Run("unknown pipeline without a Taskfile is invalid", func(t *testing.T) {
		var body map[string]string
		e.json(&body, nil, 1, "run", "nope")
		if body["code"] != "invalid" || !strings.Contains(body["error"], "no Taskfile") {
			t.Fatalf("body %v", body)
		}
	})

	t.Run("setup runs doctor then install", func(t *testing.T) {
		var out app.JobOutcome
		e.json(&out, nil, 0, "setup")
		joined := strings.Join(out.Tail, "\n")
		if out.Name != "setup" || strings.Index(joined, "doctor-ok") > strings.Index(joined, "install-ok greeting=hello") ||
			!strings.Contains(joined, "doctor-ok") {
			t.Fatalf("setup %+v", out)
		}
		e.json(&out, nil, 0, "doctor")
		if out.Name != "doctor" || !tailHas(out.Tail, "doctor-ok") {
			t.Fatalf("doctor %+v", out)
		}
	})

	t.Run("human run streams the log", func(t *testing.T) {
		stdout, code := e.rocket(nil, "run", "check")
		if code != 0 || !strings.Contains(stdout, "step-one\n") || !strings.Contains(stdout, "step-two greeting=hello") {
			t.Fatalf("exit %d stdout:\n%s", code, stdout)
		}
	})

	t.Run("detach, list and cancel", func(t *testing.T) {
		var job domain.Job
		e.json(&job, nil, 0, "run", "slow", "--detach")
		if job.Status != domain.JobRunning || job.ID == "" {
			t.Fatalf("detached %+v", job)
		}
		running := e.waitJobPGID(job.ID)
		var list app.JobsResult
		e.json(&list, nil, 0, "jobs")
		if len(list.Jobs) == 0 || list.Jobs[0].ID != job.ID || list.Jobs[0].Status != domain.JobRunning {
			t.Fatalf("jobs %+v", list.Jobs)
		}
		var canceled domain.Job
		e.json(&canceled, nil, 0, "job", "cancel", job.ID)
		if canceled.Status != domain.JobCanceled {
			t.Fatalf("canceled %+v", canceled)
		}
		if !groupGone(running.PGID) {
			t.Fatalf("process group %d still alive", running.PGID)
		}
		var logs app.JobLogsResult
		e.json(&logs, nil, 0, "job", job.ID, "logs")
		if !tailHas(logs.Lines, "slow-start") {
			t.Fatalf("logs %q", logs.Lines)
		}
	})

	t.Run("deploy confirmation gate exits 3", func(t *testing.T) {
		var body map[string]string
		e.json(&body, nil, 3, "deploy", "stage")
		if body["code"] != "confirmation_required" {
			t.Fatalf("body %v", body)
		}
		var out app.JobOutcome
		e.json(&out, nil, 0, "deploy", "stage", "--yes")
		if out.Kind != domain.JobDeploy || !tailHas(out.Tail, "deploying greeting=hello") {
			t.Fatalf("deploy %+v", out)
		}
		agent := []string{"ROCKET_OWNER=agent:t9"}
		e.json(&body, agent, 3, "deploy", "stage", "--yes")
		e.json(&out, append(agent, "ROCKET_ALLOW_DEPLOY=1"), 0, "deploy", "stage", "--yes")
		if out.Status != domain.JobSucceeded {
			t.Fatalf("allowed agent deploy %+v", out)
		}
	})

	t.Run("status summary and owner cleanup of jobs", func(t *testing.T) {
		var up app.UpResult
		e.json(&up, nil, 0, "up", "static")
		agent := []string{"ROCKET_OWNER=agent:t3"}
		var job domain.Job
		e.json(&job, agent, 0, "run", "slow", "--detach")
		running := e.waitJobPGID(job.ID)

		var sum app.Summary
		e.json(&sum, nil, 0, "status")
		if sum.Project == nil || sum.Project.Name != "rocket-fixture" || len(sum.Jobs) != 1 || sum.Jobs[0].ID != job.ID || sum.Conflicts == nil {
			t.Fatalf("summary %+v", sum)
		}
		states := map[string]domain.RunState{}
		for _, r := range sum.Services {
			states[r.Service] = r.State
		}
		if states["static"] != domain.StateRunning || states["web"] != domain.StateStopped {
			t.Fatalf("service states %v", states)
		}
		var down app.DownResult
		e.json(&down, nil, 0, "down", "--owner", "agent:t3")
		if !slices.Equal(down.CanceledJobs, []string{job.ID}) || len(down.Stopped) != 0 {
			t.Fatalf("owner down %+v", down)
		}
		if !groupGone(running.PGID) {
			t.Fatal("agent job still running")
		}
		e.json(&down, nil, 0, "down", "--all")
	})

	t.Run("tcp listener requires the bearer token", func(t *testing.T) {
		path := filepath.Join(e.home, "daemon.json")
		st, err := os.Stat(path)
		if err != nil || st.Mode().Perm() != 0o600 {
			t.Fatalf("daemon.json %v perm %v", err, st)
		}
		info, err := daemon.ReadInfo(path)
		if err != nil || !strings.HasPrefix(info.HTTP, "http://127.0.0.1:") || len(info.Token) != 64 || info.RocketBin == "" {
			t.Fatalf("info %+v err %v", info, err)
		}
		get := func(token string) int {
			req, _ := http.NewRequest(http.MethodGet, info.HTTP+"/v1/health", nil)
			if token != "" {
				req.Header.Set("Authorization", "Bearer "+token)
			}
			resp, err := http.DefaultClient.Do(req)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			_, _ = io.Copy(io.Discard, resp.Body)
			return resp.StatusCode
		}
		if got := get(""); got != http.StatusUnauthorized {
			t.Fatalf("no token: %d", got)
		}
		if got := get(strings.Repeat("0", 64)); got != http.StatusUnauthorized {
			t.Fatalf("wrong token: %d", got)
		}
		if got := get(info.Token); got != http.StatusOK {
			t.Fatalf("valid token: %d", got)
		}
	})

	t.Run("daemon crash: vanished job is marked lost", func(t *testing.T) {
		var job domain.Job
		e.json(&job, nil, 0, "run", "slow", "--detach")
		running := e.waitJobPGID(job.ID)
		var st DaemonStatus
		e.json(&st, nil, 0, "daemon", "status")
		if err := syscall.Kill(st.Info.PID, syscall.SIGKILL); err != nil {
			t.Fatal(err)
		}
		_ = syscall.Kill(-running.PGID, syscall.SIGKILL)
		waitFor(t, 5*time.Second, "daemon gone", func() bool {
			_, code := e.rocket(nil, "daemon", "status")
			return code == 1
		})
		e.json(&st, nil, 0, "daemon", "start")
		var got domain.Job
		e.json(&got, nil, 0, "job", job.ID)
		if got.Status != domain.JobLost {
			t.Fatalf("job after crash %+v", got)
		}
	})

	t.Run("init on an autodropshipping-shaped project", func(t *testing.T) {
		dir := filepath.Join(e.home, "autodropshipping")
		copyTree(t, filepath.Join("..", "..", "testdata", "init", "autodropshipping"), dir)
		var res InitResult
		out, code := e.rocketAt(dir, nil, "init", "--json")
		if code != 0 || json.Unmarshal([]byte(out), &res) != nil || !res.Written {
			t.Fatalf("init exit %d\n%s", code, out)
		}
		if _, err := os.Stat(filepath.Join(dir, "rocket.yaml")); err != nil {
			t.Fatal(err)
		}
		if out, code := e.rocketAt(dir, nil, "init", "--json"); code != 1 || !strings.Contains(out, "already exists") {
			t.Fatalf("second init must refuse: exit %d\n%s", code, out)
		}
		if _, code := e.rocketAt(dir, nil, "init", "--force"); code != 0 {
			t.Fatalf("init --force exit %d", code)
		}
		if out, code := e.rocketAt(dir, nil, "init", "--print"); code != 0 || !strings.Contains(out, "compose: postgres") {
			t.Fatalf("init --print exit %d\n%s", code, out)
		}
		// The daemon loads and validates the generated manifest.
		var ps app.StatusResult
		e.json(&ps, nil, 0, "ps", "-p", dir)
		names := map[string]bool{}
		for _, r := range ps.Services {
			names[r.Service] = r.State == domain.StateStopped
		}
		if !names["postgres"] || !names["cp-dev"] {
			t.Fatalf("services from generated manifest %+v", ps.Services)
		}
	})

	t.Run("agent install with HOME override is idempotent", func(t *testing.T) {
		home := filepath.Join(e.home, "home")
		if err := os.MkdirAll(home, 0o755); err != nil {
			t.Fatal(err)
		}
		henv := []string{"HOME=" + home}
		var res AgentInstallResult
		out, code := e.rocketAt(e.project, henv, "agent", "install", "--global", "--json")
		if code != 0 || json.Unmarshal([]byte(out), &res) != nil || len(res.Actions) != 3 {
			t.Fatalf("install exit %d\n%s", code, out)
		}
		if _, err := os.Stat(filepath.Join(home, ".claude", "skills", "rocket", "SKILL.md")); err != nil {
			t.Fatalf("global skill: %v", err)
		}
		for _, f := range []string{"AGENTS.md", "CLAUDE.md"} {
			data, err := os.ReadFile(filepath.Join(e.project, f))
			if err != nil || strings.Count(string(data), agentdocs.BeginMarker) != 1 {
				t.Fatalf("%s: %v\n%s", f, err, data)
			}
		}
		out, _ = e.rocketAt(e.project, henv, "agent", "install", "--global", "--json")
		_ = json.Unmarshal([]byte(out), &res)
		for _, a := range res.Actions {
			if a.Action != "unchanged" {
				t.Fatalf("second install changed %+v", a)
			}
		}
		if _, code := e.rocketAt(e.project, henv, "agent", "install", "--target", "claude"); code != 0 {
			t.Fatal("project skill install failed")
		}
		if _, err := os.Stat(filepath.Join(e.project, ".claude", "skills", "rocket", "SKILL.md")); err != nil {
			t.Fatalf("project skill: %v", err)
		}
	})

	t.Run("daemon stop removes daemon.json", func(t *testing.T) {
		if _, code := e.rocket(nil, "daemon", "stop"); code != 0 {
			t.Fatalf("daemon stop exit %d", code)
		}
		if _, err := os.Stat(filepath.Join(e.home, "daemon.json")); !os.IsNotExist(err) {
			t.Fatalf("daemon.json left behind: %v", err)
		}
	})
}
