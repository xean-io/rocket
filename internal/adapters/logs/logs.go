// Package logs stores service output in files and keeps an in-memory ring
// buffer per service, publishing new lines on the event bus.
//
// Children write straight into the log file (no pipe through the daemon), so
// they keep running and logging even if the daemon restarts.
package logs

import (
	"bytes"
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// Sink implements ports.LogSink.
type Sink struct {
	dir      string
	bus      ports.EventBus
	ringSize int
	interval time.Duration

	mu        sync.Mutex
	followers map[string]*follower
	stop      chan struct{}
	wg        sync.WaitGroup
}

var _ ports.LogSink = (*Sink)(nil)

type follower struct {
	project, service, path string
	job                    string // set for job logs: publish job.log events
	offset                 int64
	partial                string
	quit                   chan struct{}
	done                   chan struct{}

	mu   sync.Mutex
	ring []string
	next int
	full bool
}

// New creates a sink rooted at dir (e.g. ~/.rocket/logs).
func New(dir string, bus ports.EventBus) *Sink {
	return &Sink{dir: dir, bus: bus, ringSize: 2000, interval: 200 * time.Millisecond,
		followers: map[string]*follower{}, stop: make(chan struct{})}
}

// Path returns the log file of a service.
func (s *Sink) Path(project, service string) string {
	return filepath.Join(s.dir, project, service+".log")
}

// JobPath returns the log file of a job.
func (s *Sink) JobPath(project, jobID string) string {
	return filepath.Join(s.dir, project, "jobs", jobID+".log")
}

func jobKey(project, jobID string) string { return "job:" + project + "/" + jobID }

// Open returns the append-mode log file and starts following it.
func (s *Sink) Open(project, service string) (*os.File, string, error) {
	return s.open(project+"/"+service, &follower{project: project, service: service, path: s.Path(project, service)})
}

// OpenJob is Open for a job log; new lines are published as job.log events.
func (s *Sink) OpenJob(project, jobID string) (*os.File, string, error) {
	return s.open(jobKey(project, jobID), &follower{project: project, job: jobID, path: s.JobPath(project, jobID)})
}

// TailJob returns the last n lines of a job log.
func (s *Sink) TailJob(project, jobID string, n int) ([]string, error) {
	return tailFile(s.JobPath(project, jobID), n)
}

// CloseJob performs a final poll (publishing remaining lines) and stops
// following the job log.
func (s *Sink) CloseJob(project, jobID string) {
	s.mu.Lock()
	fl := s.followers[jobKey(project, jobID)]
	delete(s.followers, jobKey(project, jobID))
	s.mu.Unlock()
	if fl != nil {
		close(fl.quit)
		<-fl.done
	}
}

// RemoveJob stops following and deletes a job log.
func (s *Sink) RemoveJob(project, jobID string) error {
	s.CloseJob(project, jobID)
	err := os.Remove(s.JobPath(project, jobID))
	if os.IsNotExist(err) {
		return nil
	}
	return err
}

func (s *Sink) open(k string, fl *follower) (*os.File, string, error) {
	if err := os.MkdirAll(filepath.Dir(fl.path), 0o700); err != nil {
		return nil, "", err
	}
	f, err := os.OpenFile(fl.path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return nil, "", err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.followers[k]; ok {
		return f, fl.path, nil
	}
	if st, err := f.Stat(); err == nil {
		fl.offset = st.Size()
	}
	fl.ring = make([]string, s.ringSize)
	fl.quit, fl.done = make(chan struct{}), make(chan struct{})
	s.followers[k] = fl
	s.wg.Add(1)
	go s.run(fl)
	return f, fl.path, nil
}

func (s *Sink) run(fl *follower) {
	defer s.wg.Done()
	defer close(fl.done)
	t := time.NewTicker(s.interval)
	defer t.Stop()
	for {
		select {
		case <-s.stop:
			s.poll(fl)
			s.flushPartial(fl)
			return
		case <-fl.quit:
			s.poll(fl)
			s.flushPartial(fl)
			return
		case <-t.C:
			s.poll(fl)
		}
	}
}

func (s *Sink) flushPartial(fl *follower) {
	if fl.partial != "" {
		s.publishLine(fl, strings.TrimSuffix(fl.partial, "\r"))
		fl.partial = ""
	}
}

func (s *Sink) poll(fl *follower) {
	f, err := os.Open(fl.path)
	if err != nil {
		return
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return
	}
	if st.Size() < fl.offset {
		fl.offset = 0 // truncated or rotated
		fl.partial = ""
	}
	if st.Size() == fl.offset {
		return
	}
	if _, err := f.Seek(fl.offset, io.SeekStart); err != nil {
		return
	}
	data, err := io.ReadAll(io.LimitReader(f, 4<<20))
	if err != nil {
		return
	}
	fl.offset += int64(len(data))
	text := fl.partial + string(data)
	lines := strings.Split(text, "\n")
	fl.partial = lines[len(lines)-1]
	for _, line := range lines[:len(lines)-1] {
		s.publishLine(fl, strings.TrimSuffix(line, "\r"))
	}
}

func (s *Sink) publishLine(fl *follower, line string) {
	fl.push(line)
	if s.bus == nil {
		return
	}
	e := domain.Event{Type: domain.EventLogLine, Time: time.Now().UTC(), Project: fl.project, Service: fl.service, Line: line}
	if fl.job != "" {
		e.Type, e.JobID = domain.EventJobLog, fl.job
	}
	s.bus.Publish(e)
}

func (fl *follower) push(line string) {
	fl.mu.Lock()
	defer fl.mu.Unlock()
	fl.ring[fl.next] = line
	fl.next = (fl.next + 1) % len(fl.ring)
	if fl.next == 0 {
		fl.full = true
	}
}

func (fl *follower) last(n int) []string {
	fl.mu.Lock()
	defer fl.mu.Unlock()
	size := fl.next
	if fl.full {
		size = len(fl.ring)
	}
	if n > size {
		n = size
	}
	out := make([]string, 0, n)
	for i := n; i > 0; i-- {
		out = append(out, fl.ring[(fl.next-i+len(fl.ring))%len(fl.ring)])
	}
	return out
}

// Tail returns the last n lines: from the ring buffer when it holds enough,
// otherwise from the file.
func (s *Sink) Tail(project, service string, n int) ([]string, error) {
	s.mu.Lock()
	fl := s.followers[project+"/"+service]
	s.mu.Unlock()
	if fl != nil {
		if lines := fl.last(n); len(lines) == n {
			return lines, nil
		}
	}
	return tailFile(s.Path(project, service), n)
}

func tailFile(path string, n int) ([]string, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return nil, err
	}
	const window = 1 << 20
	start := st.Size() - window
	if start < 0 {
		start = 0
	}
	if _, err := f.Seek(start, io.SeekStart); err != nil {
		return nil, err
	}
	data, err := io.ReadAll(f)
	if err != nil {
		return nil, err
	}
	data = bytes.TrimRight(data, "\n")
	if len(data) == 0 {
		return []string{}, nil
	}
	lines := strings.Split(string(data), "\n")
	if start > 0 && len(lines) > 1 {
		lines = lines[1:] // first line may be cut
	}
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	return lines, nil
}

// Close stops all followers after a final poll.
func (s *Sink) Close() {
	close(s.stop)
	s.wg.Wait()
}

// FollowFile emits the last tail lines of path (tail < 0: the whole file),
// then every new line until finished() reports true and the file is
// drained. A trailing partial line is emitted at the end.
func FollowFile(ctx context.Context, path string, tail int, interval time.Duration, finished func() bool, emit func(string) error) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	var offset int64
	if tail >= 0 {
		var lines []string
		if offset, lines, err = completeTail(f, tail); err != nil {
			return err
		}
		for _, l := range lines {
			if err := emit(l); err != nil {
				return err
			}
		}
	}
	partial := ""
	buf := make([]byte, 64*1024)
	for {
		done := finished() // checked before reading so nothing written earlier is lost
		for {
			n, err := f.ReadAt(buf, offset)
			if n > 0 {
				offset += int64(n)
				text := partial + string(buf[:n])
				parts := strings.Split(text, "\n")
				partial = parts[len(parts)-1]
				for _, l := range parts[:len(parts)-1] {
					if err := emit(strings.TrimSuffix(l, "\r")); err != nil {
						return err
					}
				}
			}
			if err != nil || n < len(buf) {
				break
			}
		}
		if done {
			if partial != "" {
				return emit(partial)
			}
			return nil
		}
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(interval):
		}
	}
}

// completeTail returns the last n complete lines of f (within its last MiB)
// and the offset just after them; a trailing partial line is left unread.
func completeTail(f *os.File, n int) (int64, []string, error) {
	st, err := f.Stat()
	if err != nil {
		return 0, nil, err
	}
	const window = 1 << 20
	start := max(st.Size()-window, 0)
	data := make([]byte, st.Size()-start)
	if _, err := f.ReadAt(data, start); err != nil && err != io.EOF {
		return 0, nil, err
	}
	cut := bytes.LastIndexByte(data, '\n')
	if cut < 0 {
		return start, nil, nil
	}
	lines := strings.Split(string(data[:cut]), "\n")
	if start > 0 && len(lines) > 1 {
		lines = lines[1:] // first line may be cut by the window
	}
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	return start + int64(cut) + 1, lines, nil
}
