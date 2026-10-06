//go:build integration

package probe

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strconv"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func TestHolderReportsListeningProcess(t *testing.T) {
	if testing.Short() {
		t.Skip("listener/lsof integration")
	}
	if _, err := exec.LookPath("lsof"); err != nil {
		t.Skip("lsof not installed")
	}
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	port := l.Addr().(*net.TCPAddr).Port
	h, err := (Ports{}).Holder(port)
	if err != nil || h == nil {
		t.Fatalf("holder %v err %v", h, err)
	}
	if h.PID != os.Getpid() {
		t.Fatalf("pid %d want %d", h.PID, os.Getpid())
	}
	wd, _ := os.Getwd()
	if h.Cwd == "" || h.Command == "" {
		t.Fatalf("holder missing details: %+v (wd %s)", h, wd)
	}
}

func TestFreeDetectsListener(t *testing.T) {
	if testing.Short() {
		t.Skip("listener integration")
	}
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	port := l.Addr().(*net.TCPAddr).Port
	if (Ports{}).Free(port) {
		t.Fatalf("port %d reported free while listening", port)
	}
	l.Close()
	if !(Ports{}).Free(port) {
		t.Fatalf("port %d reported busy after close", port)
	}
}

func TestHealthChecks(t *testing.T) {
	if testing.Short() {
		t.Skip("HTTP/TCP integration")
	}
	ok := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/broken" {
			w.WriteHeader(http.StatusServiceUnavailable)
		}
	}))
	defer ok.Close()
	_, portStr, _ := net.SplitHostPort(ok.Listener.Addr().String())
	port, _ := strconv.Atoi(portStr)
	ctx := context.Background()
	if err := (Health{}).Check(ctx, domain.HealthCheck{Kind: "http", Port: port, Path: "/live"}); err != nil {
		t.Fatalf("http ok: %v", err)
	}
	if err := (Health{}).Check(ctx, domain.HealthCheck{Kind: "http", Port: port, Path: "/broken"}); err == nil {
		t.Fatal("503 must fail")
	}
	if err := (Health{}).Check(ctx, domain.HealthCheck{Kind: "tcp", Port: port}); err != nil {
		t.Fatalf("tcp: %v", err)
	}
	ok.Close()
	if err := (Health{}).Check(ctx, domain.HealthCheck{Kind: "tcp", Port: port}); err == nil {
		t.Fatal("closed port must fail")
	}
}
