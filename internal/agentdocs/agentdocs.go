// Package agentdocs renders the instructions that teach AI agents to use
// rocket (a Claude Code skill and an AGENTS.md/CLAUDE.md block) and installs
// them idempotently.
package agentdocs

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Markers delimit the managed block in AGENTS.md / CLAUDE.md.
const (
	BeginMarker = "<!-- rocket:begin -->"
	EndMarker   = "<!-- rocket:end -->"
)

// Install targets.
const (
	TargetClaude = "claude" // skill + CLAUDE.md block
	TargetAgents = "agents" // AGENTS.md block
	TargetBoth   = "both"
)

const rules = `- Use rocket for ALL process, service, test and CI operations. Never start processes yourself:
  no ` + "`&`" + `, ` + "`nohup`" + `, ` + "`docker compose up`" + `, ` + "`task dev`" + `, watchers or dev servers outside rocket.
- Identify yourself on every command: ` + "`export ROCKET_OWNER=agent:<session-id>`" + ` (or ` + "`--owner agent:<session-id>`" + `).
- Always pass ` + "`--ttl`" + ` for exploratory runs: ` + "`rocket up api --ttl 30m`" + `.
- Before finishing, clean up everything you started: ` + "`rocket down --owner agent:<session-id>`" + `.
- Run targeted checks, not full suites: ` + "`rocket run <targeted pipeline|task> [-- args]`" + `
  (e.g. ` + "`rocket run test:api -- -run TestLogin`" + `).
- Always add ` + "`--json`" + ` and check exit codes: 0 ok, 1 error, 2 partial or job failure, 3 confirmation required.
- Look before acting: ` + "`rocket status --json`" + ` shows services, ports, running jobs and port conflicts.
- Deploys need a human: ` + "`rocket deploy <env>`" + ` is refused for agents unless a human passes ` + "`--yes`" + `
  and sets ` + "`ROCKET_ALLOW_DEPLOY=1`" + `.
`

const cheatsheet = "```sh\n" +
	"export ROCKET_OWNER=agent:<session-id>\n" +
	"rocket status --json                      # project, services, ports, running jobs, conflicts\n" +
	"rocket up <svc|group> --ttl 30m --json    # idempotent; dependencies first; busy ports remapped\n" +
	"rocket ps --json | rocket ports --json\n" +
	"rocket logs <svc> --tail 100 --json\n" +
	"rocket run <pipeline|task> --json [-- args]   # blocks; {job,status,exit_code,duration_ms,log_path,tail}\n" +
	"rocket run <pipeline> --detach --json     # returns the job; then rocket job <id> logs -f\n" +
	"rocket jobs --json | rocket job <id> --json | rocket job cancel <id>\n" +
	"rocket setup --json                       # doctor -> install -> migrate\n" +
	"rocket down --owner agent:<session-id> --json\n" +
	"```\n"

// Skill returns the Claude Code skill (SKILL.md).
func Skill() string {
	return `---
name: rocket
description: "Trigger: starting or stopping dev servers, docker compose, Taskfile tasks, tests, CI, migrations, deploys, busy ports, logs. Use the rocket CLI for every process/service/test/CI operation."
---

# rocket: the single owner of dev processes

This machine runs rocket, a supervisor that owns every dev process across projects
(services, compose containers, test and CI jobs). Processes you start yourself become
orphans that hold ports and break the next run; rocket tracks, cleans and reports them.

## Rules

` + rules + `
## Commands

` + cheatsheet
}

// Block returns the managed AGENTS.md / CLAUDE.md block, ending in a newline.
func Block() string {
	return BeginMarker + "\n## Processes, services, tests and CI: use rocket\n\n" + rules + "\n" + cheatsheet + EndMarker + "\n"
}

// UpsertBlock inserts or refreshes the managed block in existing content.
func UpsertBlock(existing string) (string, bool) {
	block := Block()
	begin := strings.Index(existing, BeginMarker)
	end := strings.Index(existing, EndMarker)
	var out string
	switch {
	case begin >= 0 && end > begin:
		rest := existing[end+len(EndMarker):]
		rest = strings.TrimPrefix(rest, "\n")
		out = existing[:begin] + block + rest
	case existing == "":
		out = block
	default:
		if !strings.HasSuffix(existing, "\n") {
			existing += "\n"
		}
		out = existing + "\n" + block
	}
	return out, out != existing
}

// Options selects what Install writes.
type Options struct {
	Target      string // claude | agents | both
	Global      bool   // skill under Home/.claude instead of the project
	Home        string // user home (tests pass a temp dir)
	ProjectRoot string // where AGENTS.md/CLAUDE.md live; "" skips the blocks
}

// Action reports one file Install touched.
type Action struct {
	Path   string `json:"path"`
	Kind   string `json:"kind"`   // skill | block
	Action string `json:"action"` // created | updated | unchanged
}

type planned struct{ path, kind string }

// Plan lists the files Install would write.
func Plan(o Options) ([]Action, error) {
	files, err := plan(o)
	if err != nil {
		return nil, err
	}
	out := make([]Action, len(files))
	for i, f := range files {
		out[i] = Action{Path: f.path, Kind: f.kind, Action: "planned"}
	}
	return out, nil
}

func plan(o Options) ([]planned, error) {
	if o.Target == "" {
		o.Target = TargetBoth
	}
	claude := o.Target == TargetClaude || o.Target == TargetBoth
	agents := o.Target == TargetAgents || o.Target == TargetBoth
	if !claude && !agents {
		return nil, fmt.Errorf("unknown target %q (claude, agents or both)", o.Target)
	}
	var out []planned
	if claude {
		base := o.ProjectRoot
		if o.Global {
			base = o.Home
		}
		if base == "" {
			return nil, errors.New("no directory for the skill (not in a project and no home directory)")
		}
		out = append(out, planned{filepath.Join(base, ".claude", "skills", "rocket", "SKILL.md"), "skill"})
	}
	if o.ProjectRoot != "" {
		if agents {
			out = append(out, planned{filepath.Join(o.ProjectRoot, "AGENTS.md"), "block"})
		}
		if claude {
			out = append(out, planned{filepath.Join(o.ProjectRoot, "CLAUDE.md"), "block"})
		}
	}
	return out, nil
}

// Install writes the skill and/or blocks; running it twice changes nothing.
func Install(o Options) ([]Action, error) {
	files, err := plan(o)
	if err != nil {
		return nil, err
	}
	var out []Action
	for _, f := range files {
		old, err := os.ReadFile(f.path)
		exists := err == nil
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return out, err
		}
		next := Skill()
		if f.kind == "block" {
			next, _ = UpsertBlock(string(old))
		}
		a := Action{Path: f.path, Kind: f.kind, Action: "unchanged"}
		if !exists || string(old) != next {
			if err := os.MkdirAll(filepath.Dir(f.path), 0o755); err != nil {
				return out, err
			}
			if err := os.WriteFile(f.path, []byte(next), 0o644); err != nil {
				return out, err
			}
			a.Action = "updated"
			if !exists {
				a.Action = "created"
			}
		}
		out = append(out, a)
	}
	return out, nil
}
