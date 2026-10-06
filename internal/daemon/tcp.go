package daemon

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/xean-io/rocket/internal/adapters/api"
)

// Info is $ROCKET_HOME/daemon.json: how GUI clients (Rocket.app) reach the
// daemon's token-protected TCP listener. It exists only while rocketd runs.
type Info struct {
	Version   string    `json:"version"`
	API       string    `json:"api"`
	PID       int       `json:"pid"`
	Socket    string    `json:"socket"`
	HTTP      string    `json:"http"` // e.g. http://127.0.0.1:53124
	Token     string    `json:"token"`
	RocketBin string    `json:"rocket_bin"` // absolute path of the running rocket binary
	StartedAt time.Time `json:"started_at"`
}

// NewToken returns 32 random bytes, hex encoded.
func NewToken() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// RequireToken rejects every request without `Authorization: Bearer <token>`.
func RequireToken(token string, next http.Handler) http.Handler {
	want := []byte(token)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
		if !ok || subtle.ConstantTimeCompare([]byte(got), want) != 1 {
			w.Header().Set("WWW-Authenticate", `Bearer realm="rocketd"`)
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusUnauthorized)
			_ = json.NewEncoder(w).Encode(api.ErrorBody{Error: "missing or invalid bearer token (see daemon.json)", Code: "unauthorized"})
			return
		}
		next.ServeHTTP(w, r)
	})
}

// WriteInfo atomically writes info to path with mode 0600.
func WriteInfo(path string, info Info) error {
	data, err := json.MarshalIndent(info, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), ".daemon-*.json")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(append(data, '\n')); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}

// ReadInfo reads daemon.json.
func ReadInfo(path string) (Info, error) {
	var info Info
	data, err := os.ReadFile(path)
	if err != nil {
		return info, err
	}
	err = json.Unmarshal(data, &info)
	return info, err
}

// executable returns the absolute, symlink-resolved path of this binary.
func executable() string {
	exe, err := os.Executable()
	if err != nil {
		return ""
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		return resolved
	}
	return exe
}
