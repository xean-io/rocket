// Command rocket is the CLI and (via `rocket daemon run`) the daemon.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime/debug"
	"strings"

	"github.com/spf13/cobra"

	"github.com/xean-io/rocket/internal/client"
	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
	"github.com/xean-io/rocket/internal/paths"
)

// Exit codes: 0 ok, 1 error, 2 partial failure / failed job, 3 confirmation required.
const exitConfirmation = 3

// exitError carries a specific exit code (2 = partial failure).
type exitError struct {
	code int
	msg  string
}

func (e *exitError) Error() string { return e.msg }

type globals struct {
	json    bool
	project string
}

func main() {
	version = resolveVersion(version, debug.ReadBuildInfo)
	g := &globals{}
	root := &cobra.Command{
		Use:           "rocket",
		Short:         "Single owner of every dev process across your projects",
		SilenceUsage:  true,
		SilenceErrors: true,
		Version:       version,
	}
	root.PersistentFlags().BoolVar(&g.json, "json", false, "emit machine-readable JSON")
	root.PersistentFlags().StringVarP(&g.project, "project", "p", "", "project name or path (default: rocket.yaml found from cwd)")

	root.AddCommand(
		upCmd(g), downCmd(g), restartCmd(g), psCmd(g), logsCmd(g), portsCmd(g), gcCmd(g),
		projectsCmd(g), daemonCmd(g), schemaCmd(),
		runCmd(g), setupCmd(g), setupStepCmd(g, "doctor"), setupStepCmd(g, "install"), setupStepCmd(g, "migrate"),
		deployCmd(g), jobsCmd(g), jobCmd(g),
		statusCmd(g), initCmd(g), agentCmd(g), appCmd(g),
	)

	err := root.Execute()
	if err == nil {
		return
	}
	code := 1
	var ee *exitError
	if errors.As(err, &ee) {
		code = ee.code
		if ee.msg == "" {
			os.Exit(code)
		}
	}
	errCode := "error"
	var apiErr *client.APIError
	if errors.As(err, &apiErr) && apiErr.Body.Code != "" {
		errCode = apiErr.Body.Code
		if errCode == "confirmation_required" {
			code = exitConfirmation
		}
	}
	if g.json {
		printJSON(map[string]string{"error": err.Error(), "code": errCode})
	} else {
		fmt.Fprintln(os.Stderr, "rocket:", err)
	}
	os.Exit(code)
}

func printJSON(v any) {
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
}

// daemonClient returns a client, auto-starting the daemon.
func daemonClient(ctx context.Context) (*client.Client, error) {
	p, err := paths.Resolve()
	if err != nil {
		return nil, err
	}
	return client.Ensure(ctx, p)
}

// projectRef resolves -p (name or path) or walks up from cwd to rocket.yaml.
func (g *globals) projectRef() (string, error) {
	if g.project != "" {
		if looksLikePath(g.project) {
			abs, err := filepath.Abs(g.project)
			if err != nil {
				return "", err
			}
			return manifest.Find(abs)
		}
		return g.project, nil
	}
	cwd, err := os.Getwd()
	if err != nil {
		return "", err
	}
	dir, err := manifest.Find(cwd)
	if err != nil {
		return "", fmt.Errorf("%w (pass -p <project>)", err)
	}
	return dir, nil
}

func looksLikePath(s string) bool {
	return s == "." || s == ".." || strings.ContainsRune(s, filepath.Separator) || strings.ContainsRune(s, '/')
}

// owner returns --owner, then $ROCKET_OWNER, then "user".
func owner(flag string) string {
	if flag != "" {
		return flag
	}
	if env := os.Getenv("ROCKET_OWNER"); env != "" {
		return env
	}
	return domain.DefaultOwner
}
