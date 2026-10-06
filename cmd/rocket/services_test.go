package main

import (
	"bytes"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/app"
)

func TestWriteUpPrintsHints(t *testing.T) {
	var out bytes.Buffer
	writeUp(&out, app.UpResult{Project: "fixture", Env: "dev", Hints: []string{"Run rocket migrate.", "Run rocket setup assets."}})
	for _, want := range []string{"project fixture (env dev)", "hint: Run rocket migrate.", "hint: Run rocket setup assets."} {
		if !strings.Contains(out.String(), want) {
			t.Errorf("human output missing %q: %s", want, out.String())
		}
	}
}

func TestDownAllOutsideProjectGuidance(t *testing.T) {
	dir := t.TempDir()
	t.Chdir(dir)
	for _, explicit := range []bool{false, true} {
		t.Run(map[bool]string{false: "outside project", true: "explicit bad path"}[explicit], func(t *testing.T) {
			g := &globals{}
			if explicit {
				g.project = dir + "/missing"
			}
			cmd := downCmd(g)
			if err := cmd.Flags().Set("all", "true"); err != nil {
				t.Fatal(err)
			}
			err := cmd.RunE(cmd, nil)
			if err == nil || !strings.Contains(err.Error(), "no rocket.yaml found") {
				t.Fatalf("outside project error changed: %v", err)
			}
			if strings.Contains(err.Error(), "--everywhere") == explicit {
				t.Fatalf("global stop guidance explicit=%v: %v", explicit, err)
			}
		})
	}
}
