// Package manifest loads, validates and describes rocket.yaml.
package manifest

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"

	"gopkg.in/yaml.v3"

	"github.com/xean-io/rocket/internal/domain"
)

// FileName is the manifest file looked up in project roots.
const FileName = "rocket.yaml"

// ErrNoManifest identifies discovery outside any project without changing its message.
var ErrNoManifest = errors.New("no " + FileName + " found")

var altFileNames = []string{FileName, "rocket.yml"}

type fileYAML struct {
	Version    int                     `yaml:"version"`
	Name       string                  `yaml:"name"`
	Dotenv     []string                `yaml:"dotenv"`
	DefaultEnv string                  `yaml:"default_env"`
	Setup      map[string]stepYAML     `yaml:"setup"`
	Envs       map[string]envYAML      `yaml:"envs"`
	Services   map[string]serviceYAML  `yaml:"services"`
	Groups     map[string][]string     `yaml:"groups"`
	Pipelines  map[string]pipelineYAML `yaml:"pipelines"`
}

type serviceYAML struct {
	Compose   string              `yaml:"compose"`
	Task      string              `yaml:"task"`
	Run       string              `yaml:"run"`
	Cwd       string              `yaml:"cwd"`
	Env       map[string]string   `yaml:"env"`
	Dotenv    []string            `yaml:"dotenv"`
	Profiles  []string            `yaml:"profiles"`
	DependsOn []string            `yaml:"depends_on"`
	Ports     map[string]portYAML `yaml:"ports"`
	Health    *healthYAML         `yaml:"health"`
}

type portYAML struct {
	Default int         `yaml:"default"`
	Env     portEnvYAML `yaml:"env"`
	Probe   yaml.Node   `yaml:"probe"` // retain explicit null to distinguish it from omission
}

type portEnvYAML struct {
	Scalar   string
	Bindings map[string]domain.PortEnvBinding
}

// UnmarshalYAML accepts the legacy variable name or a variable-to-template map.
// The nested default objects are checked explicitly: yaml.Node.Decode does not
// inherit the outer decoder's KnownFields setting.
func (e *portEnvYAML) UnmarshalYAML(node *yaml.Node) error {
	if node.Kind == yaml.ScalarNode && node.Tag == "!!str" {
		e.Scalar = node.Value
		return nil
	}
	if node.Kind != yaml.MappingNode {
		return fmt.Errorf("line %d: port env must be a string or mapping", node.Line)
	}
	if len(node.Content) == 0 {
		return fmt.Errorf("line %d: port env mapping must not be empty", node.Line)
	}
	e.Bindings = map[string]domain.PortEnvBinding{}
	for i := 0; i < len(node.Content); i += 2 {
		key, value := node.Content[i], node.Content[i+1]
		if key.Kind != yaml.ScalarNode || key.Tag != "!!str" {
			return fmt.Errorf("line %d: port env variable name must be a string", key.Line)
		}
		if _, exists := e.Bindings[key.Value]; exists {
			return fmt.Errorf("line %d: port env variable %q already defined", key.Line, key.Value)
		}
		binding := domain.PortEnvBinding{}
		if value.Kind == yaml.MappingNode {
			if len(value.Content) != 2 || value.Content[0].Value != "default" {
				fields := []string{}
				for j := 0; j < len(value.Content); j += 2 {
					fields = append(fields, value.Content[j].Value)
				}
				return fmt.Errorf("line %d: port env binding %q requires only the default field (got %s)", value.Line, key.Value, strings.Join(fields, ", "))
			}
			binding.Default = true
			value = value.Content[1]
		}
		if value.Kind != yaml.ScalarNode || value.Tag != "!!str" {
			return fmt.Errorf("line %d: port env binding %q template must be a string", value.Line, key.Value)
		}
		binding.Template = value.Value
		e.Bindings[key.Value] = binding
	}
	return nil
}

type healthYAML struct {
	HTTP    string `yaml:"http"`
	TCP     bool   `yaml:"tcp"`
	Port    string `yaml:"port"`
	Timeout string `yaml:"timeout"`
}

type envYAML struct {
	Compose  []string    `yaml:"compose"`
	Profiles []string    `yaml:"profiles"`
	Deploy   *deployYAML `yaml:"deploy"`
}

type deployYAML struct {
	Task    string `yaml:"task"`
	Run     string `yaml:"run"`
	Confirm bool   `yaml:"confirm"`
}

type stepYAML struct {
	Task string `yaml:"task"`
	Run  string `yaml:"run"`
}

type pipelineYAML struct {
	Needs []string   `yaml:"needs"`
	Steps []stepYAML `yaml:"steps"`
}

func (p *pipelineYAML) UnmarshalYAML(node *yaml.Node) error {
	var steps *yaml.Node
	switch node.Kind {
	case yaml.SequenceNode:
		steps = node
	case yaml.MappingNode:
		for i := 0; i < len(node.Content); i += 2 {
			key := node.Content[i]
			switch key.Value {
			case "needs":
				needs := node.Content[i+1]
				for needs.Kind == yaml.AliasNode {
					needs = needs.Alias
				}
				if needs.Kind != yaml.SequenceNode {
					return fmt.Errorf("line %d: pipeline needs must be an array", needs.Line)
				}
			case "steps":
				steps = node.Content[i+1]
			default:
				return fmt.Errorf("line %d: unknown pipeline field %q", key.Line, key.Value)
			}
		}
		if steps == nil {
			return fmt.Errorf("line %d: pipeline object requires steps", node.Line)
		}
	default:
		return fmt.Errorf("line %d: pipeline must be an array or object", node.Line)
	}
	for steps.Kind == yaml.AliasNode {
		steps = steps.Alias
	}
	if steps.Kind != yaml.SequenceNode {
		return fmt.Errorf("line %d: pipeline steps must be an array", steps.Line)
	}
	// Custom node decoding must retain strict field checks for each nested step.
	for _, step := range steps.Content {
		for step.Kind == yaml.AliasNode {
			step = step.Alias
		}
		if step.Kind == yaml.MappingNode {
			for i := 0; i < len(step.Content); i += 2 {
				key := step.Content[i]
				if key.Value != "run" && key.Value != "task" {
					return fmt.Errorf("line %d: unknown pipeline step field %q", key.Line, key.Value)
				}
			}
		}
	}
	if node.Kind == yaml.SequenceNode {
		return node.Decode(&p.Steps)
	}
	type plain pipelineYAML
	return node.Decode((*plain)(p))
}

// ValidationError lists every problem found in a manifest.
type ValidationError struct {
	Problems []string
}

func (e *ValidationError) Error() string {
	return "invalid rocket.yaml:\n  - " + strings.Join(e.Problems, "\n  - ")
}

var (
	projectNameRe = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]*$`)
	envVarRe      = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)
)

// Find walks up from start looking for a rocket.yaml and returns its directory.
func Find(start string) (string, error) {
	dir, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	for {
		for _, n := range altFileNames {
			if st, err := os.Stat(filepath.Join(dir, n)); err == nil && !st.IsDir() {
				return dir, nil
			}
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("%w in %s or any parent directory", ErrNoManifest, start)
		}
		dir = parent
	}
}

// Loader adapts Load to ports.ManifestLoader.
type Loader struct{}

// Load implements ports.ManifestLoader.
func (Loader) Load(dir string) (*domain.Project, error) { return Load(dir) }

// Load reads and validates the manifest located in dir.
func Load(dir string) (*domain.Project, error) {
	abs, err := filepath.Abs(dir)
	if err != nil {
		return nil, err
	}
	for _, n := range altFileNames {
		data, err := os.ReadFile(filepath.Join(abs, n))
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return nil, err
		}
		p, err := Parse(data, abs)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", filepath.Join(abs, n), err)
		}
		return p, nil
	}
	return nil, fmt.Errorf("no %s in %s", FileName, abs)
}

// Parse decodes and validates manifest bytes; root is the project directory.
func Parse(data []byte, root string) (*domain.Project, error) {
	var f fileYAML
	dec := yaml.NewDecoder(bytes.NewReader(data))
	dec.KnownFields(true)
	if err := dec.Decode(&f); err != nil {
		return nil, fmt.Errorf("parse rocket.yaml: %w", err)
	}
	p, problems := convert(f, root)
	problems = append(problems, validate(p)...)
	if len(problems) > 0 {
		return nil, &ValidationError{Problems: problems}
	}
	return p, nil
}

func convert(f fileYAML, root string) (*domain.Project, []string) {
	var problems []string
	if f.Version != 1 {
		problems = append(problems, fmt.Sprintf("version must be 1 (got %d)", f.Version))
	}
	p := &domain.Project{
		Name:          f.Name,
		Root:          root,
		Dotenv:        f.Dotenv,
		DefaultEnv:    f.DefaultEnv,
		Setup:         map[string]domain.Step{},
		Envs:          map[string]domain.Environment{},
		Services:      map[string]domain.Service{},
		Groups:        f.Groups,
		Pipelines:     map[string][]domain.Step{},
		PipelineNeeds: map[string][]string{},
	}
	if p.Name == "" {
		p.Name = strings.ToLower(filepath.Base(root))
	}
	for name, s := range f.Setup {
		p.Setup[name] = domain.Step(s)
	}
	for name, pipeline := range f.Pipelines {
		p.Pipelines[name] = []domain.Step{}
		p.PipelineNeeds[name] = pipeline.Needs
		for _, s := range pipeline.Steps {
			p.Pipelines[name] = append(p.Pipelines[name], domain.Step(s))
		}
	}
	for name, e := range f.Envs {
		env := domain.Environment{Name: name, Compose: e.Compose, Profiles: e.Profiles}
		if e.Deploy != nil {
			env.Deploy = &domain.Deploy{Task: e.Deploy.Task, Run: e.Deploy.Run, Confirm: e.Deploy.Confirm}
			if (e.Deploy.Task == "") == (e.Deploy.Run == "") {
				problems = append(problems, fmt.Sprintf("env %q: deploy needs exactly one of task or run", name))
			}
		}
		p.Envs[name] = env
	}
	if len(p.Envs) == 0 {
		p.Envs["dev"] = domain.Environment{Name: "dev"}
	}
	if p.DefaultEnv == "" {
		p.DefaultEnv = "dev"
		if _, ok := p.Envs["dev"]; !ok {
			names := make([]string, 0, len(p.Envs))
			for n := range p.Envs {
				names = append(names, n)
			}
			sort.Strings(names)
			p.DefaultEnv = names[0]
		}
	}
	for name, s := range f.Services {
		svc := domain.Service{
			Name: name, Compose: s.Compose, Task: s.Task, Run: s.Run, Cwd: s.Cwd,
			Env: s.Env, Dotenv: s.Dotenv, Profiles: s.Profiles, DependsOn: s.DependsOn,
		}
		var kinds []string
		if s.Compose != "" {
			kinds = append(kinds, "compose")
			svc.Kind = domain.KindCompose
		}
		if s.Task != "" {
			kinds = append(kinds, "task")
			svc.Kind = domain.KindTask
		}
		if s.Run != "" {
			kinds = append(kinds, "run")
			svc.Kind = domain.KindRun
		}
		if len(kinds) != 1 {
			got := "none"
			if len(kinds) > 0 {
				got = strings.Join(kinds, ", ")
			}
			problems = append(problems, fmt.Sprintf("service %q: exactly one of compose, task or run is required (got %s)", name, got))
		}
		for pname, port := range s.Ports {
			spec := domain.PortSpec{Name: pname, Default: port.Default, Env: port.Env.Scalar, EnvBindings: port.Env.Bindings}
			probe := &port.Probe
			for probe.Kind == yaml.AliasNode {
				probe = probe.Alias
			}
			if probe.Kind != 0 {
				if probe.Kind != yaml.ScalarNode || probe.Tag != "!!bool" {
					problems = append(problems, fmt.Sprintf("service %q: port %q: probe must be a boolean", name, pname))
				} else {
					var enabled bool
					if err := probe.Decode(&enabled); err != nil {
						problems = append(problems, fmt.Sprintf("service %q: port %q: probe must be a boolean: %v", name, pname, err))
					} else {
						spec.Probe = &enabled
					}
				}
			}
			// Keep remaps.env and conflicts.env scalar and deterministic. A default
			// binding cannot authorize remapping because its override may be retained.
			for variable, binding := range port.Env.Bindings {
				if !binding.Default && (spec.Env == "" || variable < spec.Env) {
					spec.Env = variable
				}
			}
			svc.Ports = append(svc.Ports, spec)
		}
		sort.Slice(svc.Ports, func(i, j int) bool { return svc.Ports[i].Name < svc.Ports[j].Name })
		if s.Health != nil {
			h := &domain.HealthSpec{HTTP: s.Health.HTTP, TCP: s.Health.TCP, Port: s.Health.Port}
			if s.Health.Timeout != "" {
				d, err := time.ParseDuration(s.Health.Timeout)
				if err != nil || d <= 0 {
					problems = append(problems, fmt.Sprintf("service %q: invalid health.timeout %q", name, s.Health.Timeout))
				}
				h.Timeout = d
			}
			svc.Health = h
		}
		p.Services[name] = svc
	}
	for name, svc := range p.Services {
		seen := map[string]bool{}
		for _, dependency := range svc.DependsOn {
			seen[dependency] = true
		}
		for _, ref := range svc.PortReferences() {
			if !seen[ref.Service] {
				svc.DependsOn = append(svc.DependsOn, ref.Service)
				seen[ref.Service] = true
			}
		}
		p.Services[name] = svc
	}
	return p, problems
}

func validate(p *domain.Project) []string {
	var problems []string
	add := func(format string, args ...any) { problems = append(problems, fmt.Sprintf(format, args...)) }

	if !projectNameRe.MatchString(p.Name) {
		add("invalid project name %q (use lowercase letters, digits, '.', '_' or '-')", p.Name)
	}
	if len(p.Services) == 0 {
		add("no services declared")
	}
	if _, ok := p.Envs[p.DefaultEnv]; !ok {
		add("default_env %q is not a declared env", p.DefaultEnv)
	}
	for _, name := range p.ServiceNames() {
		s := p.Services[name]
		for _, ref := range s.PortReferences() {
			provider, ok := p.Services[ref.Service]
			if !ok {
				add("service %q: env reference %s: unknown service %q", name, ref.Token, ref.Service)
				continue
			}
			found := false
			for _, port := range provider.Ports {
				if port.Name == ref.Port {
					found = true
					break
				}
			}
			if !found {
				add("service %q: env reference %s: unknown port %q in service %q", name, ref.Token, ref.Port, ref.Service)
			}
		}
		for _, d := range s.DependsOn {
			if d == name {
				add("service %q depends on itself", name)
			} else if _, ok := p.Services[d]; !ok {
				add("service %q: depends_on references unknown service %q", name, d)
			}
		}
		portSpecs := map[string]domain.PortSpec{}
		for _, port := range s.Ports {
			portSpecs[port.Name] = port
			if port.Default < 1 || port.Default > 65535 {
				add("service %q: port %q: port %d out of range", name, port.Name, port.Default)
			}
			if port.Env != "" && !envVarRe.MatchString(port.Env) {
				add("service %q: port %q: invalid env var name %q", name, port.Name, port.Env)
			}
			variables := make([]string, 0, len(port.EnvBindings))
			for variable := range port.EnvBindings {
				variables = append(variables, variable)
			}
			sort.Strings(variables)
			for _, variable := range variables {
				if !envVarRe.MatchString(variable) {
					add("service %q: port %q: invalid env var name %q", name, port.Name, variable)
				}
				if !strings.Contains(port.EnvBindings[variable].Template, "{port}") {
					add("service %q: port %q: env binding %q template must contain {port}", name, port.Name, variable)
				}
			}
		}
		if h := s.Health; h != nil {
			if len(s.Ports) == 0 {
				add("service %q: health requires at least one port", name)
			}
			if h.Port != "" {
				if spec, ok := portSpecs[h.Port]; !ok {
					add("service %q: health.port %q is not a declared port", name, h.Port)
				} else if !spec.ProbeEnabled() {
					add("service %q: health.port %q has probe disabled", name, h.Port)
				}
			}
			if h.HTTP != "" && !strings.HasPrefix(h.HTTP, "/") {
				add("service %q: health.http %q must start with /", name, h.HTTP)
			}
		}
	}
	groupNames := make([]string, 0, len(p.Groups))
	for g := range p.Groups {
		groupNames = append(groupNames, g)
	}
	sort.Strings(groupNames)
	for _, g := range groupNames {
		if _, ok := p.Services[g]; ok {
			add("group %q: name collides with a service", g)
		}
		for _, m := range p.Groups[g] {
			if m == domain.AllServices {
				continue
			}
			if _, ok := p.Services[m]; !ok {
				add("group %q: unknown service %q", g, m)
			}
		}
	}
	pipelineNames := make([]string, 0, len(p.Pipelines))
	for n := range p.Pipelines {
		pipelineNames = append(pipelineNames, n)
	}
	sort.Strings(pipelineNames)
	for _, n := range pipelineNames {
		if _, err := p.ExpandTargets(p.PipelineNeeds[n]); err != nil {
			add("pipeline %q needs: %v", n, err)
		}
		for i, s := range p.Pipelines[n] {
			if (s.Task == "") == (s.Run == "") {
				add("pipeline %q step %d: exactly one of task or run is required", n, i+1)
			}
		}
	}
	for n, s := range p.Setup {
		if (s.Task == "") == (s.Run == "") {
			add("setup %q: exactly one of task or run is required", n)
		}
	}
	if _, err := p.StartOrder(p.ServiceNames()); err != nil {
		add("%v", err)
	}
	return problems
}
