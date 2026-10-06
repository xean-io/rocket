package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestRequireTokenGuardsEveryRequest(t *testing.T) {
	token, err := NewToken()
	if err != nil {
		t.Fatal(err)
	}
	if len(token) != 64 {
		t.Fatalf("token %q: want 32 bytes hex", token)
	}
	ok := http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(http.StatusOK) })
	handler := RequireToken(token, ok)

	tests := []struct {
		name   string
		header string
		want   int
	}{
		{"no header", "", http.StatusUnauthorized},
		{"wrong token", "Bearer " + token[:63] + "x", http.StatusUnauthorized},
		{"wrong scheme", "Basic " + token, http.StatusUnauthorized},
		{"short token", "Bearer abc", http.StatusUnauthorized},
		{"valid token", "Bearer " + token, http.StatusOK},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodGet, "/v1/health", nil)
			if tt.header != "" {
				req.Header.Set("Authorization", tt.header)
			}
			rec := httptest.NewRecorder()
			handler.ServeHTTP(rec, req)
			if rec.Code != tt.want {
				t.Fatalf("status %d, want %d", rec.Code, tt.want)
			}
			if tt.want == http.StatusUnauthorized {
				var body struct{ Error, Code string }
				_ = json.NewDecoder(rec.Body).Decode(&body)
				if body.Code != "unauthorized" || rec.Header().Get("WWW-Authenticate") == "" {
					t.Fatalf("401 body %+v headers %v", body, rec.Header())
				}
			}
		})
	}
}

func TestWriteInfoIsPrivateAndComplete(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.json")
	// A pre-existing world-readable file must not keep its mode.
	if err := os.WriteFile(path, []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}
	info := Info{Version: "1.2.3", API: "v1", PID: 42, Socket: "/s.sock", HTTP: "http://127.0.0.1:1234",
		Token: "abc", RocketBin: "/usr/local/bin/rocket", StartedAt: time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)}
	if err := WriteInfo(path, info); err != nil {
		t.Fatal(err)
	}
	st, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if st.Mode().Perm() != 0o600 {
		t.Fatalf("mode %v, want 0600", st.Mode().Perm())
	}
	data, _ := os.ReadFile(path)
	var raw map[string]any
	if err := json.Unmarshal(data, &raw); err != nil {
		t.Fatal(err)
	}
	for _, k := range []string{"version", "pid", "socket", "http", "token", "rocket_bin", "started_at"} {
		if _, ok := raw[k]; !ok {
			t.Errorf("daemon.json missing %q: %s", k, data)
		}
	}
	got, err := ReadInfo(path)
	if err != nil || got != info {
		t.Fatalf("read back %+v err %v", got, err)
	}
}
