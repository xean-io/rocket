//go:build integration

package api

import (
	"bufio"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/adapters/sqlite"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
)

func TestJobFollowHeartbeatAndTrailingTerminalOrdering(t *testing.T) {
	if testing.Short() {
		t.Skip("15-second SSE heartbeat integration")
	}
	dir := t.TempDir()
	store, err := sqlite.Open(filepath.Join(dir, "state.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	application := app.New(app.Deps{Store: store})
	defer application.Close()
	path := filepath.Join(dir, "job.log")
	if err := os.WriteFile(path, []byte("initial\npartial"), 0o600); err != nil {
		t.Fatal(err)
	}
	job := domain.Job{ID: "jstream", Project: "fixture", Status: domain.JobRunning, StartedAt: time.Now(), LogPath: path}
	if err := store.SaveJob(job); err != nil {
		t.Fatal(err)
	}
	server := httptest.NewServer((&Server{App: application}).Handler())
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 18*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodGet, server.URL+"/v1/jobs/jstream/logs?follow=true&tail=1", nil)
	started := time.Now()
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	scanner := bufio.NewScanner(response.Body)
	var lines []string
	ping := false
	for scanner.Scan() {
		line := scanner.Text()
		lines = append(lines, line)
		if line == ": ping" {
			ping = true
			break
		}
	}
	if !ping {
		t.Fatalf("job follow omitted : ping heartbeat: %v (scanner=%v)", lines, scanner.Err())
	}
	if elapsed := time.Since(started); elapsed < 14*time.Second || elapsed > 17*time.Second {
		t.Fatalf("heartbeat interval = %s, want 15s", elapsed)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	f.WriteString("-end")
	f.Close()
	job.Status = domain.JobSucceeded
	if err := store.SaveJob(job); err != nil {
		t.Fatal(err)
	}
	for scanner.Scan() {
		lines = append(lines, scanner.Text())
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	var events []domain.Event
	terminals := 0
	for _, line := range lines {
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		var event domain.Event
		if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &event); err != nil {
			t.Fatalf("interleaved SSE frame: %q: %v", line, err)
		}
		events = append(events, event)
		if event.Type == domain.EventJobState && event.Status.Terminal() {
			terminals++
		}
	}
	if terminals != 1 || len(events) != 3 || events[0].Line != "initial" || events[1].Line != "partial-end" || events[2].Status != domain.JobSucceeded {
		t.Fatalf("trailing logs/one terminal ordering: %+v", events)
	}
	lastData := -1
	for i, line := range lines {
		if strings.HasPrefix(line, "data: ") {
			lastData = i
		}
	}
	for _, line := range lines[lastData+1:] {
		if strings.TrimSpace(line) != "" {
			t.Fatalf("frame after terminal: %q", line)
		}
	}
}
