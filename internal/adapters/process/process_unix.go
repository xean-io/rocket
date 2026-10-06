//go:build unix

// Package process starts and stops local process groups.
package process

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"syscall"
	"time"

	"github.com/xean-io/rocket/internal/ports"
)

// Runner implements ports.ProcessRunner with POSIX process groups.
type Runner struct{}

var _ ports.ProcessRunner = Runner{}

// Start runs spec in a new process group (pgid == pid).
func (Runner) Start(spec ports.ProcessSpec) (ports.ProcessHandle, error) {
	if len(spec.Argv) == 0 {
		return ports.ProcessHandle{}, errors.New("empty argv")
	}
	cmd := exec.Command(spec.Argv[0], spec.Argv[1:]...)
	cmd.Dir = spec.Dir
	cmd.Env = spec.Env
	if spec.Output != nil {
		cmd.Stdout = spec.Output
		cmd.Stderr = spec.Output
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		return ports.ProcessHandle{}, err
	}
	done := make(chan int, 1)
	go func() {
		_ = cmd.Wait()
		done <- exitCode(cmd)
	}()
	pid := cmd.Process.Pid
	return ports.ProcessHandle{PID: pid, PGID: pid, Done: done}, nil
}

func exitCode(cmd *exec.Cmd) int {
	if cmd.ProcessState == nil {
		return -1
	}
	if ws, ok := cmd.ProcessState.Sys().(syscall.WaitStatus); ok && ws.Signaled() {
		return 128 + int(ws.Signal())
	}
	return cmd.ProcessState.ExitCode()
}

// Stop sends SIGTERM to the whole group, waits up to grace, then SIGKILL.
func (Runner) Stop(ctx context.Context, pgid int, grace time.Duration) error {
	if pgid <= 1 || pgid == syscall.Getpgrp() {
		return fmt.Errorf("refusing to signal process group %d", pgid)
	}
	if err := syscall.Kill(-pgid, syscall.SIGTERM); errors.Is(err, syscall.ESRCH) {
		return nil
	} else if err != nil {
		return fmt.Errorf("SIGTERM group %d: %w", pgid, err)
	}
	if waitGone(ctx, pgid, grace) {
		return nil
	}
	if err := syscall.Kill(-pgid, syscall.SIGKILL); err != nil && !errors.Is(err, syscall.ESRCH) {
		return fmt.Errorf("SIGKILL group %d: %w", pgid, err)
	}
	if !waitGone(ctx, pgid, 3*time.Second) {
		return fmt.Errorf("process group %d still alive after SIGKILL", pgid)
	}
	return nil
}

func waitGone(ctx context.Context, pgid int, d time.Duration) bool {
	deadline := time.Now().Add(d)
	for {
		if err := syscall.Kill(-pgid, 0); errors.Is(err, syscall.ESRCH) {
			return true
		}
		if time.Now().After(deadline) {
			return false
		}
		select {
		case <-ctx.Done():
			return false
		case <-time.After(25 * time.Millisecond):
		}
	}
}

// Alive reports whether pid exists and still belongs to pgid.
func (Runner) Alive(pid, pgid int) bool {
	if pid <= 0 {
		return false
	}
	if err := syscall.Kill(pid, 0); err != nil && !errors.Is(err, syscall.EPERM) {
		return false
	}
	if pgid > 0 {
		got, err := syscall.Getpgid(pid)
		if err != nil || got != pgid {
			return false
		}
	}
	return true
}
