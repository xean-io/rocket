package main

import (
	"runtime/debug"
	"strings"
)

// devVersion is the placeholder for builds that carry no release version.
const devVersion = "dev"

// version is set by GoReleaser via -ldflags "-X main.version=...". It must stay
// initialized to a constant for -X to apply; main() resolves the fallback.
var version = devVersion

// resolveVersion prefers the ldflags value, then the module version recorded
// by `go install module@version`, then devVersion.
func resolveVersion(ldflags string, read func() (*debug.BuildInfo, bool)) string {
	if ldflags != devVersion {
		return ldflags
	}
	info, ok := read()
	if !ok || info == nil {
		return devVersion
	}
	v := info.Main.Version
	if v == "" || v == "(devel)" {
		return devVersion
	}
	return strings.TrimPrefix(v, "v")
}
