package compose

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os/exec"
	"slices"
	"sort"
	"strings"

	"github.com/xean-io/rocket/internal/ports"
)

// NamedVolumes selects this service's nonexternal named mounts from the
// normalized Compose model. Bind and anonymous mounts are not first-run data.
func (d Driver) NamedVolumes(ctx context.Context, target ports.ComposeTarget, service string) ([]string, error) {
	var out bytes.Buffer
	cmd := d.cmd(ctx, target, &out, "config", "--format", "json")
	cmd.Stderr = io.Discard
	if err := run(cmd); err != nil {
		return nil, err
	}
	return namedVolumesFromConfig(out.Bytes(), service)
}

func namedVolumesFromConfig(data []byte, service string) ([]string, error) {
	var config struct {
		Services map[string]struct {
			Volumes []struct {
				Type   string `json:"type"`
				Source string `json:"source"`
			} `json:"volumes"`
		} `json:"services"`
		Volumes map[string]struct {
			Name     string `json:"name"`
			External bool   `json:"external"`
		} `json:"volumes"`
	}
	if err := json.Unmarshal(data, &config); err != nil {
		return nil, fmt.Errorf("decode compose config: %w", err)
	}
	svc, ok := config.Services[service]
	if !ok {
		return nil, fmt.Errorf("compose config has no service %q", service)
	}
	set := map[string]bool{}
	for _, mount := range svc.Volumes {
		if mount.Type != "volume" || mount.Source == "" {
			continue
		}
		volume, ok := config.Volumes[mount.Source]
		if !ok {
			return nil, fmt.Errorf("compose config has no volume %q", mount.Source)
		}
		if volume.External {
			continue
		}
		if volume.Name == "" {
			return nil, fmt.Errorf("compose volume %q has no normalized name", mount.Source)
		}
		set[volume.Name] = true
	}
	names := make([]string, 0, len(set))
	for name := range set {
		names = append(names, name)
	}
	sort.Strings(names)
	return names, nil
}

// VolumeExists treats only a successful listing as absence evidence. Docker's
// inspect command also fails on missing volumes, obscuring operational errors.
func (d Driver) VolumeExists(ctx context.Context, target ports.ComposeTarget, name string) (bool, error) {
	var out bytes.Buffer
	cmd := exec.CommandContext(ctx, d.bin(), "volume", "ls", "--format", "{{.Name}}")
	cmd.Dir = target.Dir
	if len(target.Env) > 0 {
		cmd.Env = target.Env
	}
	cmd.Stdout, cmd.Stderr = &out, io.Discard
	if err := run(cmd); err != nil {
		return false, err
	}
	return slices.Contains(strings.Split(strings.TrimSpace(out.String()), "\n"), name), nil
}
