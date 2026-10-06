package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/spf13/cobra"

	"github.com/xean-io/rocket/internal/agentdocs"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/manifest"
	"github.com/xean-io/rocket/internal/scaffold"
)

func statusCmd(g *globals) *cobra.Command {
	return &cobra.Command{
		Use:   "status",
		Short: "One-shot summary for agents: project, services, running jobs, port conflicts",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			ref, err := g.projectRef()
			if err != nil && g.project != "" {
				return err
			}
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			sum, err := c.Summary(cmd.Context(), ref) // ref "" outside a project: everything
			if err != nil {
				return err
			}
			if g.json {
				printJSON(sum)
				return nil
			}
			printSummary(sum)
			return nil
		},
	}
}

func printSummary(sum app.Summary) {
	if sum.Project != nil {
		fmt.Printf("project %s (%s, default env %s)\n", sum.Project.Name, sum.Project.Root, sum.Project.DefaultEnv)
	}
	w := table()
	fmt.Fprintln(w, "PROJECT\tSERVICE\tSTATE\tHEALTH\tPORTS\tOWNER\tTTL")
	for _, r := range sum.Services {
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n", r.Project, r.Service, r.State, orDash(string(r.Health)),
			formatPorts(r.Ports), orDash(r.Owner), expiresStr(r.ExpiresAt))
	}
	w.Flush()
	if len(sum.Jobs) > 0 {
		fmt.Println("\nrunning jobs:")
		for _, j := range sum.Jobs {
			fmt.Printf("  %s  %s %s (%s, step %d/%d, owner %s, %s)\n", j.ID, j.Kind, j.Name, j.Project, j.Step, len(j.Steps), j.Owner, durationStr(j.DurationMS))
		}
	}
	if len(sum.Conflicts) > 0 {
		fmt.Println("\nconflicts:")
		for _, c := range sum.Conflicts {
			fmt.Printf("  %s %s/%s %s:%d: %s\n", c.Kind, c.Project, c.Service, c.PortName, c.Port, c.Detail)
		}
	}
}

// InitResult is the JSON shape of `rocket init`.
type InitResult struct {
	Path     string           `json:"path"`
	Written  bool             `json:"written"`
	Content  string           `json:"content,omitempty"` // with --print
	Detected *scaffold.Result `json:"detected"`
}

func initCmd(g *globals) *cobra.Command {
	var force, printOnly bool
	cmd := &cobra.Command{
		Use:   "init",
		Short: "Detect Taskfile/compose/.env and scaffold rocket.yaml",
		Long: "Scans the current directory (or -p <dir>) for Taskfile.yml, compose files and .env and writes a\n" +
			"starting rocket.yaml. Refuses to overwrite an existing one unless --force; --print writes nothing.",
		Args: cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			dir := g.project
			if dir == "" {
				dir = "."
			}
			dir, err := filepath.Abs(dir)
			if err != nil {
				return err
			}
			if st, err := os.Stat(dir); err != nil || !st.IsDir() {
				return fmt.Errorf("%s is not a directory", dir)
			}
			det, err := scaffold.Detect(dir)
			if err != nil {
				return err
			}
			content := det.Render()
			if _, err := manifest.Parse(content, dir); err != nil {
				return fmt.Errorf("generated rocket.yaml does not validate (please report): %w", err)
			}
			path := filepath.Join(dir, manifest.FileName)
			res := InitResult{Path: path, Detected: det}
			if printOnly {
				if g.json {
					res.Content = string(content)
					printJSON(res)
				} else {
					_, err = os.Stdout.Write(content)
				}
				return err
			}
			for _, existing := range []string{"rocket.yaml", "rocket.yml"} {
				p := filepath.Join(dir, existing)
				if _, err := os.Stat(p); err == nil && !force {
					return fmt.Errorf("%s already exists (use --force to overwrite or --print to preview)", p)
				}
			}
			if err := os.WriteFile(path, content, 0o644); err != nil {
				return err
			}
			res.Written = true
			if g.json {
				printJSON(res)
				return nil
			}
			fmt.Printf("wrote %s: %d services, %d pipelines, %d envs; review the guesses, then `rocket up`\n",
				path, len(det.Services), len(det.Pipelines), len(det.Envs))
			return nil
		},
	}
	cmd.Flags().BoolVar(&force, "force", false, "overwrite an existing rocket.yaml")
	cmd.Flags().BoolVar(&printOnly, "print", false, "print the generated rocket.yaml instead of writing it")
	return cmd
}

// AgentInstallResult is the JSON shape of `rocket agent install`.
type AgentInstallResult struct {
	Actions []agentdocs.Action `json:"actions"`
	Skill   string             `json:"skill,omitempty"` // with --print
	Block   string             `json:"block,omitempty"` // with --print
}

func agentCmd(g *globals) *cobra.Command {
	cmd := &cobra.Command{Use: "agent", Short: "Teach AI agents to use rocket"}
	var target string
	var global, printOnly bool
	install := &cobra.Command{
		Use:   "install",
		Short: "Write the rocket skill and an AGENTS.md/CLAUDE.md block (idempotent)",
		Long: "--target claude: Claude Code skill + CLAUDE.md block; agents: AGENTS.md block; both (default): all.\n" +
			"The skill goes to .claude/skills/rocket/SKILL.md in the project, or ~/.claude/skills with --global.\n" +
			"Blocks go to the project root (rocket.yaml found from cwd, else cwd; skipped with --global outside a project).",
		Args: cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			home, _ := os.UserHomeDir()
			root := ""
			cwd, err := os.Getwd()
			if err != nil {
				return err
			}
			if dir, err := manifest.Find(cwd); err == nil {
				root = dir
			} else if !global {
				root = cwd
			}
			opts := agentdocs.Options{Target: target, Global: global, Home: home, ProjectRoot: root}
			if printOnly {
				plan, err := agentdocs.Plan(opts)
				if err != nil {
					return err
				}
				if g.json {
					printJSON(AgentInstallResult{Actions: plan, Skill: agentdocs.Skill(), Block: agentdocs.Block()})
					return nil
				}
				for _, a := range plan {
					fmt.Printf("# would write %s (%s)\n", a.Path, a.Kind)
				}
				fmt.Printf("\n===== SKILL.md =====\n%s\n===== AGENTS.md / CLAUDE.md block =====\n%s", agentdocs.Skill(), agentdocs.Block())
				return nil
			}
			actions, err := agentdocs.Install(opts)
			if err != nil {
				return err
			}
			if g.json {
				printJSON(AgentInstallResult{Actions: actions})
				return nil
			}
			for _, a := range actions {
				fmt.Printf("%-9s %s\n", a.Action, a.Path)
			}
			return nil
		},
	}
	install.Flags().StringVar(&target, "target", agentdocs.TargetBoth, "claude | agents | both")
	install.Flags().BoolVar(&global, "global", false, "install the skill under ~/.claude/skills instead of the project")
	install.Flags().BoolVar(&printOnly, "print", false, "show the content without writing")
	cmd.AddCommand(install)
	return cmd
}

func appCmd(g *globals) *cobra.Command {
	return &cobra.Command{
		Use:   "app",
		Short: "Open Rocket.app (macOS); starts the daemon first",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			if runtime.GOOS != "darwin" {
				return errors.New("UI not available on this OS yet; use the CLI")
			}
			if _, err := daemonClient(cmd.Context()); err != nil {
				return err
			}
			app := "Rocket"
			if env := os.Getenv("ROCKET_APP"); env != "" {
				app = env
			}
			if out, err := exec.CommandContext(cmd.Context(), "open", "-a", app).CombinedOutput(); err != nil {
				return fmt.Errorf("open -a %s: %v: %s (build it from macos/ or set ROCKET_APP to its path)", app, err, strings.TrimSpace(string(out)))
			}
			if g.json {
				printJSON(map[string]any{"opened": true, "app": app})
			}
			return nil
		},
	}
}
