//go:build integration

package api

import (
	"bufio"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/adapters/events"
	"github.com/xean-io/rocket/internal/domain"
)

func TestEventsStreamFiltersAndFormatsSSE(t *testing.T) {
	if testing.Short() {
		t.Skip("SSE network integration")
	}
	bus := events.New()
	srv := httptest.NewServer((&Server{Bus: bus, Info: HealthInfo{OK: true, API: Version}}).Handler())
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, srv.URL+"/v1/events?project=p&types=service.state", nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if ct := resp.Header.Get("Content-Type"); ct != "text/event-stream" {
		t.Fatalf("content-type %q", ct)
	}
	go func() {
		time.Sleep(50 * time.Millisecond)
		bus.Publish(domain.Event{Type: domain.EventLogLine, Project: "p", Line: "filtered by type"})
		bus.Publish(domain.Event{Type: domain.EventServiceState, Project: "other", State: domain.StateRunning})
		bus.Publish(domain.Event{Type: domain.EventServiceState, Project: "p", Service: "api", State: domain.StateRunning})
	}()
	sc := bufio.NewScanner(resp.Body)
	var lines []string
	for sc.Scan() {
		lines = append(lines, sc.Text())
		if strings.HasPrefix(sc.Text(), "data: ") {
			break
		}
	}
	// Clients like URLSession only surface the response once body bytes arrive.
	if len(lines) == 0 || lines[0] != ": ok" {
		t.Fatalf("stream must open with an ': ok' comment, got %q", lines)
	}
	if len(lines) < 2 || lines[len(lines)-2] != "event: service.state" {
		t.Fatalf("lines %q", lines)
	}
	var e domain.Event
	if err := json.Unmarshal([]byte(strings.TrimPrefix(lines[len(lines)-1], "data: ")), &e); err != nil {
		t.Fatal(err)
	}
	if e.Project != "p" || e.Service != "api" || e.State != domain.StateRunning {
		t.Fatalf("event %+v", e)
	}
}

func TestEventsFilterByJob(t *testing.T) {
	if testing.Short() {
		t.Skip("SSE network integration")
	}
	bus := events.New()
	srv := httptest.NewServer((&Server{Bus: bus}).Handler())
	defer srv.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, srv.URL+"/v1/events?job=j2", nil)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	go func() {
		time.Sleep(50 * time.Millisecond)
		bus.Publish(domain.Event{Type: domain.EventJobLog, Project: "p", JobID: "j1", Line: "other job"})
		bus.Publish(domain.Event{Type: domain.EventJobLog, Project: "p", JobID: "j2", Line: "mine"})
	}()
	sc := bufio.NewScanner(resp.Body)
	var lines []string
	for sc.Scan() {
		lines = append(lines, sc.Text())
		if strings.HasPrefix(sc.Text(), "data: ") {
			break
		}
	}
	if len(lines) < 2 || lines[len(lines)-2] != "event: job.log" || !strings.Contains(lines[len(lines)-1], `"line":"mine"`) {
		t.Fatalf("lines %q", lines)
	}
}
