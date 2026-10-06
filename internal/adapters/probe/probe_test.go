package probe

import "testing"

func TestParseLsof(t *testing.T) {
	out := "p4242\ncnode\nn*:3000\np4243\ncother\n"
	h := ParseLsof(out)
	if h == nil || h.PID != 4242 || h.Command != "node" {
		t.Fatalf("got %+v", h)
	}
	if ParseLsof("") != nil {
		t.Fatal("empty output should yield nil")
	}
	if parseCwd("p1\nfcwd\nn/Users/me/code\n") != "/Users/me/code" {
		t.Fatal("cwd not parsed")
	}
}
