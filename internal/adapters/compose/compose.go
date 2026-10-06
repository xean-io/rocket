// Package compose wraps `docker compose` with a forced project name so
// rocket-managed stacks never collide with each other or with manual runs.
package compose

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os/exec"
	"strconv"
	"strings"

	"github.com/xean-io/rocket/internal/ports"
)

// Driver implements ports.ComposeDriver by shelling out to docker.
type Driver struct {
	Bin string // defaults to "docker"
}

var _ ports.ComposeDriver = Driver{}

func (d Driver) bin() string {
	if d.Bin == "" {
		return "docker"
	}
	return d.Bin
}

// Args builds `compose -p <name> -f … --profile … <op…>`.
func Args(t ports.ComposeTarget, op ...string) []string {
	args := []string{"compose", "-p", t.ProjectName}
	for _, f := range t.Files {
		args = append(args, "-f", f)
	}
	for _, p := range t.Profiles {
		args = append(args, "--profile", p)
	}
	return append(args, op...)
}

func (d Driver) cmd(ctx context.Context, t ports.ComposeTarget, out io.Writer, op ...string) *exec.Cmd {
	cmd := exec.CommandContext(ctx, d.bin(), Args(t, op...)...)
	cmd.Dir = t.Dir
	if len(t.Env) > 0 {
		cmd.Env = t.Env
	}
	cmd.Stdout = out
	cmd.Stderr = out
	return cmd
}

func run(cmd *exec.Cmd) error {
	var tail bytes.Buffer
	if cmd.Stderr == nil || cmd.Stderr == io.Discard {
		cmd.Stderr = &tail
	} else {
		cmd.Stderr = io.MultiWriter(cmd.Stderr, &tail)
	}
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(tail.String())
		if len(msg) > 400 {
			msg = "…" + msg[len(msg)-400:]
		}
		if msg != "" {
			return fmt.Errorf("%w: %s", err, msg)
		}
		return err
	}
	return nil
}

// Up starts one service detached and waits for its healthcheck.
func (d Driver) Up(ctx context.Context, t ports.ComposeTarget, service string, out io.Writer) (string, error) {
	if err := run(d.cmd(ctx, t, out, "up", "-d", "--wait", service)); err != nil {
		return "", err
	}
	var buf bytes.Buffer
	c := d.cmd(ctx, t, &buf, "ps", "-q", service)
	c.Stderr = io.Discard
	_ = c.Run()
	return strings.TrimSpace(buf.String()), nil
}

// Stop stops one service.
func (d Driver) Stop(ctx context.Context, t ports.ComposeTarget, service string, out io.Writer) error {
	return run(d.cmd(ctx, t, out, "stop", service))
}

// Down removes the whole compose project (containers + network).
func (d Driver) Down(ctx context.Context, t ports.ComposeTarget, out io.Writer) error {
	return run(d.cmd(ctx, t, out, "down", "--remove-orphans"))
}

// Running checks for a running container by compose labels.
func (d Driver) Running(ctx context.Context, composeProject, service string) (bool, error) {
	var buf bytes.Buffer
	cmd := exec.CommandContext(ctx, d.bin(), "ps", "-q",
		"--filter", "label=com.docker.compose.project="+composeProject,
		"--filter", "label=com.docker.compose.service="+service,
		"--filter", "status=running")
	cmd.Stdout = &buf
	if err := run(cmd); err != nil {
		return false, err
	}
	return strings.TrimSpace(buf.String()) != "", nil
}

// Logs returns the last lines of a service's container logs.
func (d Driver) Logs(ctx context.Context, t ports.ComposeTarget, service string, tail int) ([]string, error) {
	var buf bytes.Buffer
	if err := run(d.cmd(ctx, t, &buf, "logs", "--no-color", "--tail", strconv.Itoa(tail), service)); err != nil {
		return nil, err
	}
	s := strings.TrimRight(buf.String(), "\n")
	if s == "" {
		return []string{}, nil
	}
	return strings.Split(s, "\n"), nil
}
