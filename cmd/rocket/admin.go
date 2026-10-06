package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/spf13/cobra"

	"github.com/xean-io/rocket/internal/adapters/api"
	"github.com/xean-io/rocket/internal/client"
	"github.com/xean-io/rocket/internal/daemon"
	"github.com/xean-io/rocket/internal/manifest"
	"github.com/xean-io/rocket/internal/paths"
)

func printJSONLine(v any) {
	data, _ := json.Marshal(v)
	fmt.Println(string(data))
}

func projectsCmd(g *globals) *cobra.Command {
	cmd := &cobra.Command{Use: "projects", Short: "Manage the global project registry"}
	cmd.AddCommand(&cobra.Command{
		Use:   "add [path]",
		Short: "Register a project (default: rocket.yaml found from cwd)",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 1 {
				g.project = args[0]
				if !looksLikePath(g.project) {
					g.project = "./" + g.project
				}
			}
			ref, err := g.projectRef()
			if err != nil {
				return err
			}
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			res, err := c.AddProject(cmd.Context(), ref)
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
			} else {
				fmt.Printf("registered %s -> %s\n", res.Name, res.Path)
			}
			return nil
		},
	}, &cobra.Command{
		Use:     "ls",
		Aliases: []string{"list"},
		Short:   "List registered projects",
		RunE: func(cmd *cobra.Command, _ []string) error {
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			res, err := c.Projects(cmd.Context())
			if err != nil {
				return err
			}
			if g.json {
				printJSON(res)
				return nil
			}
			w := table()
			fmt.Fprintln(w, "NAME\tPATH")
			for _, p := range res.Projects {
				fmt.Fprintf(w, "%s\t%s\n", p.Name, p.Path)
			}
			return w.Flush()
		},
	}, &cobra.Command{
		Use:     "rm <name>",
		Aliases: []string{"remove"},
		Short:   "Unregister a project (must have nothing running)",
		Args:    cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			c, err := daemonClient(cmd.Context())
			if err != nil {
				return err
			}
			if err := c.RemoveProject(cmd.Context(), args[0]); err != nil {
				return err
			}
			if g.json {
				printJSON(map[string]string{"removed": args[0]})
			} else {
				fmt.Printf("removed %s\n", args[0])
			}
			return nil
		},
	})
	return cmd
}

// DaemonStatus is the JSON shape of `rocket daemon status|start|stop`.
type DaemonStatus struct {
	Running bool            `json:"running"`
	Info    *api.HealthInfo `json:"info,omitempty"`
	Paths   paths.Paths     `json:"paths"`
}

func daemonCmd(g *globals) *cobra.Command {
	cmd := &cobra.Command{Use: "daemon", Short: "Manage rocketd"}
	report := func(p paths.Paths) error {
		info, ok := client.Running(context.Background(), p)
		st := DaemonStatus{Running: ok, Paths: p}
		if ok {
			st.Info = &info
		}
		if g.json {
			printJSON(st)
		} else if ok {
			fmt.Printf("rocketd running (pid %d, %s, up %s)\nsocket %s\n", info.PID, info.Version,
				time.Since(info.StartedAt).Round(time.Second), info.Socket)
		} else {
			fmt.Println("rocketd not running")
		}
		if !ok {
			return &exitError{code: 1}
		}
		return nil
	}
	cmd.AddCommand(&cobra.Command{
		Use:   "run",
		Short: "Run the daemon in the foreground",
		RunE: func(cmd *cobra.Command, _ []string) error {
			p, err := paths.Resolve()
			if err != nil {
				return err
			}
			return daemon.Run(cmd.Context(), p, version)
		},
	}, &cobra.Command{
		Use:   "start",
		Short: "Start the daemon in the background (no-op when running)",
		RunE: func(cmd *cobra.Command, _ []string) error {
			p, err := paths.Resolve()
			if err != nil {
				return err
			}
			if _, err := client.Ensure(cmd.Context(), p); err != nil {
				return err
			}
			return report(p)
		},
	}, &cobra.Command{
		Use:   "stop",
		Short: "Stop the daemon (supervised services keep running and are adopted on next start)",
		RunE: func(cmd *cobra.Command, _ []string) error {
			p, err := paths.Resolve()
			if err != nil {
				return err
			}
			if _, ok := client.Running(cmd.Context(), p); ok {
				if err := client.New(p.Socket).Shutdown(cmd.Context()); err != nil {
					return err
				}
				deadline := time.Now().Add(10 * time.Second)
				for time.Now().Before(deadline) {
					if _, ok := client.Running(cmd.Context(), p); !ok {
						break
					}
					time.Sleep(100 * time.Millisecond)
				}
			}
			if _, ok := client.Running(cmd.Context(), p); ok {
				return fmt.Errorf("rocketd still running after 10s")
			}
			if g.json {
				printJSON(DaemonStatus{Running: false, Paths: p})
			} else {
				fmt.Println("rocketd stopped")
			}
			return nil
		},
	}, &cobra.Command{
		Use:   "status",
		Short: "Show daemon status (exit 1 when not running)",
		RunE: func(cmd *cobra.Command, _ []string) error {
			p, err := paths.Resolve()
			if err != nil {
				return err
			}
			return report(p)
		},
	})
	return cmd
}

func schemaCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "schema",
		Short: "Print the JSON Schema of rocket.yaml",
		RunE: func(*cobra.Command, []string) error {
			_, err := os.Stdout.Write(append(manifest.Schema(), '\n'))
			return err
		},
	}
}
