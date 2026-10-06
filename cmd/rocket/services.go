package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/signal"
	"sort"
	"strings"
	"syscall"
	"text/tabwriter"
	"time"

	"github.com/spf13/cobra"

	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
)

func table() *tabwriter.Writer { return tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0) }

func formatPorts(ports map[string]int) string {
	if len(ports) == 0 {
		return "-"
	}
	names := make([]string, 0, len(ports))
	for n := range ports {
		names = append(names, n)
	}
	sort.Strings(names)
	parts := make([]string, 0, len(names))
	for _, n := range names {
		parts = append(parts, fmt.Sprintf("%s=%d", n, ports[n]))
	}
	return strings.Join(parts, ",")
}

func orDash(s string) string {
	if s == "" {
		return "-"
	}
	return s
}

func pidStr(pid int) string {
	if pid == 0 {
		return "-"
	}
	return fmt.Sprint(pid)
}

func expiresStr(t *time.Time) string {
	if t == nil {
		return "-"
	}
	d := time.Until(*t).Round(time.Second)
	if d < 0 {
		return "expired"
	}
	return d.String()
}

func printUp(res app.UpResult) {
	writeUp(os.Stdout, res)
}

func writeUp(out io.Writer, res app.UpResult) {
	fmt.Fprintf(out, "project %s (env %s)\n", res.Project, res.Env)
	w := tabwriter.NewWriter(out, 0, 2, 2, ' ', 0)
	fmt.Fprintln(w, "SERVICE\tACTION\tSTATE\tPORTS\tNOTE")
	for _, s := range res.Services {
		var notes []string
		for _, r := range s.Remaps {
			notes = append(notes, fmt.Sprintf("%s %d->%d via %s (%s)", r.Name, r.From, r.To, r.Env, r.Reason))
		}
		if s.Error != "" {
			notes = append(notes, s.Error)
		}
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\n", s.Service, s.Action, s.State, formatPorts(s.Ports), strings.Join(notes, "; "))
	}
	w.Flush()
	for _, hint := range res.Hints {
		fmt.Fprintf(out, "hint: %s\n", hint)
	}
}

func upCmd(g *globals) *cobra.Command {
	var env, ownerFlag, ttl string
	var profiles []string
	cmd := &cobra.Command{
		Use:   "up [service|group...]",
		Short: "Start services (and their dependencies); idempotent",
		RunE: func(cmd *cobra.Command, args []string) error {
			return runUp(cmd.Context(), g, args, env, ownerFlag, ttl, profiles, false)
		},
	}
	cmd.Flags().StringVar(&env, "env", "", "environment (default: project default_env)")
	cmd.Flags().StringArrayVar(&profiles, "profile", nil, "enable a startup profile (repeatable)")
	cmd.Flags().StringVar(&ownerFlag, "owner", "", "owner tag (default: $ROCKET_OWNER or \"user\")")
	cmd.Flags().StringVar(&ttl, "ttl", "", "stop automatically after this duration, e.g. 30m")
	return cmd
}

func restartCmd(g *globals) *cobra.Command {
	var env, ownerFlag, ttl string
	var profiles []string
	cmd := &cobra.Command{
		Use:   "restart <service|group...>",
		Short: "Stop then start services",
		Args:  cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runUp(cmd.Context(), g, args, env, ownerFlag, ttl, profiles, true)
		},
	}
	cmd.Flags().StringVar(&env, "env", "", "environment (default: project default_env)")
	cmd.Flags().StringArrayVar(&profiles, "profile", nil, "enable a startup profile (repeatable)")
	cmd.Flags().StringVar(&ownerFlag, "owner", "", "owner tag (default: $ROCKET_OWNER or \"user\")")
	cmd.Flags().StringVar(&ttl, "ttl", "", "stop automatically after this duration, e.g. 30m")
	return cmd
}

func runUp(ctx context.Context, g *globals, args []string, env, ownerFlag, ttl string, profiles []string, restart bool) error {
	ref, err := g.projectRef()
	if err != nil {
		return err
	}
	c, err := daemonClient(ctx)
	if err != nil {
		return err
	}
	req := app.UpRequest{Project: ref, Services: args, Env: env, Profiles: profiles, Owner: owner(ownerFlag), TTL: ttl}
	var res app.UpResult
	if restart {
		res, err = c.Restart(ctx, req)
	} else {
		res, err = c.Up(ctx, req)
	}
	if err != nil {
		return err
	}
	if g.json {
		printJSON(res)
	} else {
		printUp(res)
	}
	if res.Failed() {
		return &exitError{code: 2}
	}
	return nil
}

func downCmd(g *globals) *cobra.Command {
	var all, everywhere bool
	var ownerFlag string
	cmd := &cobra.Command{
		Use:   "down [service|group...]",
		Short: "Stop services (reverse dependency order)",
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 && !all && !everywhere && ownerFlag == "" {
				return errors.New("nothing selected: pass services/groups, --all, --owner <owner> or --everywhere")
			}
			req := app.DownRequest{Services: args, Owner: ownerFlag, Everywhere: everywhere}
			if !everywhere {
				ref, err := g.projectRef()
				if err != nil {
					if all && g.project == "" && errors.Is(err, manifest.ErrNoManifest) {
						return fmt.Errorf("%w; use --everywhere to stop services across all registered projects", err)
					}
					return err
				}
				req.Project = ref
			}
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			res, err := c.Down(cmd.Context(), req)
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
			} else {
				if len(res.Stopped) == 0 {
					fmt.Println("nothing running")
				}
				for _, r := range res.Stopped {
					fmt.Printf("stopped %s/%s\n", r.Project, r.Service)
				}
				for _, n := range res.ComposeDown {
					fmt.Printf("compose down %s\n", n)
				}
				for _, e := range res.Errors {
					fmt.Fprintf(os.Stderr, "error: %s\n", e)
				}
			}
			if len(res.Errors) > 0 {
				return &exitError{code: 2}
			}
			return nil
		},
	}
	cmd.Flags().BoolVar(&all, "all", false, "stop every service of the project")
	cmd.Flags().StringVar(&ownerFlag, "owner", "", "only stop services started by this owner")
	cmd.Flags().BoolVar(&everywhere, "everywhere", false, "apply to all projects")
	return cmd
}

func psCmd(g *globals) *cobra.Command {
	var allProjects bool
	cmd := &cobra.Command{
		Use:   "ps",
		Short: "Show service state",
		RunE: func(cmd *cobra.Command, _ []string) error {
			req := app.StatusRequest{AllProjects: allProjects}
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
			res, err := c.Status(cmd.Context(), req)
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
				return nil
			}
			w := table()
			fmt.Fprintln(w, "PROJECT\tSERVICE\tKIND\tSTATE\tHEALTH\tPID\tPORTS\tOWNER\tTTL")
			for _, r := range res.Services {
				fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n", r.Project, r.Service, r.Kind, r.State,
					orDash(string(r.Health)), pidStr(r.PID), formatPorts(r.Ports), orDash(r.Owner), expiresStr(r.ExpiresAt))
			}
			return w.Flush()
		},
	}
	cmd.Flags().BoolVar(&allProjects, "all-projects", false, "show every project")
	return cmd
}

func logsCmd(g *globals) *cobra.Command {
	var follow bool
	var tail int
	cmd := &cobra.Command{
		Use:   "logs <service>",
		Short: "Show service output",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			ref, err := g.projectRef()
			if err != nil {
				return err
			}
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			req := app.LogsRequest{Project: ref, Service: args[0], Tail: tail}
			if !follow {
				res, err := c.Logs(cmd.Context(), req)
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
			return c.FollowLogs(ctx, req, func(e domain.Event) error {
				if g.json {
					printJSONLine(e)
				} else {
					fmt.Println(e.Line)
				}
				return nil
			})
		},
	}
	cmd.Flags().BoolVarP(&follow, "follow", "f", false, "stream new lines")
	cmd.Flags().IntVar(&tail, "tail", 100, "number of lines to show")
	return cmd
}

func portsCmd(g *globals) *cobra.Command {
	return &cobra.Command{
		Use:   "ports",
		Short: "Show the global port map across projects",
		RunE: func(cmd *cobra.Command, _ []string) error {
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			res, err := c.Ports(cmd.Context())
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
				return nil
			}
			w := table()
			fmt.Fprintln(w, "PORT\tPROJECT\tSERVICE\tNAME\tSTATE\tOWNER\tPID")
			for _, p := range res.Ports {
				fmt.Fprintf(w, "%d\t%s\t%s\t%s\t%s\t%s\t%s\n", p.Port, p.Project, p.Service, p.PortName, orDash(string(p.State)), orDash(p.Owner), pidStr(p.PID))
			}
			return w.Flush()
		},
	}
}

func gcCmd(g *globals) *cobra.Command {
	return &cobra.Command{
		Use:   "gc",
		Short: "Reconcile state, expire TTLs, release stale leases, prune old runs",
		RunE: func(cmd *cobra.Command, _ []string) error {
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			res, err := c.GC(cmd.Context())
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
				return nil
			}
			if len(res.Actions) == 0 {
				fmt.Println("nothing to clean")
				return nil
			}
			w := table()
			fmt.Fprintln(w, "ACTION\tPROJECT\tSERVICE\tDETAIL")
			for _, a := range res.Actions {
				detail := a.Detail
				if a.Port != 0 {
					detail = strings.TrimSpace(fmt.Sprintf("port %d %s", a.Port, detail))
				}
				fmt.Fprintf(w, "%s\t%s\t%s\t%s\n", a.Action, a.Project, a.Service, orDash(detail))
			}
			return w.Flush()
		},
	}
}
