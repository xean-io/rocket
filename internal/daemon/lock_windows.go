//go:build windows

package daemon

// lockFile is a no-op on Windows until the daemon is supported there.
func lockFile(string) (func(), error) { return func() {}, nil }
