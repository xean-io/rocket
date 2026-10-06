package manifest

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
)

// ParseDotenv parses KEY=VALUE lines (optional `export`, quotes, # comments).
// Malformed lines are ignored.
func ParseDotenv(data []byte) map[string]string {
	out := map[string]string{}
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(strings.TrimSuffix(line, "\r"))
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		line = strings.TrimPrefix(line, "export ")
		k, v, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		k = strings.TrimSpace(k)
		if !envVarRe.MatchString(k) {
			continue
		}
		v = strings.TrimSpace(v)
		switch {
		case len(v) >= 2 && (v[0] == '"' || v[0] == '\'') && strings.IndexByte(v[1:], v[0]) >= 0:
			q := v[0]
			v = v[1 : 1+strings.IndexByte(v[1:], q)]
			if q == '"' {
				v = strings.ReplaceAll(v, `\n`, "\n")
			}
		default:
			if i := strings.Index(v, " #"); i >= 0 {
				v = strings.TrimSpace(v[:i])
			}
		}
		out[k] = v
	}
	return out
}

// EnvFiles reads dotenv files relative to a project root. It implements
// ports.EnvSource.
type EnvFiles struct{}

// Dotenv merges the given files in order; missing files are skipped.
func (EnvFiles) Dotenv(root string, files []string) (map[string]string, error) {
	out := map[string]string{}
	for _, f := range files {
		path := f
		if !filepath.IsAbs(path) {
			path = filepath.Join(root, f)
		}
		data, err := os.ReadFile(path)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return nil, err
		}
		for k, v := range ParseDotenv(data) {
			out[k] = v
		}
	}
	return out, nil
}
