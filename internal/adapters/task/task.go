// Package task builds go-task invocations. Long-running tasks are executed
// by the process adapter so they get the same group supervision.
package task

import (
	"os"
	"path/filepath"

	"github.com/xean-io/rocket/internal/ports"
)

// TaskfileNames are the files go-task looks for, in its own order.
var TaskfileNames = []string{"Taskfile.yml", "taskfile.yml", "Taskfile.yaml", "taskfile.yaml",
	"Taskfile.dist.yml", "taskfile.dist.yml", "Taskfile.dist.yaml", "taskfile.dist.yaml"}

// Driver implements ports.TaskDriver.
type Driver struct {
	Bin string // defaults to "task"
}

var _ ports.TaskDriver = Driver{}

// Argv returns the command line for `task <name> [-- args]`.
func (d Driver) Argv(name string, args ...string) []string {
	bin := d.Bin
	if bin == "" {
		bin = "task"
	}
	argv := []string{bin, name}
	if len(args) > 0 {
		argv = append(append(argv, "--"), args...)
	}
	return argv
}

// Taskfile returns the Taskfile in dir, or "" when there is none.
func (Driver) Taskfile(dir string) string {
	for _, n := range TaskfileNames {
		p := filepath.Join(dir, n)
		if st, err := os.Stat(p); err == nil && !st.IsDir() {
			return p
		}
	}
	return ""
}
