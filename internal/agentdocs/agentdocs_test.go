package agentdocs

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestUpsertBlockIsIdempotent(t *testing.T) {
	tests := []struct {
		name     string
		existing string
		changed  bool
	}{
		{"empty file", "", true},
		{"appends to existing content", "# Agents\n\nBe nice.\n", true},
		{"replaces an outdated block", "# A\n" + BeginMarker + "\nold\n" + EndMarker + "\ntail\n", true},
		{"unchanged when current", "# A\n\n" + Block(), false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, changed := UpsertBlock(tt.existing)
			if changed != tt.changed {
				t.Fatalf("changed=%v want %v\n%s", changed, tt.changed, got)
			}
			if strings.Count(got, BeginMarker) != 1 || strings.Count(got, EndMarker) != 1 {
				t.Fatalf("markers not unique:\n%s", got)
			}
			if again, changed := UpsertBlock(got); changed || again != got {
				t.Fatalf("second upsert changed the file:\n%s", again)
			}
			if strings.Contains(tt.existing, "tail\n") && !strings.HasSuffix(got, "tail\n") {
				t.Fatalf("content after the block lost:\n%s", got)
			}
			if strings.Contains(tt.existing, "Be nice.") && !strings.HasPrefix(got, "# Agents\n\nBe nice.\n") {
				t.Fatalf("existing content altered:\n%s", got)
			}
		})
	}
}

func TestContentCarriesTheRules(t *testing.T) {
	for name, text := range map[string]string{"skill": Skill(), "block": Block()} {
		for _, want := range []string{"ROCKET_OWNER=agent:", "--ttl", "rocket down --owner", "--json", "nohup", "docker compose up", "task dev", "rocket run"} {
			if !strings.Contains(text, want) {
				t.Errorf("%s lacks %q", name, want)
			}
		}
	}
	if !strings.HasPrefix(Skill(), "---\nname: rocket\ndescription: ") {
		t.Fatalf("skill frontmatter:\n%s", Skill()[:80])
	}
}

func TestInstallTargets(t *testing.T) {
	tests := []struct {
		name   string
		opts   Options
		writes []string // relative to project unless prefixed with ~/
		skips  []string
	}{
		{"both, project skill", Options{Target: TargetBoth},
			[]string{".claude/skills/rocket/SKILL.md", "AGENTS.md", "CLAUDE.md"}, []string{"~/.claude/skills/rocket/SKILL.md"}},
		{"claude, global skill", Options{Target: TargetClaude, Global: true},
			[]string{"~/.claude/skills/rocket/SKILL.md", "CLAUDE.md"}, []string{"AGENTS.md", ".claude/skills/rocket/SKILL.md"}},
		{"agents only", Options{Target: TargetAgents},
			[]string{"AGENTS.md"}, []string{"CLAUDE.md", ".claude/skills/rocket/SKILL.md"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			home, project := t.TempDir(), t.TempDir()
			tt.opts.Home, tt.opts.ProjectRoot = home, project
			resolve := func(p string) string {
				if rest, ok := strings.CutPrefix(p, "~/"); ok {
					return filepath.Join(home, rest)
				}
				return filepath.Join(project, p)
			}
			actions, err := Install(tt.opts)
			if err != nil {
				t.Fatal(err)
			}
			if len(actions) != len(tt.writes) {
				t.Fatalf("actions %+v", actions)
			}
			for _, w := range tt.writes {
				data, err := os.ReadFile(resolve(w))
				if err != nil {
					t.Fatalf("%s not written: %v", w, err)
				}
				if !strings.Contains(string(data), "ROCKET_OWNER=agent:") {
					t.Fatalf("%s content:\n%s", w, data)
				}
			}
			for _, s := range tt.skips {
				if _, err := os.Stat(resolve(s)); err == nil {
					t.Fatalf("%s must not be written", s)
				}
			}
			again, err := Install(tt.opts)
			if err != nil {
				t.Fatal(err)
			}
			for _, a := range again {
				if a.Action != "unchanged" {
					t.Fatalf("second install not idempotent: %+v", again)
				}
			}
		})
	}
	if _, err := Install(Options{Target: "vim", Home: t.TempDir(), ProjectRoot: t.TempDir()}); err == nil {
		t.Fatal("unknown target accepted")
	}
}
