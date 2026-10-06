package events

import (
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func TestPublishSubscribe(t *testing.T) {
	b := New()
	ch, cancel := b.Subscribe(1)
	b.Publish(domain.Event{Type: domain.EventLogLine, Line: "a"})
	b.Publish(domain.Event{Type: domain.EventLogLine, Line: "dropped"}) // buffer full: must not block
	if e := <-ch; e.Line != "a" {
		t.Fatalf("got %+v", e)
	}
	cancel()
	cancel()
	if _, open := <-ch; open {
		t.Fatal("channel should be closed after cancel")
	}
	b.Publish(domain.Event{Type: domain.EventLogLine}) // no subscribers: no panic
}
