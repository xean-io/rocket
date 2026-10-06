package scaffold

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
	"github.com/xean-io/rocket/internal/manifest"
)

func TestParsePortMapping(t *testing.T) {
	tests := []struct {
		in      string
		ok      bool
		def     int
		env     string
		target  int
		comment string
	}{
		{in: "${PORT:-3000}:3000", ok: true, def: 3000, env: "PORT", target: 3000},
		{in: "127.0.0.1:${PG:-5435}:5432", ok: true, def: 5435, env: "PG", target: 5432},
		{in: "${WEB-8081}:80/tcp", ok: true, def: 8081, env: "WEB", target: 80},
		{in: "${API_PORT}:8080", ok: true, def: 8080, env: "API_PORT", target: 8080},
		{in: "$API_PORT:8080", ok: true, def: 8080, env: "API_PORT", target: 8080},
		{in: "6379:6379", ok: true, def: 6379, target: 6379},
		{in: "8080", ok: false, comment: "container-only port is not published"},
		{in: "3000-3005:3000-3005", ok: false, comment: "ranges are skipped"},
		{in: "127.0.0.1::5432", ok: false, comment: "ephemeral host port"},
	}
	for _, tt := range tests {
		t.Run(tt.in, func(t *testing.T) {
			p, ok := parseShortPort(tt.in)
			if ok != tt.ok {
				t.Fatalf("ok=%v want %v (%s)", ok, tt.ok, tt.comment)
			}
			if ok && (p.Default != tt.def || p.Env != tt.env || p.Target != tt.target) {
				t.Fatalf("got %+v", p)
			}
		})
	}
}

func fixtureCopy(t *testing.T) string {
	t.Helper()
	dst := filepath.Join(t.TempDir(), "autodropshipping")
	src := filepath.Join("..", "..", "testdata", "init", "autodropshipping")
	err := filepath.WalkDir(src, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(src, path)
		target := filepath.Join(dst, rel)
		if d.IsDir() {
			return os.MkdirAll(target, 0o755)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		return os.WriteFile(target, data, 0o644)
	})
	if err != nil {
		t.Fatal(err)
	}
	return dst
}

func TestDetectAutodropshippingShape(t *testing.T) {
	root := fixtureCopy(t)
	r, err := Detect(root)
	if err != nil {
		t.Fatal(err)
	}
	if r.Name != "autodropshipping" || !slices.Equal(r.Dotenv, []string{".env"}) || r.Taskfile != "Taskfile.yml" {
		t.Fatalf("basics %+v", r)
	}
	if len(r.Envs) != 2 || r.Envs[0].Name != "dev" || r.Envs[0].Files[0] != "compose.yaml" ||
		r.Envs[1].Name != "smoke" || r.Envs[1].Files[0] != "docker-compose.yaml" {
		t.Fatalf("envs %+v", r.Envs)
	}
	svc := map[string]Service{}
	for _, s := range r.Services {
		svc[s.Name] = s
	}
	if pg := svc["postgres"]; pg.Compose != "postgres" || !slices.Equal(pg.Profiles, []string{"deps"}) ||
		len(pg.Ports) != 1 || pg.Ports[0] != (Port{Name: "main", Default: 5432, Env: "POSTGRES_PORT", Target: 5432}) {
		t.Fatalf("postgres %+v", pg)
	}
	if rd := svc["redis"]; len(rd.Ports) != 1 || rd.Ports[0].Env != "" || rd.Ports[0].Default != 6379 {
		t.Fatalf("redis %+v", rd)
	}
	if mn := svc["minio"]; len(mn.Ports) != 2 || mn.Ports[0].Env != "MINIO_PORT" || mn.Ports[1].Env != "MINIO_CONSOLE_PORT" {
		t.Fatalf("minio %+v", mn)
	}
	if w := svc["worker"]; w.Compose != "worker" || len(w.Ports) != 0 {
		t.Fatalf("worker %+v", w)
	}
	if svc["dev"].Task != "dev" || svc["cp-dev"].Task != "cp:dev" {
		t.Fatalf("task services dev=%+v cp-dev=%+v", svc["dev"], svc["cp-dev"])
	}
	if _, ok := svc["control-plane"]; ok {
		t.Fatal("prod-only compose service must not become a dev service")
	}
	if want := []string{"cp:test", "infra:down", "infra:up", "lint", "test"}; !slices.Equal(r.Pipelines, want) {
		t.Fatalf("pipelines %v, want %v", r.Pipelines, want)
	}
	if want := []string{"deploy:prod", "release:ci"}; !slices.Equal(r.DeployTasks, want) {
		t.Fatalf("deploy tasks %v", r.DeployTasks)
	}
	if len(r.Setup) != 2 || r.Setup[0] != (SetupStep{Name: "install", Task: "install"}) || r.Setup[1] != (SetupStep{Name: "migrate", Task: "db:migrate"}) {
		t.Fatalf("setup %+v", r.Setup)
	}
	var names []string
	for _, tk := range r.Tasks {
		names = append(names, tk.Name)
	}
	for _, hidden := range []string{"_helper", "default"} {
		if slices.Contains(names, hidden) {
			t.Fatalf("tasks list contains %s: %v", hidden, names)
		}
	}
	if !slices.Contains(names, "cp:dev") || !slices.Contains(names, "infra:up") {
		t.Fatalf("included tasks missing: %v", names)
	}
}

func TestRenderIsAValidManifest(t *testing.T) {
	root := fixtureCopy(t)
	r, err := Detect(root)
	if err != nil {
		t.Fatal(err)
	}
	out := r.Render()
	p, err := manifest.Parse(out, root)
	if err != nil {
		t.Fatalf("generated rocket.yaml is invalid: %v\n%s", err, out)
	}
	if p.Services["postgres"].Kind != domain.KindCompose || p.Services["cp-dev"].Task != "cp:dev" {
		t.Fatalf("services %+v", p.Services)
	}
	if p.Pipelines["cp:test"][0].Task != "cp:test" || p.Setup["migrate"].Task != "db:migrate" {
		t.Fatalf("pipelines %+v setup %+v", p.Pipelines, p.Setup)
	}
	if got := p.Envs["smoke"].Compose; !slices.Equal(got, []string{"docker-compose.yaml"}) {
		t.Fatalf("smoke env %v", got)
	}
	text := string(out)
	for _, want := range []string{
		"#   - deploy:prod",                // deploy-looking tasks only as a hint
		"#   - cp:dev — Control plane dev", // task hint block
		"control-plane",                    // prod-only service mentioned in a comment
		"xean-spectrai",                    // shared compose project name flagged
	} {
		if !strings.Contains(text, want) {
			t.Errorf("output lacks %q:\n%s", want, text)
		}
	}
	if _, ok := p.Pipelines["deploy:prod"]; ok {
		t.Fatal("deploy task must not become a pipeline")
	}
}

func TestRenderWithoutDetectionsStillValid(t *testing.T) {
	root := filepath.Join(t.TempDir(), "Empty Project")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatal(err)
	}
	r, err := Detect(root)
	if err != nil {
		t.Fatal(err)
	}
	if r.Name != "empty-project" {
		t.Fatalf("name %q", r.Name)
	}
	if _, err := manifest.Parse(r.Render(), root); err != nil {
		t.Fatalf("placeholder manifest invalid: %v\n%s", err, r.Render())
	}
}
