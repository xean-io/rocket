// Package probe inspects host ports (free? who holds it?) and performs
// TCP/HTTP readiness checks.
package probe

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"net/http"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

// Ports implements ports.PortProbe.
type Ports struct {
	LsofBin string // defaults to "lsof"
}

var _ ports.PortProbe = Ports{}

// Free reports whether nothing accepts connections on the port and it can be
// bound on both loopback and the wildcard address.
func (Ports) Free(port int) bool {
	p := strconv.Itoa(port)
	for _, host := range []string{"127.0.0.1", "::1"} {
		if c, err := net.DialTimeout("tcp", net.JoinHostPort(host, p), 150*time.Millisecond); err == nil {
			c.Close()
			return false
		}
	}
	for _, host := range []string{"127.0.0.1", "0.0.0.0"} {
		l, err := net.Listen("tcp", net.JoinHostPort(host, p))
		if err != nil {
			return false
		}
		l.Close()
	}
	return true
}

// Holder reports the process listening on port using lsof. It returns nil
// when no holder is found or lsof is unavailable.
func (pp Ports) Holder(port int) (*domain.PortHolder, error) {
	bin := pp.LsofBin
	if bin == "" {
		bin = "lsof"
	}
	if _, err := exec.LookPath(bin); err != nil {
		return nil, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	out, _ := exec.CommandContext(ctx, bin, "-nP", fmt.Sprintf("-iTCP:%d", port), "-sTCP:LISTEN", "-Fpcn").Output()
	h := ParseLsof(string(out))
	if h == nil {
		return nil, nil
	}
	cwdOut, _ := exec.CommandContext(ctx, bin, "-a", "-p", strconv.Itoa(h.PID), "-d", "cwd", "-Fn").Output()
	h.Cwd = parseCwd(string(cwdOut))
	return h, nil
}

// ParseLsof parses `lsof -F pcn` output and returns the first process.
func ParseLsof(out string) *domain.PortHolder {
	var h *domain.PortHolder
	sc := bufio.NewScanner(strings.NewReader(out))
	for sc.Scan() {
		line := sc.Text()
		if line == "" {
			continue
		}
		switch line[0] {
		case 'p':
			if h != nil {
				return h
			}
			pid, err := strconv.Atoi(line[1:])
			if err != nil {
				continue
			}
			h = &domain.PortHolder{PID: pid}
		case 'c':
			if h != nil {
				h.Command = line[1:]
			}
		}
	}
	return h
}

func parseCwd(out string) string {
	for _, line := range strings.Split(out, "\n") {
		if strings.HasPrefix(line, "n") {
			return line[1:]
		}
	}
	return ""
}

// Health implements ports.HealthProbe.
type Health struct {
	Client *http.Client
}

var _ ports.HealthProbe = Health{}

// Check performs one probe against loopback (IPv4, then IPv6).
func (h Health) Check(ctx context.Context, c domain.HealthCheck) error {
	var lastErr error
	for _, host := range []string{"127.0.0.1", "::1"} {
		addr := net.JoinHostPort(host, strconv.Itoa(c.Port))
		switch c.Kind {
		case "http":
			lastErr = h.httpCheck(ctx, "http://"+addr+c.Path)
		default:
			var d net.Dialer
			conn, err := d.DialContext(ctx, "tcp", addr)
			if err == nil {
				conn.Close()
			}
			lastErr = err
		}
		if lastErr == nil {
			return nil
		}
	}
	return lastErr
}

func (h Health) httpCheck(ctx context.Context, url string) error {
	client := h.Client
	if client == nil {
		client = &http.Client{Timeout: 2 * time.Second}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	resp.Body.Close()
	if resp.StatusCode >= 400 {
		return fmt.Errorf("GET %s: status %d", url, resp.StatusCode)
	}
	return nil
}
