// Package events is an in-memory, non-blocking pub/sub for daemon events.
package events

import (
	"sync"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// Bus implements ports.EventBus. Slow subscribers drop events rather than
// blocking publishers.
type Bus struct {
	mu   sync.Mutex
	next int
	subs map[int]chan domain.Event
}

var _ ports.EventBus = (*Bus)(nil)

// New returns an empty bus.
func New() *Bus { return &Bus{subs: map[int]chan domain.Event{}} }

// Publish delivers e to every subscriber with buffer space.
func (b *Bus) Publish(e domain.Event) {
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, ch := range b.subs {
		select {
		case ch <- e:
		default:
		}
	}
}

// Subscribe registers a subscriber; call cancel to unsubscribe.
func (b *Bus) Subscribe(buffer int) (<-chan domain.Event, func()) {
	if buffer <= 0 {
		buffer = 256
	}
	ch := make(chan domain.Event, buffer)
	b.mu.Lock()
	id := b.next
	b.next++
	b.subs[id] = ch
	b.mu.Unlock()
	var once sync.Once
	return ch, func() {
		once.Do(func() {
			b.mu.Lock()
			delete(b.subs, id)
			close(ch)
			b.mu.Unlock()
		})
	}
}
