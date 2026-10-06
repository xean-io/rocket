// Package scaffold detects a project's Taskfile, compose files and .env and
// renders a starting rocket.yaml for `rocket init`. Detection only parses
// YAML; it never runs task or docker.
package scaffold

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"sort"
	"strconv"
	"strings"

	"gopkg.in/yaml.v3"

	"github.com/xean-io/rocket/internal/adapters/task"
)

// Port is a published compose port.
type Port struct {
	Name    string `json:"name"`
	Default int    `json:"default"`
	Env     string `json:"env,omitempty"` // from ${VAR:-default}; empty = not remappable
	Target  int    `json:"target"`        // container port
}

// Service is a guessed rocket service.
type Service struct {
	Name     string   `json:"name"`
	Compose  string   `json:"compose,omitempty"`
	Task     string   `json:"task,omitempty"`
	Profiles []string `json:"profiles,omitempty"`
	Ports    []Port   `json:"ports,omitempty"`
}

// Env is a guessed compose environment.
type Env struct {
	Name  string   `json:"name"`
	Files []string `json:"files"`
}

// SetupStep maps a setup action to a Taskfile task.
type SetupStep struct {
	Name string `json:"name"`
	Task string `json:"task"`
}

// Task is a Taskfile task (including included namespaces).
type Task struct {
	Name string `json:"name"`
	Desc string `json:"desc,omitempty"`
}

// Result is everything Detect found.
type Result struct {
	Name         string      `json:"name"`
	Root         string      `json:"root"`
	Dotenv       []string    `json:"dotenv,omitempty"`
	Taskfile     string      `json:"taskfile,omitempty"`
	ComposeFiles []string    `json:"compose_files,omitempty"`
	Envs         []Env       `json:"envs,omitempty"`
	Services     []Service   `json:"services"`
	Setup        []SetupStep `json:"setup,omitempty"`
	Pipelines    []string    `json:"pipelines,omitempty"`
	DeployTasks  []string    `json:"deploy_tasks,omitempty"` // left out of pipelines on purpose
	Tasks        []Task      `json:"tasks,omitempty"`
	Notes        []string    `json:"notes,omitempty"`
}

var (
	composeFileRe = regexp.MustCompile(`^(docker-)?compose(\.([A-Za-z0-9_-]+))?\.ya?ml$`)
	devComposeOrd = []string{"compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"}
	nameSanitizer = regexp.MustCompile(`[^a-z0-9._-]+`)
)

// Detect inspects root.
func Detect(root string) (*Result, error) {
	abs, err := filepath.Abs(root)
	if err != nil {
		return nil, err
	}
	r := &Result{Name: projectName(abs), Root: abs, Services: []Service{}}
	if st, err := os.Stat(filepath.Join(abs, ".env")); err == nil && !st.IsDir() {
		r.Dotenv = []string{".env"}
	}
	if err := r.detectCompose(abs); err != nil {
		return nil, err
	}
	if tf := (task.Driver{}).Taskfile(abs); tf != "" {
		r.Taskfile = filepath.Base(tf)
		tasks, err := readTaskfile(tf, "", 0)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", r.Taskfile, err)
		}
		r.classifyTasks(tasks)
	}
	return r, nil
}

func projectName(dir string) string {
	n := strings.Trim(nameSanitizer.ReplaceAllString(strings.ToLower(filepath.Base(dir)), "-"), "-._")
	if n == "" {
		return "project"
	}
	return n
}

type composeFile struct {
	Name     string `yaml:"name"`
	Services map[string]struct {
		Profiles []string    `yaml:"profiles"`
		Ports    []yaml.Node `yaml:"ports"`
	} `yaml:"services"`
}

func (r *Result) detectCompose(root string) error {
	entries, err := os.ReadDir(root)
	if err != nil {
		return err
	}
	var files []string
	for _, e := range entries {
		if !e.IsDir() && composeFileRe.MatchString(e.Name()) {
			files = append(files, e.Name())
		}
	}
	if len(files) == 0 {
		return nil
	}
	sort.Slice(files, func(i, j int) bool {
		return composeRank(files[i]) < composeRank(files[j]) || (composeRank(files[i]) == composeRank(files[j]) && files[i] < files[j])
	})
	r.ComposeFiles = files

	parsed := map[string]composeFile{}
	for _, f := range files {
		data, err := os.ReadFile(filepath.Join(root, f))
		if err != nil {
			return err
		}
		var cf composeFile
		if err := yaml.Unmarshal(data, &cf); err != nil {
			return fmt.Errorf("%s: %w", f, err)
		}
		parsed[f] = cf
	}

	// The first un-suffixed file is dev (plus override files); every other
	// file becomes its own env.
	dev := Env{Name: "dev"}
	var others []Env
	used := map[string]bool{"dev": true}
	smoke := 0
	for _, f := range files {
		suffix := composeFileRe.FindStringSubmatch(f)[3]
		switch {
		case suffix == "" && len(dev.Files) == 0:
			dev.Files = append(dev.Files, f)
		case suffix == "override":
			dev.Files = append(dev.Files, f)
		default:
			name := strings.ToLower(suffix)
			if name == "" {
				smoke++
				name = "smoke"
				if smoke > 1 {
					name = fmt.Sprintf("smoke%d", smoke)
				}
			}
			for used[name] {
				name += "-alt"
			}
			used[name] = true
			others = append(others, Env{Name: name, Files: []string{f}})
		}
	}
	if len(dev.Files) == 0 { // only suffixed files: promote the first
		dev.Files, others = others[0].Files, others[1:]
	}
	r.Envs = append([]Env{dev}, others...)

	devServices := map[string]bool{}
	for _, f := range dev.Files {
		cf := parsed[f]
		for _, name := range sortedKeys(cf.Services) {
			if devServices[name] {
				continue
			}
			devServices[name] = true
			s := cf.Services[name]
			svc := Service{Name: name, Compose: name, Profiles: s.Profiles}
			for _, node := range s.Ports {
				if p, ok := parsePortNode(node); ok {
					svc.Ports = append(svc.Ports, p)
				}
			}
			nameSvcPorts(svc.Ports)
			r.Services = append(r.Services, svc)
		}
	}
	for _, e := range others {
		var only []string
		for _, name := range sortedKeys(parsed[e.Files[0]].Services) {
			if !devServices[name] {
				only = append(only, name)
			}
		}
		if len(only) > 0 {
			r.Notes = append(r.Notes, fmt.Sprintf("services only in %s (env %s, not added): %s", e.Files[0], e.Name, strings.Join(only, ", ")))
		}
	}
	byName := map[string][]string{}
	for _, f := range files {
		if n := parsed[f].Name; n != "" {
			byName[n] = append(byName[n], f)
		}
	}
	for _, n := range sortedKeys(byName) {
		if len(byName[n]) > 1 {
			r.Notes = append(r.Notes, fmt.Sprintf("%s share the compose project name %q; rocket forces rocket-%s-<env> per env, so their volumes no longer collide",
				strings.Join(byName[n], " and "), n, r.Name))
		}
	}
	return nil
}

func composeRank(f string) int {
	if i := slices.Index(devComposeOrd, f); i >= 0 {
		return i
	}
	return len(devComposeOrd)
}

func nameSvcPorts(ports []Port) {
	for i := range ports {
		if len(ports) == 1 {
			ports[i].Name = "main"
		} else {
			ports[i].Name = fmt.Sprintf("p%d", ports[i].Target)
		}
	}
}

func sortedKeys[V any](m map[string]V) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

var (
	braceVarRe = regexp.MustCompile(`^\$\{([A-Za-z_][A-Za-z0-9_]*)(:?-([^}]*))?\}$`)
	plainVarRe = regexp.MustCompile(`^\$([A-Za-z_][A-Za-z0-9_]*)$`)
)

func parsePortNode(n yaml.Node) (Port, bool) {
	switch n.Kind {
	case yaml.ScalarNode:
		return parseShortPort(n.Value)
	case yaml.MappingNode:
		var long struct {
			Target    int    `yaml:"target"`
			Published string `yaml:"published"`
		}
		if err := n.Decode(&long); err != nil || long.Published == "" {
			return Port{}, false
		}
		return parsePublished(long.Published, long.Target)
	}
	return Port{}, false
}

// parseShortPort parses "[ip:]host:container[/proto]", where host may be
// ${VAR:-default}. Container-only, ranges and ephemeral host ports are skipped.
func parseShortPort(s string) (Port, bool) {
	s = strings.TrimSpace(s)
	if i := strings.LastIndexByte(s, '/'); i >= 0 && !strings.Contains(s[i:], "}") {
		s = s[:i]
	}
	parts := splitOutsideBraces(s)
	if len(parts) < 2 {
		return Port{}, false
	}
	target, err := strconv.Atoi(parts[len(parts)-1])
	if err != nil {
		return Port{}, false
	}
	return parsePublished(parts[len(parts)-2], target)
}

func parsePublished(host string, target int) (Port, bool) {
	host = strings.TrimSpace(host)
	if m := braceVarRe.FindStringSubmatch(host); m != nil {
		def := target
		if m[3] != "" {
			v, err := strconv.Atoi(m[3])
			if err != nil {
				return Port{}, false
			}
			def = v
		}
		return Port{Default: def, Env: m[1], Target: target}, def > 0
	}
	if m := plainVarRe.FindStringSubmatch(host); m != nil {
		return Port{Default: target, Env: m[1], Target: target}, target > 0
	}
	v, err := strconv.Atoi(host)
	if err != nil || v <= 0 {
		return Port{}, false
	}
	return Port{Default: v, Target: target}, true
}

func splitOutsideBraces(s string) []string {
	var parts []string
	depth, start := 0, 0
	for i, c := range s {
		switch c {
		case '{':
			depth++
		case '}':
			depth--
		case ':':
			if depth == 0 {
				parts = append(parts, s[start:i])
				start = i + 1
			}
		}
	}
	return append(parts, s[start:])
}

type taskfileYAML struct {
	Includes yaml.Node `yaml:"includes"`
	Tasks    yaml.Node `yaml:"tasks"`
}

// readTaskfile lists the tasks of a Taskfile and its includes (namespaced,
// up to three levels). Internal tasks/includes and templated paths are skipped.
func readTaskfile(path, prefix string, depth int) ([]Task, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var tf taskfileYAML
	if err := yaml.Unmarshal(data, &tf); err != nil {
		return nil, err
	}
	var out []Task
	for i := 0; i+1 < len(tf.Tasks.Content); i += 2 {
		name, body := tf.Tasks.Content[i].Value, tf.Tasks.Content[i+1]
		var meta struct {
			Desc     string `yaml:"desc"`
			Internal bool   `yaml:"internal"`
		}
		if body.Kind == yaml.MappingNode {
			_ = body.Decode(&meta)
		}
		if meta.Internal || name == "default" {
			continue
		}
		out = append(out, Task{Name: prefix + name, Desc: meta.Desc})
	}
	if depth >= 3 {
		return out, nil
	}
	for i := 0; i+1 < len(tf.Includes.Content); i += 2 {
		ns, body := tf.Includes.Content[i].Value, tf.Includes.Content[i+1]
		var inc struct {
			Taskfile string `yaml:"taskfile"`
			Internal bool   `yaml:"internal"`
		}
		if body.Kind == yaml.ScalarNode {
			inc.Taskfile = body.Value
		} else {
			_ = body.Decode(&inc)
		}
		if inc.Internal || inc.Taskfile == "" || strings.Contains(inc.Taskfile, "{{") {
			continue
		}
		p := inc.Taskfile
		if !filepath.IsAbs(p) {
			p = filepath.Join(filepath.Dir(path), p)
		}
		if st, err := os.Stat(p); err == nil && st.IsDir() {
			p = (task.Driver{}).Taskfile(p)
		}
		if p == "" {
			continue
		}
		sub, err := readTaskfile(p, prefix+ns+":", depth+1)
		if err != nil {
			continue // optional or missing include: ignore
		}
		out = append(out, sub...)
	}
	return out, nil
}

// IsLongRunning guesses whether a task is a dev server.
func IsLongRunning(name string) bool {
	return name == "dev" || name == "serve" || name == "start" || strings.HasPrefix(name, "dev:") || strings.HasSuffix(name, ":dev")
}

func isDeployish(name string) bool {
	for _, seg := range strings.Split(name, ":") {
		if strings.Contains(seg, "deploy") || strings.Contains(seg, "release") {
			return true
		}
	}
	return false
}

func (r *Result) classifyTasks(tasks []Task) {
	sort.Slice(tasks, func(i, j int) bool { return tasks[i].Name < tasks[j].Name })
	r.Tasks = tasks
	have := map[string]bool{}
	for _, t := range tasks {
		have[t.Name] = true
	}
	setupTask := map[string]string{}
	for _, s := range []SetupStep{{"doctor", "doctor"}, {"install", "install"}, {"install", "setup"}, {"migrate", "migrate"}, {"migrate", "db:migrate"}} {
		if have[s.Task] && setupTask[s.Name] == "" {
			setupTask[s.Name] = s.Task
			r.Setup = append(r.Setup, s)
		}
	}
	isSetup := map[string]bool{"setup": true} // reserved fallback even when explicit install takes precedence
	for _, s := range r.Setup {
		isSetup[s.Task] = true
	}
	taken := map[string]bool{}
	for _, s := range r.Services {
		taken[s.Name] = true
	}
	for _, t := range tasks {
		switch {
		case isSetup[t.Name]:
		case isDeployish(t.Name):
			r.DeployTasks = append(r.DeployTasks, t.Name)
		case IsLongRunning(t.Name):
			name := strings.ReplaceAll(t.Name, ":", "-")
			for taken[name] {
				name += "-task"
			}
			taken[name] = true
			r.Services = append(r.Services, Service{Name: name, Task: t.Name})
		default:
			r.Pipelines = append(r.Pipelines, t.Name)
		}
	}
}
