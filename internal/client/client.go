// Package client talks to rocketd over its unix socket and auto-starts it.
package client

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/xean-io/rocket/internal/adapters/api"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/paths"
)

// Client is a typed HTTP client for the daemon API.
type Client struct {
	http *http.Client
}

// APIError is a non-2xx daemon response.
type APIError struct {
	Status int
	Body   api.ErrorBody
}

func (e *APIError) Error() string { return e.Body.Error }

// New returns a client bound to the unix socket at path.
func New(socket string) *Client {
	tr := &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			var d net.Dialer
			return d.DialContext(ctx, "unix", socket)
		},
	}
	return &Client{http: &http.Client{Transport: tr}}
}

const base = "http://rocketd"

func (c *Client) do(ctx context.Context, method, path string, body, out any) error {
	var rdr io.Reader
	if body != nil {
		data, err := json.Marshal(body)
		if err != nil {
			return err
		}
		rdr = bytes.NewReader(data)
	}
	req, err := http.NewRequestWithContext(ctx, method, base+path, rdr)
	if err != nil {
		return err
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 300 {
		apiErr := &APIError{Status: resp.StatusCode}
		data, _ := io.ReadAll(resp.Body)
		if json.Unmarshal(data, &apiErr.Body) != nil || apiErr.Body.Error == "" {
			apiErr.Body.Error = fmt.Sprintf("daemon returned %d: %s", resp.StatusCode, strings.TrimSpace(string(data)))
		}
		return apiErr
	}
	if out == nil {
		return nil
	}
	return json.NewDecoder(resp.Body).Decode(out)
}

func (c *Client) Health(ctx context.Context) (api.HealthInfo, error) {
	var out api.HealthInfo
	err := c.do(ctx, http.MethodGet, "/v1/health", nil, &out)
	return out, err
}

func (c *Client) Up(ctx context.Context, req app.UpRequest) (app.UpResult, error) {
	var out app.UpResult
	err := c.do(ctx, http.MethodPost, "/v1/up", req, &out)
	return out, err
}

func (c *Client) Restart(ctx context.Context, req app.UpRequest) (app.UpResult, error) {
	var out app.UpResult
	err := c.do(ctx, http.MethodPost, "/v1/restart", req, &out)
	return out, err
}

func (c *Client) Down(ctx context.Context, req app.DownRequest) (app.DownResult, error) {
	var out app.DownResult
	err := c.do(ctx, http.MethodPost, "/v1/down", req, &out)
	return out, err
}

func (c *Client) Status(ctx context.Context, req app.StatusRequest) (app.StatusResult, error) {
	q := url.Values{}
	if req.Project != "" {
		q.Set("project", req.Project)
	}
	if req.AllProjects {
		q.Set("all", "true")
	}
	var out app.StatusResult
	err := c.do(ctx, http.MethodGet, "/v1/ps?"+q.Encode(), nil, &out)
	return out, err
}

func logsQuery(req app.LogsRequest) url.Values {
	q := url.Values{}
	q.Set("project", req.Project)
	q.Set("service", req.Service)
	q.Set("tail", strconv.Itoa(req.Tail))
	return q
}

func (c *Client) Logs(ctx context.Context, req app.LogsRequest) (app.LogsResult, error) {
	var out app.LogsResult
	err := c.do(ctx, http.MethodGet, "/v1/logs?"+logsQuery(req).Encode(), nil, &out)
	return out, err
}

// FollowLogs streams the tail and then new lines until ctx ends.
func (c *Client) FollowLogs(ctx context.Context, req app.LogsRequest, fn func(domain.Event) error) error {
	q := logsQuery(req)
	q.Set("follow", "true")
	return c.stream(ctx, "/v1/logs?"+q.Encode(), fn)
}

// Events streams daemon events matching the optional filters.
func (c *Client) Events(ctx context.Context, project, service string, types []string, fn func(domain.Event) error) error {
	q := url.Values{}
	if project != "" {
		q.Set("project", project)
	}
	if service != "" {
		q.Set("service", service)
	}
	if len(types) > 0 {
		q.Set("types", strings.Join(types, ","))
	}
	return c.stream(ctx, "/v1/events?"+q.Encode(), fn)
}

func (c *Client) stream(ctx context.Context, path string, fn func(domain.Event) error) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+path, nil)
	if err != nil {
		return err
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 300 {
		apiErr := &APIError{Status: resp.StatusCode}
		_ = json.NewDecoder(resp.Body).Decode(&apiErr.Body)
		return apiErr
	}
	sc := bufio.NewScanner(resp.Body)
	sc.Buffer(make([]byte, 64*1024), 4<<20)
	for sc.Scan() {
		line := sc.Text()
		data, ok := strings.CutPrefix(line, "data: ")
		if !ok {
			continue
		}
		var e domain.Event
		if err := json.Unmarshal([]byte(data), &e); err != nil {
			continue
		}
		if err := fn(e); err != nil {
			return err
		}
	}
	if ctx.Err() != nil {
		return nil
	}
	return sc.Err()
}

func (c *Client) Ports(ctx context.Context) (app.PortsResult, error) {
	var out app.PortsResult
	err := c.do(ctx, http.MethodGet, "/v1/ports", nil, &out)
	return out, err
}

func (c *Client) GC(ctx context.Context) (app.GCResult, error) {
	var out app.GCResult
	err := c.do(ctx, http.MethodPost, "/v1/gc", nil, &out)
	return out, err
}

func (c *Client) Projects(ctx context.Context) (api.ProjectsResult, error) {
	var out api.ProjectsResult
	err := c.do(ctx, http.MethodGet, "/v1/projects", nil, &out)
	return out, err
}

func (c *Client) AddProject(ctx context.Context, path string) (domain.ProjectRef, error) {
	var out domain.ProjectRef
	err := c.do(ctx, http.MethodPost, "/v1/projects", api.AddProjectRequest{Path: path}, &out)
	return out, err
}

func (c *Client) RemoveProject(ctx context.Context, name string) error {
	return c.do(ctx, http.MethodDelete, "/v1/projects/"+url.PathEscape(name), nil, nil)
}

func (c *Client) Shutdown(ctx context.Context) error {
	return c.do(ctx, http.MethodPost, "/v1/shutdown", nil, nil)
}

// Running reports whether a daemon answers on the socket.
func Running(ctx context.Context, p paths.Paths) (api.HealthInfo, bool) {
	ctx, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	info, err := New(p.Socket).Health(ctx)
	return info, err == nil
}

// Ensure returns a client to a running daemon, starting one detached
// (`rocket daemon run`) when none answers.
func Ensure(ctx context.Context, p paths.Paths) (*Client, error) {
	if _, ok := Running(ctx, p); ok {
		return New(p.Socket), nil
	}
	if err := StartDetached(p); err != nil {
		return nil, err
	}
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		if _, ok := Running(ctx, p); ok {
			return New(p.Socket), nil
		}
		time.Sleep(100 * time.Millisecond)
	}
	return nil, fmt.Errorf("rocketd did not start within 10s; see %s", p.DaemonLog)
}

// StartDetached launches `rocket daemon run` in its own session.
func StartDetached(p paths.Paths) error {
	if err := p.Ensure(); err != nil {
		return err
	}
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	logf, err := os.OpenFile(p.DaemonLog, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	defer logf.Close()
	cmd := exec.Command(exe, "daemon", "run")
	cmd.Env = append(os.Environ(), "ROCKET_HOME="+p.Home)
	cmd.Stdout = logf
	cmd.Stderr = logf
	detach(cmd)
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("start rocketd: %w", err)
	}
	return cmd.Process.Release()
}

// Summary returns the `rocket status` overview ("" = all projects).
func (c *Client) Summary(ctx context.Context, project string) (app.Summary, error) {
	q := url.Values{}
	if project != "" {
		q.Set("project", project)
	}
	var out app.Summary
	err := c.do(ctx, http.MethodGet, "/v1/status?"+q.Encode(), nil, &out)
	return out, err
}

// StartJob starts a pipeline, setup action or deploy; it returns at once.
func (c *Client) StartJob(ctx context.Context, req app.JobRequest) (domain.Job, error) {
	var out domain.Job
	err := c.do(ctx, http.MethodPost, "/v1/jobs", req, &out)
	return out, err
}

func (c *Client) Jobs(ctx context.Context, req app.JobsRequest) (app.JobsResult, error) {
	q := url.Values{}
	if req.Project != "" {
		q.Set("project", req.Project)
	}
	if req.AllProjects {
		q.Set("all", "true")
	}
	if req.Limit > 0 {
		q.Set("limit", strconv.Itoa(req.Limit))
	}
	var out app.JobsResult
	err := c.do(ctx, http.MethodGet, "/v1/jobs?"+q.Encode(), nil, &out)
	return out, err
}

// Job returns a job; with wait it blocks until the job finished.
func (c *Client) Job(ctx context.Context, id string, wait bool) (domain.Job, error) {
	path := "/v1/jobs/" + url.PathEscape(id)
	if wait {
		path += "?wait=true"
	}
	var out domain.Job
	err := c.do(ctx, http.MethodGet, path, nil, &out)
	return out, err
}

func (c *Client) JobLogs(ctx context.Context, id string, tail int) (app.JobLogsResult, error) {
	var out app.JobLogsResult
	err := c.do(ctx, http.MethodGet, "/v1/jobs/"+url.PathEscape(id)+"/logs?tail="+strconv.Itoa(tail), nil, &out)
	return out, err
}

// FollowJobLogs streams job.log events (tail < 0: from the start) and returns
// after the terminal job.state event.
func (c *Client) FollowJobLogs(ctx context.Context, id string, tail int, fn func(domain.Event) error) error {
	q := url.Values{}
	q.Set("follow", "true")
	if tail >= 0 {
		q.Set("tail", strconv.Itoa(tail))
	}
	return c.stream(ctx, "/v1/jobs/"+url.PathEscape(id)+"/logs?"+q.Encode(), fn)
}

func (c *Client) CancelJob(ctx context.Context, id string) (domain.Job, error) {
	var out domain.Job
	err := c.do(ctx, http.MethodPost, "/v1/jobs/"+url.PathEscape(id)+"/cancel", nil, &out)
	return out, err
}
