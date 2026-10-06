//go:build integration && unix

package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/adapters/probe"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
)

var fixturePorts = []int{18431, 18432, 18433, 18531}

type env struct {
	t       *testing.T
	bin     string
	home    string
	project string
}

func setup(t *testing.T) *env {
	t.Helper()
	if _, err := exec.LookPath("python3"); err != nil {
		t.Skip("python3 not installed")
	}
	for _, p := range fixturePorts {
		if !(probe.Ports{}).Free(p) {
			t.Skipf("fixture port %d is busy on this host", p)
		}
	}
	// Short home: unix socket paths are limited to ~104 bytes.
	home, err := os.MkdirTemp("/tmp", "rk")
	if err != nil {
		t.Fatal(err)
	}
	bin := filepath.Join(home, "rocket")
	if prebuilt := os.Getenv("ROCKET_BIN"); prebuilt != "" {
		// Parity runs point the suite at another implementation's binary.
		if !filepath.IsAbs(prebuilt) {
			t.Fatalf("ROCKET_BIN must be an absolute path, got %q", prebuilt)
		}
		if err := copyExecutable(prebuilt, bin); err != nil {
			t.Fatalf("copy ROCKET_BIN: %v", err)
		}
	} else {
		build := exec.Command("go", "build", "-o", bin, ".")
		if out, err := build.CombinedOutput(); err != nil {
			t.Fatalf("build: %v\n%s", err, out)
		}
	}
	project := filepath.Join(home, "fixture")
	if err := os.MkdirAll(project, 0o755); err != nil {
		t.Fatal(err)
	}
	for _, f := range []string{"rocket.yaml", ".env"} {
		data, err := os.ReadFile(filepath.Join("..", "..", "testdata", "fixture", f))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(project, f), data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	e := &env{t: t, bin: bin, home: home, project: project}
	t.Cleanup(func() {
		e.rocket(nil, "down", "--everywhere")
		e.rocket(nil, "daemon", "stop")
		for _, p := range fixturePorts {
			if !(probe.Ports{}).Free(p) {
				t.Errorf("fixture port %d still busy after cleanup", p)
			}
		}
		os.RemoveAll(home)
	})
	return e
}

// rocket runs the CLI in the fixture dir and returns stdout and exit code.
func (e *env) rocket(extraEnv []string, args ...string) (string, int) {
	return e.rocketAt(e.project, extraEnv, args...)
}

// rocketAt runs the CLI in dir; later extraEnv entries override earlier ones.
func (e *env) rocketAt(dir string, extraEnv []string, args ...string) (string, int) {
	cmd := exec.Command(e.bin, args...)
	cmd.Dir = dir
	cmd.Env = append(filterEnv(os.Environ()), append([]string{"ROCKET_HOME=" + e.home}, extraEnv...)...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()
	code := 0
	var ee *exec.ExitError
	if errors.As(err, &ee) {
		code = ee.ExitCode()
	} else if err != nil {
		e.t.Fatalf("run %v: %v", args, err)
	}
	if stderr.Len() > 0 {
		e.t.Logf("rocket %v stderr: %s", args, stderr.String())
	}
	return stdout.String(), code
}

// copyExecutable copies src to dst with the executable bit set. ROCKET_BIN is
// read by the test process only: filterEnv strips ROCKET_* from child envs.
func copyExecutable(src, dst string) error {
	data, err := os.ReadFile(src)
	if err != nil {
		return err
	}
	return os.WriteFile(dst, data, 0o755)
}

func filterEnv(in []string) []string {
	var out []string
	for _, kv := range in {
		if !strings.HasPrefix(kv, "ROCKET_") && !strings.HasPrefix(kv, "FIXTURE_") {
			out = append(out, kv)
		}
	}
	return out
}

func (e *env) json(v any, extraEnv []string, wantCode int, args ...string) {
	e.t.Helper()
	out, code := e.rocket(extraEnv, append(args, "--json")...)
	if code != wantCode {
		e.t.Fatalf("rocket %v: exit %d (want %d)\n%s", args, code, wantCode, out)
	}
	if err := json.Unmarshal([]byte(out), v); err != nil {
		e.t.Fatalf("rocket %v: bad json: %v\n%s", args, err, out)
	}
}

func (e *env) ps() map[string]domain.Run {
	e.t.Helper()
	var res app.StatusResult
	e.json(&res, nil, 0, "ps")
	out := map[string]domain.Run{}
	for _, r := range res.Services {
		out[r.Service] = r
	}
	return out
}

func waitFor(t *testing.T, d time.Duration, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(d)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(200 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func httpOK(port int) bool {
	c := http.Client{Timeout: time.Second}
	resp, err := c.Get("http://127.0.0.1:" + strconv.Itoa(port) + "/")
	if err != nil {
		return false
	}
	resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

func TestEndToEnd(t *testing.T) {
	e := setup(t)

	t.Run("up all starts everything healthy", func(t *testing.T) {
		var res app.UpResult
		e.json(&res, nil, 0, "up", "all")
		if len(res.Services) != 4 {
			t.Fatalf("unexpected result %+v", res)
		}
		for _, s := range res.Services {
			if s.Action != app.ActionStarted || s.State != domain.StateRunning {
				t.Fatalf("%s: %s/%s %s", s.Service, s.Action, s.State, s.Error)
			}
		}
		if !httpOK(18431) || !httpOK(18432) || !httpOK(18433) {
			t.Fatal("fixture servers not reachable")
		}
		var again app.UpResult
		e.json(&again, nil, 0, "up", "all")
		for _, s := range again.Services {
			if s.Action != app.ActionAlreadyRunning {
				t.Fatalf("second up: %s %s", s.Service, s.Action)
			}
		}
	})

	t.Run("dotenv reaches the child and logs are captured", func(t *testing.T) {
		waitFor(t, 5*time.Second, "sleeper log", func() bool {
			var logs app.LogsResult
			e.json(&logs, nil, 0, "logs", "sleeper", "--tail", "20")
			return strings.Contains(strings.Join(logs.Lines, "\n"), "greeting=hello")
		})
	})

	t.Run("ports lists global leases", func(t *testing.T) {
		var res app.PortsResult
		e.json(&res, nil, 0, "ports")
		got := map[int]string{}
		for _, p := range res.Ports {
			got[p.Port] = p.Service
		}
		if got[18431] != "static" || got[18432] != "web" || got[18433] != "fixed" {
			t.Fatalf("ports %+v", res.Ports)
		}
	})

	t.Run("down all frees every port", func(t *testing.T) {
		var res app.DownResult
		e.json(&res, nil, 0, "down", "--all")
		if len(res.Stopped) != 4 || res.Stopped[len(res.Stopped)-1].Service == "sleeper" {
			t.Fatalf("stopped %+v", res.Stopped)
		}
		for _, p := range []int{18431, 18432, 18433} {
			if !(probe.Ports{}).Free(p) {
				t.Fatalf("port %d still busy", p)
			}
		}
		var ports app.PortsResult
		e.json(&ports, nil, 0, "ports")
		if len(ports.Ports) != 0 {
			t.Fatalf("leases left %+v", ports.Ports)
		}
	})

	t.Run("busy port is remapped via env, or fails without env", func(t *testing.T) {
		l1, err := net.Listen("tcp", "127.0.0.1:18431")
		if err != nil {
			t.Fatal(err)
		}
		defer l1.Close()
		var res app.UpResult
		e.json(&res, nil, 0, "up", "static")
		s := res.Services[0]
		if s.Ports["http"] != 18531 || len(s.Remaps) != 1 || s.Remaps[0].Holder == nil || s.Remaps[0].Holder.PID != os.Getpid() {
			t.Fatalf("remap %+v", s)
		}
		if !httpOK(18531) {
			t.Fatal("remapped server not reachable")
		}

		l2, err := net.Listen("tcp", "127.0.0.1:18433")
		if err != nil {
			t.Fatal(err)
		}
		defer l2.Close()
		var failed app.UpResult
		e.json(&failed, nil, 2, "up", "fixed")
		if failed.Services[0].Action != app.ActionFailed || !strings.Contains(failed.Services[0].Error, "no env var") {
			t.Fatalf("fixed %+v", failed.Services[0])
		}
		var down app.DownResult
		e.json(&down, nil, 0, "down", "--all")
	})

	t.Run("owners and ttl", func(t *testing.T) {
		var res app.UpResult
		e.json(&res, nil, 0, "up", "static")
		e.json(&res, []string{"ROCKET_OWNER=agent:t2"}, 0, "up", "web")
		e.json(&res, []string{"ROCKET_OWNER=agent:t1"}, 0, "up", "sleeper", "--ttl", "2s")
		ps := e.ps()
		if ps["static"].Owner != "user" || ps["web"].Owner != "agent:t2" || ps["sleeper"].Owner != "agent:t1" {
			t.Fatalf("owners %+v", ps)
		}
		var down app.DownResult
		e.json(&down, nil, 0, "down", "--owner", "agent:t2")
		if len(down.Stopped) != 1 || down.Stopped[0].Service != "web" {
			t.Fatalf("owner down %+v", down.Stopped)
		}
		waitFor(t, 15*time.Second, "ttl expiry", func() bool { return e.ps()["sleeper"].State == domain.StateStopped })
		if e.ps()["static"].State != domain.StateRunning {
			t.Fatal("user's static must survive agent cleanup")
		}
	})

	t.Run("daemon crash: reconcile adopts live and marks dead", func(t *testing.T) {
		var res app.UpResult
		e.json(&res, nil, 0, "up", "web")
		ps := e.ps()
		webPID := ps["web"].PID
		var st DaemonStatus
		e.json(&st, nil, 0, "daemon", "status")
		if err := syscall.Kill(st.Info.PID, syscall.SIGKILL); err != nil {
			t.Fatal(err)
		}
		_ = syscall.Kill(-webPID, syscall.SIGKILL) // service dies while the daemon is down
		waitFor(t, 5*time.Second, "daemon gone", func() bool {
			_, code := e.rocket(nil, "daemon", "status")
			return code == 1
		})
		e.json(&st, nil, 0, "daemon", "start")
		ps = e.ps()
		if ps["static"].State != domain.StateRunning || !httpOK(18431) {
			t.Fatalf("static should be adopted: %+v", ps["static"])
		}
		if ps["web"].State != domain.StateDead {
			t.Fatalf("web should be dead: %+v", ps["web"])
		}
		var gc app.GCResult
		e.json(&gc, nil, 0, "gc")
		var down app.DownResult
		e.json(&down, nil, 0, "down", "--everywhere")
		if len(down.Stopped) != 1 || down.Stopped[0].Service != "static" {
			t.Fatalf("down everywhere %+v", down.Stopped)
		}
		for _, p := range fixturePorts {
			if !(probe.Ports{}).Free(p) {
				t.Fatalf("port %d still busy", p)
			}
		}
	})
}
