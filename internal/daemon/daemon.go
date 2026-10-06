// Package daemon is rocketd's composition root: it wires adapters into the
// application and serves the API on the unix socket.
package daemon

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"sync"
	"syscall"
	"time"

	"github.com/xean-io/rocket/internal/adapters/api"
	"github.com/xean-io/rocket/internal/adapters/compose"
	"github.com/xean-io/rocket/internal/adapters/events"
	"github.com/xean-io/rocket/internal/adapters/logs"
	"github.com/xean-io/rocket/internal/adapters/probe"
	"github.com/xean-io/rocket/internal/adapters/process"
	"github.com/xean-io/rocket/internal/adapters/sqlite"
	"github.com/xean-io/rocket/internal/adapters/task"
	"github.com/xean-io/rocket/internal/app"
	"github.com/xean-io/rocket/internal/manifest"
	"github.com/xean-io/rocket/internal/paths"
)

// TTLInterval is how often persisted service/job deadlines are checked.
const TTLInterval = 5 * time.Second

// Run serves until SIGINT/SIGTERM or POST /v1/shutdown.
func Run(ctx context.Context, p paths.Paths, version string) error {
	if err := p.Ensure(); err != nil {
		return err
	}
	unlock, err := lockFile(p.LockFile)
	if err != nil {
		return fmt.Errorf("another rocketd holds %s: %w", p.LockFile, err)
	}
	defer unlock()

	if conn, err := net.DialTimeout("unix", p.Socket, 500*time.Millisecond); err == nil {
		conn.Close()
		return errors.New("rocketd is already running")
	}
	_ = os.Remove(p.Socket) // stale socket from a crashed daemon

	store, err := sqlite.Open(p.DB)
	if err != nil {
		return err
	}
	defer store.Close()
	bus := events.New()
	sink := logs.New(p.Logs, bus)
	defer sink.Close()

	application := app.New(app.Deps{
		Store:     store,
		Manifests: manifest.Loader{},
		Env:       manifest.EnvFiles{},
		Runner:    process.Runner{},
		Compose:   compose.Driver{},
		Tasks:     task.Driver{},
		Probe:     probe.Ports{},
		Health:    probe.Health{},
		Bus:       bus,
		Logs:      sink,
	})
	defer application.Close()

	rec, err := application.Reconcile(ctx)
	if err != nil {
		log.Printf("reconcile: %v", err)
	} else {
		log.Printf("reconcile: adopted=%d dead=%d released_leases=%d lost_jobs=%d",
			len(rec.Adopted), len(rec.Dead), len(rec.ReleasedLeases), len(rec.LostJobs))
	}

	ln, err := net.Listen("unix", p.Socket)
	if err != nil {
		return err
	}
	_ = os.Chmod(p.Socket, 0o600)
	_ = os.WriteFile(p.PIDFile, []byte(strconv.Itoa(os.Getpid())), 0o600)
	defer os.Remove(p.PIDFile)
	defer os.Remove(p.Socket)

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	var once sync.Once
	stop := func() { once.Do(cancel) }

	// The TCP listener (for Rocket.app) serves the same API behind a bearer
	// token; the unix socket (mode 0600) stays token-free for the CLI.
	startedAt := time.Now().UTC()
	tcpLn, tcpErr := net.Listen("tcp", "127.0.0.1:0")
	httpURL := ""
	if tcpErr != nil {
		log.Printf("tcp listener disabled: %v", tcpErr)
	} else {
		httpURL = "http://" + tcpLn.Addr().String()
	}
	handler := (&api.Server{
		App: application,
		Bus: bus,
		Info: api.HealthInfo{OK: true, API: api.Version, Version: version, PID: os.Getpid(),
			StartedAt: startedAt, Home: p.Home, Socket: p.Socket, HTTP: httpURL},
		Shutdown: stop,
	}).Handler()
	srv := &http.Server{Handler: handler, ReadHeaderTimeout: 10 * time.Second}
	var tcpSrv *http.Server
	if tcpLn != nil {
		token, err := NewToken()
		if err != nil {
			return err
		}
		tcpSrv = &http.Server{Handler: RequireToken(token, handler), ReadHeaderTimeout: 10 * time.Second}
		info := Info{Version: version, API: api.Version, PID: os.Getpid(), Socket: p.Socket, HTTP: httpURL,
			Token: token, RocketBin: executable(), StartedAt: startedAt}
		if err := WriteInfo(p.DaemonJSON, info); err != nil {
			return fmt.Errorf("write %s: %w", p.DaemonJSON, err)
		}
		defer os.Remove(p.DaemonJSON)
	}

	sigs := make(chan os.Signal, 1)
	signal.Notify(sigs, syscall.SIGINT, syscall.SIGTERM)
	signal.Ignore(syscall.SIGHUP)
	defer signal.Stop(sigs)
	go func() {
		select {
		case s := <-sigs:
			log.Printf("received %s, shutting down", s)
			stop()
		case <-ctx.Done():
		}
	}()

	go func() {
		t := time.NewTicker(TTLInterval)
		defer t.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-t.C:
				if expired, err := application.ExpireJobsTTL(ctx); err != nil {
					log.Printf("job ttl: %v", err)
				} else {
					for _, job := range expired {
						log.Printf("ttl expired: job %s (%s/%s)", job.ID, job.Project, job.Name)
					}
				}
				if expired, err := application.ExpireTTL(ctx); err != nil {
					log.Printf("ttl: %v", err)
				} else {
					for _, r := range expired {
						log.Printf("ttl expired: %s/%s (owner %s)", r.Project, r.Service, r.Owner)
					}
				}
			}
		}
	}()

	errCh := make(chan error, 2)
	go func() { errCh <- srv.Serve(ln) }()
	if tcpSrv != nil {
		go func() { errCh <- tcpSrv.Serve(tcpLn) }()
	}
	log.Printf("rocketd %s listening on %s and %s (pid %d)", version, p.Socket, orNone(httpURL), os.Getpid())

	select {
	case <-ctx.Done():
	case err := <-errCh:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	}
	// Remove discovery files before the listeners close, so a client that
	// sees the socket go away never finds a stale daemon.json.
	if tcpSrv != nil {
		_ = os.Remove(p.DaemonJSON)
	}
	shutdownCtx, c := context.WithTimeout(context.Background(), 5*time.Second)
	defer c()
	if tcpSrv != nil {
		_ = tcpSrv.Shutdown(shutdownCtx)
	}
	_ = srv.Shutdown(shutdownCtx)
	log.Printf("rocketd stopped; supervised services keep running and will be adopted on next start")
	return nil
}

func orNone(s string) string {
	if s == "" {
		return "(no tcp)"
	}
	return s
}
