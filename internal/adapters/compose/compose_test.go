package compose

import (
	"slices"
	"testing"

	"github.com/xean-io/rocket/internal/ports"
)

func TestArgsForceProjectNameFilesAndProfiles(t *testing.T) {
	target := ports.ComposeTarget{
		ProjectName: "rocket-nuvara-smoke",
		Files:       []string{"/code/nuvara/docker-compose.prod.yml"},
		Profiles:    []string{"release", "deps"},
	}
	got := Args(target, "up", "-d", "--wait", "postgres")
	want := []string{"compose", "-p", "rocket-nuvara-smoke", "-f", "/code/nuvara/docker-compose.prod.yml",
		"--profile", "release", "--profile", "deps", "up", "-d", "--wait", "postgres"}
	if !slices.Equal(got, want) {
		t.Fatalf("got  %v\nwant %v", got, want)
	}
}
