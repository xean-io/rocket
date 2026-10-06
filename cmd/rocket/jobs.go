package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/spf13/cobra"

	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
)

// tailLines is how many log lines a finished blocking job returns.
const tailLines = 50

type jobFlags struct {
	owner    string
	ttl      string
	env      string
	profiles []string
	detach   bool
}

func (f *jobFlags) register(cmd *cobra.Command) {
	cmd.Flags().StringVar(&f.owner, "owner", "", "owner tag (default: $ROCKET_OWNER or \"user\")")
	cmd.Flags().StringVar(&f.ttl, "ttl", "", "cancel automatically after this duration, e.g. 30m")
	cmd.Flags().BoolVar(&f.detach, "detach", false, "return the job id immediately instead of waiting")
}

func (f *jobFlags) registerSelection(cmd *cobra.Command) {
	cmd.Flags().StringVar(&f.env, "env", "", "environment (default: project default_env)")
	cmd.Flags().StringArrayVar(&f.profiles, "profile", nil, "enable a prerequisite startup profile (repeatable)")
}

// runJob starts a job and, unless detached, streams its log (human) or
// waits for it (--json). Ctrl-C cancels the job. Failed jobs exit 2.
func runJob(ctx context.Context, g *globals, req app.JobRequest, f jobFlags) error {
	ref, err := g.projectRef()
	if err != nil {
		return err
	}
	req.Project, req.Owner = ref, owner(f.owner)
	req.TTL = f.ttl
	req.Env, req.Profiles = f.env, f.profiles
	c, err := daemonClient(ctx)
	if err != nil {
		return err
	}
	job, err := c.StartJob(ctx, req)
	if err != nil {
		return err
	}
	if f.detach {
		if g.json {
			printJSON(job)
		} else {
			fmt.Println(job.ID)
			fmt.Fprintf(os.Stderr, "rocket: started job %s (%s %s); follow with `rocket job %s logs -f`\n", job.ID, job.Kind, job.Name, job.ID)
		}
		return nil
	}

	sigCtx, stop := signal.NotifyContext(ctx, os.Interrupt, syscall.SIGTERM)
	defer stop()
	if g.json {
		_, err = c.Job(sigCtx, job.ID, true)
	} else {
		err = c.FollowJobLogs(sigCtx, job.ID, -1, func(e domain.Event) error {
			if e.Type == domain.EventJobLog {
				fmt.Println(e.Line)
			}
			return nil
		})
	}
	bg := context.Background()
	if sigCtx.Err() != nil && ctx.Err() == nil {
		fmt.Fprintf(os.Stderr, "rocket: interrupted, canceling job %s\n", job.ID)
		if _, cerr := c.CancelJob(bg, job.ID); cerr != nil {
			return cerr
		}
	} else if err != nil {
		return err
	}
	final, err := c.Job(bg, job.ID, true)
	if err != nil {
		return err
	}
	logs, _ := c.JobLogs(bg, job.ID, tailLines)
	out := app.Outcome(final, logs.Lines)
	if g.json {
		printJSON(out)
	} else {
		fmt.Fprintf(os.Stderr, "rocket: job %s (%s %s) %s in %s, exit %s\n", out.Job, out.Kind, out.Name, out.Status,
			time.Duration(out.DurationMS)*time.Millisecond, exitStr(out.ExitCode))
		if out.Error != "" {
			fmt.Fprintf(os.Stderr, "rocket: %s\n", out.Error)
		}
		fmt.Fprintf(os.Stderr, "rocket: log %s\n", out.LogPath)
	}
	if final.Status != domain.JobSucceeded {
		return &exitError{code: 2}
	}
	return nil
}

func exitStr(code *int) string {
	if code == nil {
		return "-"
	}
	return fmt.Sprint(*code)
}

func runCmd(g *globals) *cobra.Command {
	var f jobFlags
	cmd := &cobra.Command{
		Use:   "run <pipeline|task> [-- args...]",
		Short: "Run a pipeline (or a Taskfile task) as a supervised job",
		Long: "Runs pipelines.<name> from rocket.yaml; unknown names fall back to `task <name>` of the project's Taskfile.\n" +
			"Steps run sequentially and stop at the first failure. Arguments after -- go to the last step.\n" +
			"Blocks until done (exit 0 on success, 2 on failure); --detach returns the job id.",
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if cmd.ArgsLenAtDash() == 0 {
				return errors.New("pipeline name must come before --")
			}
			return runJob(cmd.Context(), g, app.JobRequest{Kind: domain.JobPipeline, Name: args[0], Args: args[1:]}, f)
		},
	}
	f.register(cmd)
	f.registerSelection(cmd)
	return cmd
}

func setupCmd(g *globals) *cobra.Command {
	var f jobFlags
	cmd := &cobra.Command{
		Use:   "setup [name]",
		Short: "Run setup.doctor → setup.install → setup.migrate (skipping undefined), or one setup.<name>",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			req := app.JobRequest{Kind: domain.JobSetup}
			if len(args) == 1 {
				req.Name = args[0]
			}
			return runJob(cmd.Context(), g, req, f)
		},
	}
	f.register(cmd)
	f.registerSelection(cmd)
	return cmd
}

// setupStepCmd is `rocket doctor|install|migrate`.
func setupStepCmd(g *globals, name string) *cobra.Command {
	var f jobFlags
	cmd := &cobra.Command{
		Use:   name,
		Short: fmt.Sprintf("Run setup.%s from rocket.yaml", name),
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			return runJob(cmd.Context(), g, app.JobRequest{Kind: domain.JobSetup, Name: name}, f)
		},
	}
	f.register(cmd)
	f.registerSelection(cmd)
	return cmd
}

func deployCmd(g *globals) *cobra.Command {
	var f jobFlags
	var yes bool
	cmd := &cobra.Command{
		Use:   "deploy <env>",
		Short: "Run envs.<env>.deploy (needs --yes when confirm: true)",
		Long: "Envs with `confirm: true` require --yes. Agents (owner agent:*) are refused unless --yes is passed AND\n" +
			"ROCKET_ALLOW_DEPLOY=1 is set by a human. A refused deploy exits 3.",
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			req := app.JobRequest{Kind: domain.JobDeploy, Name: args[0], Yes: yes, AllowAgentDeploy: os.Getenv("ROCKET_ALLOW_DEPLOY") == "1"}
			return runJob(cmd.Context(), g, req, f)
		},
	}
	f.register(cmd)
	cmd.Flags().BoolVar(&yes, "yes", false, "confirm the deploy")
	return cmd
}

func durationStr(ms int64) string {
	return (time.Duration(ms) * time.Millisecond).Round(100 * time.Millisecond).String()
}

func jobsCmd(g *globals) *cobra.Command {
	var allProjects bool
	var limit int
	cmd := &cobra.Command{
		Use:   "jobs",
		Short: "List recent jobs (newest first)",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			req := app.JobsRequest{AllProjects: allProjects, Limit: limit}
			if !allProjects {
				ref, err := g.projectRef()
				if err != nil && g.project != "" {
					return err
				}
				req.Project = ref // empty when no rocket.yaml: all projects
			}
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			res, err := c.Jobs(cmd.Context(), req)
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
				return nil
			}
			w := table()
			fmt.Fprintln(w, "ID\tPROJECT\tKIND\tNAME\tSTATUS\tEXIT\tDURATION\tOWNER\tSTARTED")
			for _, j := range res.Jobs {
				fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", j.ID, j.Project, j.Kind, j.Name, j.Status,
					exitStr(j.ExitCode), durationStr(j.DurationMS), j.Owner, j.StartedAt.Local().Format("01-02 15:04:05"))
			}
			return w.Flush()
		},
	}
	cmd.Flags().BoolVar(&allProjects, "all-projects", false, "show jobs of every project")
	cmd.Flags().IntVar(&limit, "limit", 50, "maximum number of jobs")
	return cmd
}

func jobCmd(g *globals) *cobra.Command {
	var follow bool
	var tail int
	cmd := &cobra.Command{
		Use:   "job <id> [logs]",
		Short: "Show a job, or its log with `logs` (-f to follow)",
		Args:  cobra.RangeArgs(1, 2),
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 2 && args[1] != "logs" {
				return fmt.Errorf("unknown job action %q (use `rocket job <id> logs` or `rocket job cancel <id>`)", args[1])
			}
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			id := args[0]
			if len(args) == 1 {
				j, err := c.Job(cmd.Context(), id, false)
				if err != nil {
					return err
				}
				if g.json {
					printJSON(j)
					return nil
				}
				printJob(j)
				return nil
			}
			if !follow {
				res, err := c.JobLogs(cmd.Context(), id, tail)
				if err != nil {
					return err
				}
				if g.json {
					printJSON(res)
					return nil
				}
				for _, l := range res.Lines {
					fmt.Println(l)
				}
				return nil
			}
			ctx, stop := signal.NotifyContext(cmd.Context(), os.Interrupt, syscall.SIGTERM)
			defer stop()
			t := -1
			if cmd.Flags().Changed("tail") {
				t = tail
			}
			return c.FollowJobLogs(ctx, id, t, func(e domain.Event) error {
				switch {
				case g.json:
					printJSONLine(e)
				case e.Type == domain.EventJobLog:
					fmt.Println(e.Line)
				}
				return nil
			})
		},
	}
	cmd.Flags().BoolVarP(&follow, "follow", "f", false, "stream the log until the job ends")
	cmd.Flags().IntVar(&tail, "tail", 100, "number of lines (with -f: backlog lines; default whole log)")
	cmd.AddCommand(&cobra.Command{
		Use:   "cancel <id>",
		Short: "Cancel a running job (stops its process group)",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			j, err := c.CancelJob(cmd.Context(), args[0])
			if err != nil {
				return err
			}
			if g.json {
				printJSON(j)
			} else {
				fmt.Printf("job %s %s\n", j.ID, j.Status)
			}
			return nil
		},
	})
	return cmd
}

func printJob(j domain.Job) {
	w := table()
	fmt.Fprintf(w, "id\t%s\n", j.ID)
	fmt.Fprintf(w, "project\t%s\n", j.Project)
	fmt.Fprintf(w, "kind/name\t%s %s\n", j.Kind, j.Name)
	fmt.Fprintf(w, "status\t%s (exit %s)\n", j.Status, exitStr(j.ExitCode))
	fmt.Fprintf(w, "owner\t%s\n", j.Owner)
	fmt.Fprintf(w, "duration\t%s\n", durationStr(j.DurationMS))
	for i, s := range j.Steps {
		mark := " "
		if i+1 == j.Step {
			mark = ">"
		}
		fmt.Fprintf(w, "step %d\t%s %s\n", i+1, mark, s.Describe())
	}
	if len(j.Args) > 0 {
		fmt.Fprintf(w, "args\t%s\n", strings.Join(j.Args, " "))
	}
	if j.Error != "" {
		fmt.Fprintf(w, "error\t%s\n", j.Error)
	}
	fmt.Fprintf(w, "log\t%s\n", j.LogPath)
	w.Flush()
}
