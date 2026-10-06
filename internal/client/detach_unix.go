//go:build unix

package client

import (
	"os/exec"
	"syscall"
)

// detach starts the daemon in a new session so it survives the CLI's
// terminal and is not part of the caller's process group.
func detach(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
}
