package app

import (
	"context"
	"fmt"
	"sort"
	"strings"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/ports"
)

func (a *App) missingComposeVolumes(ctx context.Context, target ports.ComposeTarget, service string) []string {
	names, err := a.d.Compose.NamedVolumes(ctx, target, service)
	if err != nil {
		return nil // hints are advisory; incomplete evidence must not block startup
	}
	set := map[string]bool{}
	for _, name := range names {
		set[name] = true
	}
	names = make([]string, 0, len(set))
	for name := range set {
		names = append(names, name)
	}
	sort.Strings(names)
	var missing []string
	for _, name := range names {
		if exists, err := a.d.Compose.VolumeExists(ctx, target, name); err == nil && !exists {
			missing = append(missing, name)
		}
	}
	return missing
}

func (a *App) confirmComposeVolumes(ctx context.Context, target ports.ComposeTarget, missing []string, created map[string]bool) {
	for _, name := range missing {
		if exists, err := a.d.Compose.VolumeExists(ctx, target, name); err == nil && exists {
			created[name] = true
		}
	}
}

func composeVolumeHints(p *domain.Project, env string, created map[string]bool) []string {
	if len(created) == 0 {
		return nil
	}
	var actions []string
	selection := " -p " + hintShellArg(p.Name)
	if env != p.DefaultEnv {
		selection += " --env " + hintShellArg(env)
	}
	if _, ok := p.Setup["migrate"]; ok {
		actions = append(actions, "rocket migrate"+selection)
	}
	if _, ok := p.Setup["assets"]; ok {
		actions = append(actions, "rocket setup assets"+selection)
	}
	if len(actions) == 0 {
		return nil
	}
	names := make([]string, 0, len(created))
	for name := range created {
		names = append(names, name)
	}
	sort.Strings(names)
	return []string{fmt.Sprintf("New Compose volumes (%s/%s): %s. Run %s when prerequisites are ready.", p.Name, env, strings.Join(names, ", "), strings.Join(actions, " and "))}
}

func hintShellArg(value string) string {
	if value != "" && strings.IndexFunc(value, func(r rune) bool {
		return !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || strings.ContainsRune("_./-", r))
	}) < 0 {
		return value
	}
	return "'" + strings.ReplaceAll(value, "'", `'"'"'`) + "'"
}
