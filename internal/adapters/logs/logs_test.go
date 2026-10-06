package logs

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/adapters/events"
	"github.com/xean-io/rocket/internal/domain"
)

func TestOpenFollowTailAndEvents(t *testing.T) {
	bus := events.New()
	ch, cancel := bus.Subscribe(100)
	defer cancel()
	s := New(t.TempDir(), bus)
	s.interval = 10 * time.Millisecond
	s.ringSize = 3
	defer s.Close()

	f, path, err := s.Open("proj", "api")
	if err != nil {
		t.Fatal(err)
	}
	if path != s.Path("proj", "api") {
		t.Fatalf("path %s", path)
	}
	for i := 1; i <= 4; i++ {
		fmt.Fprintf(f, "line %d\n", i)
	}
	fmt.Fprint(f, "partial")
	f.Close()

	var got []string
	deadline := time.After(2 * time.Second)
	for len(got) < 4 {
		select {
		case e := <-ch:
			if e.Type != domain.EventLogLine || e.Project != "proj" || e.Service != "api" {
				t.Fatalf("event %+v", e)
			}
			got = append(got, e.Line)
		case <-deadline:
			t.Fatalf("got only %v", got)
		}
	}
	if !slices.Equal(got, []string{"line 1", "line 2", "line 3", "line 4"}) {
		t.Fatalf("events %v", got)
	}
	tail, _ := s.Tail("proj", "api", 2)
	if !slices.Equal(tail, []string{"line 3", "line 4"}) {
		t.Fatalf("ring tail %v", tail)
	}
	// more than the ring holds -> falls back to the file
	tail, _ = s.Tail("proj", "api", 10)
	if !slices.Equal(tail, []string{"line 1", "line 2", "line 3", "line 4", "partial"}) {
		t.Fatalf("file tail %v", tail)
	}
}

func TestJobLogPublishesJobEventsAndFlushesOnClose(t *testing.T) {
	bus := events.New()
	ch, cancel := bus.Subscribe(100)
	defer cancel()
	s := New(t.TempDir(), bus)
	s.interval = time.Hour // only CloseJob's final poll publishes
	defer s.Close()

	f, path, err := s.OpenJob("proj", "j1")
	if err != nil {
		t.Fatal(err)
	}
	if path != s.JobPath("proj", "j1") {
		t.Fatalf("path %s", path)
	}
	fmt.Fprint(f, "one\ntwo\n")
	f.Close()
	s.CloseJob("proj", "j1")

	var got []string
	for len(got) < 2 {
		select {
		case e := <-ch:
			if e.Type != domain.EventJobLog || e.JobID != "j1" || e.Project != "proj" || e.Service != "" {
				t.Fatalf("event %+v", e)
			}
			got = append(got, e.Line)
		case <-time.After(2 * time.Second):
			t.Fatalf("got only %v", got)
		}
	}
	tail, err := s.TailJob("proj", "j1", 1)
	if err != nil || !slices.Equal(tail, []string{"two"}) {
		t.Fatalf("tail %v %v", tail, err)
	}
	if err := s.RemoveJob("proj", "j1"); err != nil {
		t.Fatal(err)
	}
	if _, err := s.TailJob("proj", "j1", 1); err == nil {
		t.Fatal("log not removed")
	}
}

func TestCloseJobFlushesTrailingPartial(t *testing.T) {
	bus := events.New()
	ch, cancel := bus.Subscribe(10)
	defer cancel()
	s := New(t.TempDir(), bus)
	s.interval = time.Hour
	defer s.Close()
	f, _, err := s.OpenJob("p", "jpartial")
	if err != nil {
		t.Fatal(err)
	}
	f.WriteString("partial-without-newline")
	f.Close()
	s.CloseJob("p", "jpartial")
	select {
	case event := <-ch:
		if event.Type != domain.EventJobLog || event.Line != "partial-without-newline" {
			t.Fatalf("trailing event = %+v", event)
		}
	default:
		t.Fatal("CloseJob dropped the trailing partial line")
	}
}

func TestFollowFileStreamsUntilFinishedAndDrained(t *testing.T) {
	path := filepath.Join(t.TempDir(), "j.log")
	if err := os.WriteFile(path, []byte("a\nb\nc\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	var finished atomic.Bool
	go func() {
		time.Sleep(30 * time.Millisecond)
		f, _ := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0)
		fmt.Fprint(f, "d\ne-no-newline")
		f.Close()
		finished.Store(true)
	}()
	var got []string
	err := FollowFile(context.Background(), path, 2, 5*time.Millisecond, finished.Load, func(l string) error {
		got = append(got, l)
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(got, []string{"b", "c", "d", "e-no-newline"}) {
		t.Fatalf("lines %q", got)
	}

	got = nil
	_ = FollowFile(context.Background(), path, -1, time.Millisecond, func() bool { return true }, func(l string) error {
		got = append(got, l)
		return nil
	})
	if len(got) != 5 || got[0] != "a" {
		t.Fatalf("whole file %q", got)
	}
}
