// Package api exposes the application over HTTP+JSON (served on the daemon's
// unix socket) including an SSE event stream. See README.md for the contract.
package api

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/xean-io/rocket/internal/adapters/logs"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// Version of the API contract, reported by /v1/health.
const Version = "v1"

// HealthInfo is returned by GET /v1/health.
type HealthInfo struct {
	OK        bool      `json:"ok"`
	API       string    `json:"api"`
	Version   string    `json:"version"`
	PID       int       `json:"pid"`
	StartedAt time.Time `json:"started_at"`
	Home      string    `json:"home"`
	Socket    string    `json:"socket"`
	HTTP      string    `json:"http,omitempty"` // token-protected TCP listener
}

// ErrorBody is the JSON body of every non-2xx response.
type ErrorBody struct {
	Error string `json:"error"`
	Code  string `json:"code"` // invalid | not_found | conflict | confirmation_required | unauthorized | internal
}

// AddProjectRequest is the body of POST /v1/projects.
type AddProjectRequest struct {
	Path string `json:"path"`
}

// ProjectsResult is returned by GET /v1/projects.
type ProjectsResult struct {
	Projects []domain.ProjectRef `json:"projects"`
}

// Server adapts *app.App to HTTP.
type Server struct {
	App      *app.App
	Bus      ports.EventBus
	Info     HealthInfo
	Shutdown func() // invoked asynchronously by POST /v1/shutdown
}

// Handler returns the routed handler.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /v1/health", s.health)
	mux.HandleFunc("POST /v1/up", s.up)
	mux.HandleFunc("POST /v1/restart", s.restart)
	mux.HandleFunc("POST /v1/down", s.down)
	mux.HandleFunc("GET /v1/ps", s.ps)
	mux.HandleFunc("GET /v1/logs", s.logs)
	mux.HandleFunc("GET /v1/ports", s.ports)
	mux.HandleFunc("POST /v1/gc", s.gc)
	mux.HandleFunc("GET /v1/projects", s.listProjects)
	mux.HandleFunc("POST /v1/projects", s.addProject)
	mux.HandleFunc("DELETE /v1/projects/{name}", s.removeProject)
	mux.HandleFunc("GET /v1/status", s.status)
	mux.HandleFunc("POST /v1/jobs", s.startJob)
	mux.HandleFunc("GET /v1/jobs", s.listJobs)
	mux.HandleFunc("GET /v1/jobs/{id}", s.getJob)
	mux.HandleFunc("GET /v1/jobs/{id}/logs", s.jobLogs)
	mux.HandleFunc("POST /v1/jobs/{id}/cancel", s.cancelJob)
	mux.HandleFunc("GET /v1/events", s.events)
	mux.HandleFunc("POST /v1/shutdown", s.shutdown)
	return mux
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
}

func writeErr(w http.ResponseWriter, err error) {
	status, code := http.StatusInternalServerError, "internal"
	switch {
	case errors.Is(err, app.ErrInvalid):
		status, code = http.StatusBadRequest, "invalid"
	case errors.Is(err, app.ErrNotFound):
		status, code = http.StatusNotFound, "not_found"
	case errors.Is(err, app.ErrConflict):
		status, code = http.StatusConflict, "conflict"
	case errors.Is(err, app.ErrConfirmation):
		status, code = http.StatusPreconditionRequired, "confirmation_required"
	}
	writeJSON(w, status, ErrorBody{Error: err.Error(), Code: code})
}

func decode(w http.ResponseWriter, r *http.Request, v any) bool {
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		writeErr(w, fmt.Errorf("%w: bad JSON body: %v", app.ErrInvalid, err))
		return false
	}
	return true
}

// opCtx detaches long operations from the client connection so a dropped
// CLI never leaves a half-applied up/down.
func opCtx(r *http.Request) context.Context { return context.WithoutCancel(r.Context()) }

func (s *Server) health(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, s.Info)
}

func (s *Server) up(w http.ResponseWriter, r *http.Request) {
	var req app.UpRequest
	if !decode(w, r, &req) {
		return
	}
	res, err := s.App.Up(opCtx(r), req)
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) restart(w http.ResponseWriter, r *http.Request) {
	var req app.UpRequest
	if !decode(w, r, &req) {
		return
	}
	res, err := s.App.Restart(opCtx(r), req)
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) down(w http.ResponseWriter, r *http.Request) {
	var req app.DownRequest
	if !decode(w, r, &req) {
		return
	}
	res, err := s.App.Down(opCtx(r), req)
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func boolParam(r *http.Request, name string) bool {
	v, _ := strconv.ParseBool(r.URL.Query().Get(name))
	return v
}

func (s *Server) ps(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	res, err := s.App.Status(r.Context(), app.StatusRequest{Project: q.Get("project"), AllProjects: boolParam(r, "all")})
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) logs(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	tail, _ := strconv.Atoi(q.Get("tail"))
	req := app.LogsRequest{Project: q.Get("project"), Service: q.Get("service"), Tail: tail}
	if !boolParam(r, "follow") {
		res, err := s.App.Logs(r.Context(), req)
		if err != nil {
			writeErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, res)
		return
	}
	// Subscribe before reading the tail so no line falls in between.
	ch, cancel := s.Bus.Subscribe(1024)
	defer cancel()
	res, err := s.App.Logs(r.Context(), req)
	if err != nil {
		writeErr(w, err)
		return
	}
	sse := startSSE(w)
	if sse == nil {
		return
	}
	for _, line := range res.Lines {
		sse.send(domain.Event{Type: domain.EventLogLine, Time: time.Now().UTC(), Project: res.Project, Service: res.Service, Line: line})
	}
	s.pump(r.Context(), sse, ch, filter{project: res.Project, service: res.Service, types: map[string]bool{domain.EventLogLine: true}})
}

func (s *Server) ports(w http.ResponseWriter, _ *http.Request) {
	res, err := s.App.Ports()
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) gc(w http.ResponseWriter, r *http.Request) {
	res, err := s.App.GC(opCtx(r))
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) listProjects(w http.ResponseWriter, _ *http.Request) {
	list, err := s.App.ListProjects()
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, ProjectsResult{Projects: list})
}

func (s *Server) addProject(w http.ResponseWriter, r *http.Request) {
	var req AddProjectRequest
	if !decode(w, r, &req) {
		return
	}
	ref, err := s.App.AddProject(req.Path)
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, ref)
}

func (s *Server) removeProject(w http.ResponseWriter, r *http.Request) {
	name := r.PathValue("name")
	if err := s.App.RemoveProject(name); err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"removed": name})
}

func (s *Server) shutdown(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, map[string]any{"ok": true, "pid": os.Getpid()})
	if s.Shutdown != nil {
		go s.Shutdown()
	}
}

type filter struct {
	project, service, job string
	types                 map[string]bool
}

func (f filter) match(e domain.Event) bool {
	if f.project != "" && e.Project != f.project {
		return false
	}
	if f.job != "" && e.JobID != f.job {
		return false
	}
	if f.service != "" && e.Service != f.service {
		return false
	}
	return len(f.types) == 0 || f.types[e.Type]
}

func (s *Server) events(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	f := filter{project: q.Get("project"), service: q.Get("service"), job: q.Get("job"), types: map[string]bool{}}
	for _, t := range strings.Split(q.Get("types"), ",") {
		if t = strings.TrimSpace(t); t != "" {
			f.types[t] = true
		}
	}
	ch, cancel := s.Bus.Subscribe(1024)
	defer cancel()
	sse := startSSE(w)
	if sse == nil {
		return
	}
	s.pump(r.Context(), sse, ch, f)
}

func (s *Server) pump(ctx context.Context, sse *sseWriter, ch <-chan domain.Event, f filter) {
	heartbeat := time.NewTicker(15 * time.Second)
	defer heartbeat.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-heartbeat.C:
			if !sse.comment("ping") {
				return
			}
		case e, ok := <-ch:
			if !ok {
				return
			}
			if f.match(e) && !sse.send(e) {
				return
			}
		}
	}
}

type sseWriter struct {
	w http.ResponseWriter
	f http.Flusher
}

func startSSE(w http.ResponseWriter) *sseWriter {
	f, ok := w.(http.Flusher)
	if !ok {
		writeErr(w, errors.New("streaming unsupported"))
		return nil
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	w.WriteHeader(http.StatusOK)
	// Opening comment: some clients (URLSession) hold the response until body bytes arrive.
	_, _ = io.WriteString(w, ": ok\n\n")
	f.Flush()
	return &sseWriter{w: w, f: f}
}

func (s *sseWriter) send(e domain.Event) bool {
	data, err := json.Marshal(e)
	if err != nil {
		return true
	}
	if _, err := fmt.Fprintf(s.w, "event: %s\ndata: %s\n\n", e.Type, data); err != nil {
		return false
	}
	s.f.Flush()
	return true
}

func (s *sseWriter) comment(text string) bool {
	if _, err := fmt.Fprintf(s.w, ": %s\n\n", text); err != nil {
		return false
	}
	s.f.Flush()
	return true
}

func (s *Server) status(w http.ResponseWriter, r *http.Request) {
	res, err := s.App.Summary(r.Context(), r.URL.Query().Get("project"))
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) startJob(w http.ResponseWriter, r *http.Request) {
	var req app.JobRequest
	if !decode(w, r, &req) {
		return
	}
	job, err := s.App.StartJob(opCtx(r), req)
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, job)
}

func (s *Server) listJobs(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	limit, _ := strconv.Atoi(q.Get("limit"))
	res, err := s.App.ListJobs(app.JobsRequest{Project: q.Get("project"), AllProjects: boolParam(r, "all"), Limit: limit})
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

func (s *Server) getJob(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	var (
		job domain.Job
		err error
	)
	if boolParam(r, "wait") {
		job, err = s.App.WaitJob(r.Context(), id)
	} else {
		job, err = s.App.GetJob(id)
	}
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, job)
}

func (s *Server) cancelJob(w http.ResponseWriter, r *http.Request) {
	job, err := s.App.CancelJob(opCtx(r), r.PathValue("id"))
	if err != nil {
		writeErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, job)
}

// jobLogs returns the tail, or with follow=true streams job.log events read
// straight from the log file (no gaps, no duplicates) and ends with the
// terminal job.state event.
func (s *Server) jobLogs(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	q := r.URL.Query()
	tail, err := strconv.Atoi(q.Get("tail"))
	if !boolParam(r, "follow") {
		res, err := s.App.JobLogs(id, tail)
		if err != nil {
			writeErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, res)
		return
	}
	if err != nil {
		tail = -1 // follow without tail: the whole log
	}
	job, jerr := s.App.GetJob(id)
	if jerr != nil {
		writeErr(w, jerr)
		return
	}
	sse := startSSE(w)
	if sse == nil {
		return
	}
	finished := func() bool {
		j, err := s.App.GetJob(id)
		return err != nil || j.Status.Terminal()
	}
	emit := func(line string) error {
		if !sse.send(domain.Event{Type: domain.EventJobLog, Time: time.Now().UTC(), Project: job.Project, JobID: id, Line: line}) {
			return errors.New("client gone")
		}
		return nil
	}
	ctx, cancel := context.WithCancel(r.Context())
	defer cancel()
	lines := make(chan string)
	done := make(chan error, 1)
	go func() {
		done <- logs.FollowFile(ctx, job.LogPath, tail, 100*time.Millisecond, finished, func(line string) error {
			select {
			case lines <- line:
				return nil
			case <-ctx.Done():
				return ctx.Err()
			}
		})
	}()
	heartbeat := time.NewTicker(15 * time.Second)
	defer heartbeat.Stop()
	// This handler alone writes frames: logs, heartbeat comments and the final
	// state cannot interleave, and the file follower drains before done arrives.
	for {
		select {
		case <-ctx.Done():
			return
		case line := <-lines:
			if err := emit(line); err != nil {
				return
			}
		case <-heartbeat.C:
			if !sse.comment("ping") {
				return
			}
		case err := <-done:
			heartbeat.Stop()
			if err != nil || ctx.Err() != nil {
				return
			}
			if final, err := s.App.GetJob(id); err == nil && final.Status.Terminal() {
				sse.send(domain.Event{Type: domain.EventJobState, Time: time.Now().UTC(), Project: final.Project, JobID: id, Status: final.Status, Job: &final})
			}
			return
		}
	}
}
