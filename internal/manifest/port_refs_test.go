package manifest

import (
	"slices"
	"strings"
	"testing"
)

func TestPortReferencesAddDependencies(t *testing.T) {
	p, err := Parse([]byte(`version: 1
name: refs
services:
  api: {run: api, ports: {http: {default: 8080, env: PORT}}}
  commerce: {run: commerce, ports: {http: {default: 9000, env: PORT}}}
  web:
    task: web
    depends_on: [commerce]
    env:
      API_URL: "http://127.0.0.1:{api.http}/{api.http}"
      COMMERCE_URL: "http://127.0.0.1:{commerce.http}"
`), "/code/refs")
	if err != nil {
		t.Fatal(err)
	}
	web := p.Services["web"]
	if !slices.Equal(web.DependsOn, []string{"commerce", "api"}) {
		t.Fatalf("dependencies must preserve explicit entries and add references once: %v", web.DependsOn)
	}
	order, err := p.StartOrder([]string{"web"})
	if err != nil || !slices.Equal(order, []string{"api", "commerce", "web"}) {
		t.Fatalf("start order = %v, %v", order, err)
	}
}

func TestPortReferenceValidation(t *testing.T) {
	for _, tt := range []struct {
		name     string
		services string
		want     string
	}{
		{"unknown service", `web: {run: web, env: {API_URL: "http://localhost:{missing.http}"}}`, `unknown service "missing"`},
		{"unknown port", `api: {run: api, ports: {http: {default: 8080}}}
  web: {run: web, env: {API_URL: "http://localhost:{api.admin}"}}`, `unknown port "admin"`},
		{"self reference", `api: {run: api, ports: {http: {default: 8080}}, env: {URL: "http://localhost:{api.http}"}}`, "depends on itself"},
		{"reference cycle", `api: {run: api, ports: {http: {default: 8080}}, env: {URL: "http://localhost:{web.http}"}}
  web: {run: web, ports: {http: {default: 3000}}, env: {URL: "http://localhost:{api.http}"}}`, "dependency cycle"},
		{"mixed explicit cycle", `api: {run: api, depends_on: [web], ports: {http: {default: 8080}}}
  web: {run: web, env: {URL: "http://localhost:{api.http}"}}`, "dependency cycle"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			_, err := Parse([]byte("version: 1\nname: refs\nservices:\n  "+tt.services+"\n"), "/code/refs")
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Fatalf("error = %v, want %q", err, tt.want)
			}
		})
	}
}

func TestPortReferencesSupportDottedServiceNamesAndLeaveShellExpressions(t *testing.T) {
	p, err := Parse([]byte(`version: 1
name: refs
services:
  api.backend: {run: api, ports: {http: {default: 8080}}}
  web: {run: web, env: {API_URL: "http://localhost:{api.backend.http}", SHELL: "${missing.http}"}}
`), "/code/refs")
	if err != nil {
		t.Fatal(err)
	}
	if got := p.Services["web"].DependsOn; !slices.Equal(got, []string{"api.backend"}) {
		t.Fatalf("dependencies = %v", got)
	}
}
