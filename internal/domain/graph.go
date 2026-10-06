package domain

import (
	"fmt"
	"slices"
	"sort"
	"strings"
)

// AllServices is the group member that expands to every service.
const AllServices = "*"

// ServiceNames returns all service names sorted.
func (p *Project) ServiceNames() []string {
	names := make([]string, 0, len(p.Services))
	for n := range p.Services {
		names = append(names, n)
	}
	sort.Strings(names)
	return names
}

// ExpandTargets resolves service and group names to a sorted, unique set of
// service names. An empty input selects every service.
func (p *Project) ExpandTargets(targets []string) ([]string, error) {
	return p.expandTargets(targets, p.ServiceNames())
}

// ExpandStartupTargets filters only wildcard members by the active profiles.
// Explicit service/group members and dependencies remain selectable.
func (p *Project) ExpandStartupTargets(targets, profiles []string) ([]string, error) {
	active := map[string]bool{}
	for _, profile := range profiles {
		active[profile] = true
	}
	var eligible []string
	for _, name := range p.ServiceNames() {
		svc := p.Services[name]
		include := len(svc.Profiles) == 0
		for _, profile := range svc.Profiles {
			include = include || active[profile]
		}
		if include {
			eligible = append(eligible, name)
		}
	}
	return p.expandTargets(targets, eligible)
}

func (p *Project) expandTargets(targets, wildcard []string) ([]string, error) {
	if len(targets) == 0 {
		return slices.Clone(wildcard), nil
	}
	set := map[string]bool{}
	for _, t := range targets {
		switch {
		case t == AllServices:
			for _, n := range wildcard {
				set[n] = true
			}
		case p.hasService(t):
			set[t] = true
		case p.Groups[t] != nil:
			for _, m := range p.Groups[t] {
				if m == AllServices {
					for _, n := range wildcard {
						set[n] = true
					}
					continue
				}
				set[m] = true
			}
		default:
			return nil, fmt.Errorf("unknown service or group %q in project %s", t, p.Name)
		}
	}
	out := make([]string, 0, len(set))
	for n := range set {
		out = append(out, n)
	}
	sort.Strings(out)
	return out, nil
}

// MergeProfiles returns a sorted, unique profile selection from all layers.
func MergeProfiles(layers ...[]string) []string {
	set := map[string]bool{}
	for _, profiles := range layers {
		for _, profile := range profiles {
			set[profile] = true
		}
	}
	out := make([]string, 0, len(set))
	for profile := range set {
		out = append(out, profile)
	}
	sort.Strings(out)
	return out
}

func (p *Project) hasService(name string) bool {
	_, ok := p.Services[name]
	return ok
}

// StartOrder returns the targets plus their transitive dependencies in an
// order where every dependency comes before its dependents. Ties are broken
// alphabetically so the order is deterministic.
func (p *Project) StartOrder(targets []string) ([]string, error) {
	closure := map[string]bool{}
	var visit func(string)
	visit = func(n string) {
		if closure[n] {
			return
		}
		closure[n] = true
		for _, d := range p.Services[n].DependsOn {
			visit(d)
		}
	}
	for _, t := range targets {
		visit(t)
	}

	indegree := map[string]int{}
	dependents := map[string][]string{}
	for n := range closure {
		indegree[n] += 0
		for _, d := range p.Services[n].DependsOn {
			if closure[d] {
				indegree[n]++
				dependents[d] = append(dependents[d], n)
			}
		}
	}
	var ready []string
	for n, deg := range indegree {
		if deg == 0 {
			ready = append(ready, n)
		}
	}
	sort.Strings(ready)
	var order []string
	for len(ready) > 0 {
		n := ready[0]
		ready = ready[1:]
		order = append(order, n)
		for _, m := range dependents[n] {
			indegree[m]--
			if indegree[m] == 0 {
				ready = append(ready, m)
				sort.Strings(ready)
			}
		}
	}
	if len(order) != len(closure) {
		var stuck []string
		for n, deg := range indegree {
			if deg > 0 {
				stuck = append(stuck, n)
			}
		}
		sort.Strings(stuck)
		return nil, fmt.Errorf("dependency cycle between services: %s", strings.Join(stuck, ", "))
	}
	return order, nil
}

// StopOrder sorts the given services so dependents stop before their
// dependencies. Unknown services are kept at the end.
func (p *Project) StopOrder(services []string) []string {
	known := make([]string, 0, len(services))
	var unknown []string
	for _, s := range services {
		if p.hasService(s) {
			known = append(known, s)
		} else {
			unknown = append(unknown, s)
		}
	}
	full, err := p.StartOrder(known)
	if err != nil {
		out := slices.Clone(services)
		sort.Strings(out)
		return out
	}
	want := map[string]bool{}
	for _, s := range known {
		want[s] = true
	}
	var out []string
	for i := len(full) - 1; i >= 0; i-- {
		if want[full[i]] {
			out = append(out, full[i])
		}
	}
	sort.Strings(unknown)
	return append(out, unknown...)
}
