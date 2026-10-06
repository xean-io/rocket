//go:build windows

// Package process starts and stops local process groups.
package process

import (
	"context"
	"time"

	"github.com/xean-io/rocket/internal/ports"
)

// Runner is a stub until Job Object support lands on Windows.
type Runner struct{}

var _ ports.ProcessRunner = Runner{}

func (Runner) Start(ports.ProcessSpec) (ports.ProcessHandle, error) {
	return ports.ProcessHandle{}, ports.ErrUnsupported
}

func (Runner) Stop(context.Context, int, time.Duration) error { return ports.ErrUnsupported }

func (Runner) Alive(int, int) bool { return false }
