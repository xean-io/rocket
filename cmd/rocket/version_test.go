package main

import (
	"runtime/debug"
	"testing"
)

func TestResolveVersion(t *testing.T) {
	info := func(v string) func() (*debug.BuildInfo, bool) {
		return func() (*debug.BuildInfo, bool) {
			return &debug.BuildInfo{Main: debug.Module{Version: v}}, true
		}
	}
	noInfo := func() (*debug.BuildInfo, bool) { return nil, false }

	cases := []struct {
		name    string
		ldflags string
		read    func() (*debug.BuildInfo, bool)
		want    string
	}{
		{"ldflags win (goreleaser)", "0.2.0", info("v0.2.0"), "0.2.0"},
		{"go install module version", devVersion, info("v0.1.0"), "0.1.0"},
		{"go install pseudo-version", devVersion, info("v0.1.1-0.20261006143000-df0b91fabcde"), "0.1.1-0.20261006143000-df0b91fabcde"},
		{"local build", devVersion, info("(devel)"), devVersion},
		{"empty build info version", devVersion, info(""), devVersion},
		{"no build info", devVersion, noInfo, devVersion},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := resolveVersion(c.ldflags, c.read); got != c.want {
				t.Errorf("resolveVersion(%q) = %q, want %q", c.ldflags, got, c.want)
			}
		})
	}
}
