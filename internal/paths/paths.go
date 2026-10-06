// Package paths resolves rocket's on-disk layout (~/.rocket or $ROCKET_HOME).
package paths

import (
	"fmt"
	"os"
	"path/filepath"
)

// Paths is the daemon's file layout.
type Paths struct {
	Home      string `json:"home"`
	Socket    string `json:"socket"`
	DB        string `json:"db"`
	Logs      string `json:"logs"`
	PIDFile   string `json:"pid_file"`
	LockFile  string `json:"lock_file"`
	DaemonLog string `json:"daemon_log"`
	// DaemonJSON tells GUI clients the TCP address and bearer token.
	DaemonJSON string `json:"daemon_json"`
}

// maxSocketPath is the conservative sun_path limit (macOS: 104 bytes).
const maxSocketPath = 103

// Resolve returns the layout rooted at $ROCKET_HOME or ~/.rocket.
func Resolve() (Paths, error) {
	home := os.Getenv("ROCKET_HOME")
	if home == "" {
		h, err := os.UserHomeDir()
		if err != nil {
			return Paths{}, err
		}
		home = filepath.Join(h, ".rocket")
	}
	home, err := filepath.Abs(home)
	if err != nil {
		return Paths{}, err
	}
	p := Paths{
		Home:       home,
		Socket:     filepath.Join(home, "rocketd.sock"),
		DB:         filepath.Join(home, "state.db"),
		Logs:       filepath.Join(home, "logs"),
		PIDFile:    filepath.Join(home, "rocketd.pid"),
		LockFile:   filepath.Join(home, "rocketd.lock"),
		DaemonLog:  filepath.Join(home, "rocketd.log"),
		DaemonJSON: filepath.Join(home, "daemon.json"),
	}
	if len(p.Socket) > maxSocketPath {
		return Paths{}, fmt.Errorf("socket path %s is too long for a unix socket; set ROCKET_HOME to a shorter directory", p.Socket)
	}
	return p, nil
}

// Ensure creates the home and logs directories with private permissions.
func (p Paths) Ensure() error {
	for _, d := range []string{p.Home, p.Logs} {
		if err := os.MkdirAll(d, 0o700); err != nil {
			return err
		}
	}
	return nil
}
