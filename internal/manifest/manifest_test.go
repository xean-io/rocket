package manifest

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/xean-io/rocket/internal/domain"
)

func loadFixture(t *testing.T, name string) (*domain.Project, error) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("..", "..", "testdata", "manifests", name))
	if err != nil {
		t.Fatal(err)
	}
	return Parse(data, "/projects/"+strings.TrimSuffix(name, ".yaml"))
}

func TestParseNuvara(t *testing.T) {
	p, err := loadFixture(t, "nuvara.yaml")
	if err != nil {
		t.Fatal(err)
	}
	if p.Name != "nuvara" || p.Root != "/projects/nuvara" || p.DefaultEnv != "dev" {
		t.Fatalf("unexpected project header: %+v", p)
	}
	api := p.Services["api"]
	if api.Kind != domain.KindRun || api.Cwd != "apps/api" {
		t.Fatalf("api kind/cwd: %+v", api)
	}
	if !reflect.DeepEqual(api.Ports, []domain.PortSpec{{Name: "http", Default: 3002, Env: "PORT"}}) {
		t.Fatalf("api ports: %+v", api.Ports)
	}
	if api.Health == nil || api.Health.HTTP != "/api/health/live" || api.Health.Timeout != 90*time.Second {
		t.Fatalf("api health: %+v", api.Health)
	}
	if p.Services["postgres"].Kind != domain.KindCompose || p.Services["causation"].Kind != domain.KindTask {
		t.Fatal("service kinds not resolved")
	}
	if got := p.Envs["smoke"].Profiles; !reflect.DeepEqual(got, []string{"release"}) {
		t.Fatalf("smoke profiles %v", got)
	}
	if d := p.Envs["prod"].Deploy; d == nil || d.Task != "release:ci" || !d.Confirm {
		t.Fatalf("prod deploy %+v", d)
	}
	if len(p.Pipelines["ci"]) != 3 || p.Setup["migrate"].Task != "db:migrate" {
		t.Fatal("pipelines/setup not parsed")
	}
}

func TestParseAutodropshipping(t *testing.T) {
	p, err := loadFixture(t, "autodropshipping.yaml")
	if err != nil {
		t.Fatal(err)
	}
	cp := p.Services["control-plane"]
	if cp.Env["NODE_ENV"] != "development" || cp.Ports[0].Default != 3000 {
		t.Fatalf("control-plane: %+v", cp)
	}
	if got := p.Envs["full"].Compose; !reflect.DeepEqual(got, []string{"compose.yaml", "docker-compose.yaml"}) {
		t.Fatalf("full compose %v", got)
	}
}

func TestValidationReportsAllProblems(t *testing.T) {
	_, err := loadFixture(t, "invalid.yaml")
	if err == nil {
		t.Fatal("expected validation error")
	}
	msg := err.Error()
	for _, want := range []string{
		`service "a": exactly one of compose, task or run is required (got task, run)`,
		`service "b": exactly one of compose, task or run is required (got none)`,
		`service "a": depends_on references unknown service "ghost"`,
		`group "g": unknown service "missing"`,
		`group "a": name collides with a service`,
		`port 70000 out of range`,
		`invalid env var name "bad-name"`,
		`health.port "nope" is not a declared port`,
		`dependency cycle`,
	} {
		if !strings.Contains(msg, want) {
			t.Errorf("missing problem %q in:\n%s", want, msg)
		}
	}
}

func TestUnknownFieldsRejected(t *testing.T) {
	_, err := loadFixture(t, "unknown_field.yaml")
	if err == nil || !strings.Contains(err.Error(), "dependson") {
		t.Fatalf("err = %v, want unknown field error", err)
	}
}

func TestValidationCases(t *testing.T) {
	tests := []struct {
		name    string
		yaml    string
		wantErr string
	}{
		{name: "missing version", yaml: "name: x\nservices: {a: {run: x}}", wantErr: "version must be 1"},
		{name: "bad name", yaml: "version: 1\nname: 'Bad Name'\nservices: {a: {run: x}}", wantErr: "invalid project name"},
		{name: "self dependency", yaml: "version: 1\nname: x\nservices: {a: {run: x, depends_on: [a]}}", wantErr: "depends on itself"},
		{name: "unknown default env", yaml: "version: 1\nname: x\ndefault_env: qa\nenvs: {dev: {}}\nservices: {a: {run: x}}", wantErr: `default_env "qa"`},
		{name: "health path", yaml: "version: 1\nname: x\nservices: {a: {run: x, ports: {h: {default: 1}}, health: {http: health}}}", wantErr: "must start with /"},
		{name: "health without ports", yaml: "version: 1\nname: x\nservices: {a: {run: x, health: {tcp: true}}}", wantErr: "health requires at least one port"},
		{name: "bad timeout", yaml: "version: 1\nname: x\nservices: {a: {run: x, ports: {h: {default: 1}}, health: {timeout: soon}}}", wantErr: "invalid health.timeout"},
		{name: "pipeline step", yaml: "version: 1\nname: x\nservices: {a: {run: x}}\npipelines: {t: [{}]}", wantErr: `pipeline "t" step 1`},
		{name: "no services", yaml: "version: 1\nname: x", wantErr: "no services"},
		{name: "deploy without action", yaml: "version: 1\nname: x\nenvs: {dev: {}, prod: {deploy: {confirm: true}}}\nservices: {a: {run: x}}", wantErr: `env "prod": deploy needs exactly one of task or run`},
		{name: "deploy with both", yaml: "version: 1\nname: x\nenvs: {dev: {}, prod: {deploy: {task: a, run: b}}}\nservices: {a: {run: x}}", wantErr: `env "prod": deploy needs exactly one`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Parse([]byte(tt.yaml), "/tmp/x")
			if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
				t.Fatalf("err = %v, want %q", err, tt.wantErr)
			}
		})
	}
}

func TestNameDefaultsToDirectory(t *testing.T) {
	p, err := Parse([]byte("version: 1\nservices: {a: {run: x}}"), "/code/my-app")
	if err != nil {
		t.Fatal(err)
	}
	if p.Name != "my-app" || p.DefaultEnv != "dev" {
		t.Fatalf("got name %q env %q", p.Name, p.DefaultEnv)
	}
	if _, ok := p.Envs["dev"]; !ok {
		t.Fatal("implicit dev env missing")
	}
}

func TestFindWalksUp(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, FileName), []byte("version: 1\nservices: {a: {run: x}}"), 0o644); err != nil {
		t.Fatal(err)
	}
	deep := filepath.Join(root, "a", "b")
	if err := os.MkdirAll(deep, 0o755); err != nil {
		t.Fatal(err)
	}
	got, err := Find(deep)
	if err != nil {
		t.Fatal(err)
	}
	want, _ := filepath.EvalSymlinks(root)
	gotReal, _ := filepath.EvalSymlinks(got)
	if gotReal != want {
		t.Fatalf("got %s want %s", got, root)
	}
	if _, err := Find(t.TempDir()); err == nil {
		t.Fatal("expected not found")
	}
}

func TestSchemaCoversManifestFields(t *testing.T) {
	raw := Schema()
	var schema map[string]any
	if err := json.Unmarshal(raw, &schema); err != nil {
		t.Fatalf("schema is not valid JSON: %v", err)
	}
	props := schema["properties"].(map[string]any)
	for _, f := range yamlFields(reflect.TypeOf(fileYAML{})) {
		if _, ok := props[f]; !ok {
			t.Errorf("schema missing top-level property %q", f)
		}
	}
	svc := schema["$defs"].(map[string]any)["service"].(map[string]any)["properties"].(map[string]any)
	for _, f := range yamlFields(reflect.TypeOf(serviceYAML{})) {
		if _, ok := svc[f]; !ok {
			t.Errorf("schema missing service property %q", f)
		}
	}
}

func yamlFields(t reflect.Type) []string {
	var out []string
	for i := 0; i < t.NumField(); i++ {
		tag := strings.Split(t.Field(i).Tag.Get("yaml"), ",")[0]
		if tag != "" && tag != "-" {
			out = append(out, tag)
		}
	}
	return out
}

func TestParseDotenv(t *testing.T) {
	src := "# comment\nexport A=1\nB = \"two words\"\nC='x#y'\nD=val # trailing\n\nEMPTY=\nbad line\n"
	got := ParseDotenv([]byte(src))
	want := map[string]string{"A": "1", "B": "two words", "C": "x#y", "D": "val", "EMPTY": ""}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v want %v", got, want)
	}
}
