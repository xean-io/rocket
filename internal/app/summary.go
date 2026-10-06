package app

import (
	"context"
	"fmt"
	"sort"

	"github.com/xean-io/rocket/internal/domain"
)

// Conflict kinds reported by Summary.
const (
	ConflictPortRemapped = "port_remapped" // running on another port than declared
	ConflictPortBusy     = "port_busy"     // a stopped service's default port is taken
)

// ProjectInfo identifies the project of a summary.
type ProjectInfo struct {
	Name       string   `json:"name"`
	Root       string   `json:"root"`
	DefaultEnv string   `json:"default_env"`
	Envs       []string `json:"envs"`
	Pipelines  []string `json:"pipelines"`
	DeployEnvs []string `json:"deploy_envs"`
}

// Conflict is a port problem an agent should know about before acting.
type Conflict struct {
	Kind       string             `json:"kind"`
	Project    string             `json:"project"`
	Service    string             `json:"service"`
	PortName   string             `json:"port_name"`
	Port       int                `json:"port"`    // remapped port, or the busy default
	Default    int                `json:"default"` // port declared in rocket.yaml
	Env        string             `json:"env,omitempty"`
	Remappable bool               `json:"remappable"` // declares an env var rocket can inject
	Holder     *domain.PortHolder `json:"holder,omitempty"`
	Detail     string             `json:"detail"`
}

// Summary is the one-shot overview behind `rocket status`.
type Summary struct {
	Project   *ProjectInfo `json:"project,omitempty"`
	Services  []domain.Run `json:"services"`
	Jobs      []domain.Job `json:"jobs"` // running jobs
	Conflicts []Conflict   `json:"conflicts"`
}

// Summary reports services, running jobs and port conflicts of one project,
// or of every project when project is empty (conflicts need a project).
func (a *App) Summary(ctx context.Context, project string) (Summary, error) {
	st, err := a.Status(ctx, StatusRequest{Project: project})
	if err != nil {
		return Summary{}, err
	}
	sum := Summary{Services: st.Services, Jobs: []domain.Job{}, Conflicts: []Conflict{}}
	jobProject := ""
	if project != "" {
		p, err := a.ResolveProject(project)
		if err != nil {
			return Summary{}, err
		}
		sum.Project = &ProjectInfo{Name: p.Name, Root: p.Root, DefaultEnv: p.DefaultEnv,
			Envs: []string{}, Pipelines: []string{}, DeployEnvs: []string{}}
		for name, env := range p.Envs {
			sum.Project.Envs = append(sum.Project.Envs, name)
			if env.Deploy != nil {
				sum.Project.DeployEnvs = append(sum.Project.DeployEnvs, name)
			}
		}
		for name := range p.Pipelines {
			sum.Project.Pipelines = append(sum.Project.Pipelines, name)
		}
		sort.Strings(sum.Project.Envs)
		sort.Strings(sum.Project.Pipelines)
		sort.Strings(sum.Project.DeployEnvs)
		sum.Conflicts = a.conflicts(p)
		jobProject = p.Name
	}
	jobs, err := a.d.Store.ListJobs(jobProject, 0)
	if err != nil {
		return Summary{}, err
	}
	now := a.now()
	for _, j := range jobs {
		if j.Status == domain.JobRunning {
			j.DurationMS = now.Sub(j.StartedAt).Milliseconds()
			sum.Jobs = append(sum.Jobs, j)
		}
	}
	return sum, nil
}

func (a *App) conflicts(p *domain.Project) []Conflict {
	out := []Conflict{}
	for _, name := range p.ServiceNames() {
		svc := p.Services[name]
		run, ok, _ := a.d.Store.GetRun(p.Name, name)
		active := ok && run.State.Active()
		for _, spec := range svc.Ports {
			c := Conflict{Project: p.Name, Service: name, PortName: spec.Name, Default: spec.Default, Env: spec.Env, Remappable: spec.Env != ""}
			if active {
				actual := run.Ports[spec.Name]
				if actual == 0 || actual == spec.Default {
					continue
				}
				c.Kind, c.Port = ConflictPortRemapped, actual
				c.Detail = fmt.Sprintf("running on %d instead of %d (injected via %s)", actual, spec.Default, spec.Env)
				out = append(out, c)
				continue
			}
			reason, holder := a.portTaken(spec.Default, p.Name, name)
			if reason == "" {
				continue
			}
			c.Kind, c.Port, c.Holder = ConflictPortBusy, spec.Default, holder
			if c.Remappable {
				c.Detail = fmt.Sprintf("%s; `rocket up %s` will remap it via %s", reason, name, spec.Env)
			} else {
				c.Detail = fmt.Sprintf("%s; `rocket up %s` will fail (no env var to remap)", reason, name)
			}
			out = append(out, c)
		}
	}
	return out
}

// portTaken is a read-only variant of portBusy (no stale lease cleanup).
func (a *App) portTaken(port int, project, service string) (string, *domain.PortHolder) {
	leases, _ := a.d.Store.ListLeases()
	for _, l := range leases {
		if l.Port != port || (l.Project == project && l.Service == service) {
			continue
		}
		if run, ok, _ := a.d.Store.GetRun(l.Project, l.Service); ok && run.State.Active() {
			return fmt.Sprintf("leased by %s/%s", l.Project, l.Service), nil
		}
	}
	if a.d.Probe.Free(port) {
		return "", nil
	}
	if holder, _ := a.d.Probe.Holder(port); holder != nil {
		return fmt.Sprintf("in use by %s (pid %d)", holder.Command, holder.PID), holder
	}
	return "in use by an unknown process", nil
}
