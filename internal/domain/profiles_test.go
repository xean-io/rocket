package domain

import (
	"slices"
	"strings"
	"testing"
)

func TestExpandStartupTargetsFiltersOnlyWildcards(t *testing.T) {
	p := &Project{Name: "profiles", Services: map[string]Service{
		"api":      {Name: "api", DependsOn: []string{"internal"}},
		"internal": {Name: "internal", Profiles: []string{"hidden"}},
		"ordinary": {Name: "ordinary"},
		"trends":   {Name: "trends", Profiles: []string{"trends", "reports"}},
	}, Groups: map[string][]string{"all": {"*"}, "explicit": {"trends"}, "mixed": {"*", "trends"}}}
	for _, tt := range []struct {
		name                    string
		targets, profiles, want []string
		wantErr                 string
	}{
		{"empty", nil, nil, []string{"api", "ordinary"}, ""},
		{"star", []string{"*"}, nil, []string{"api", "ordinary"}, ""},
		{"all group", []string{"all"}, nil, []string{"api", "ordinary"}, ""},
		{"any matching profile", []string{"all"}, []string{"reports"}, []string{"api", "ordinary", "trends"}, ""},
		{"all active", []string{"all"}, []string{"hidden", "trends"}, []string{"api", "internal", "ordinary", "trends"}, ""},
		{"unknown profile selects no gated service", []string{"all"}, []string{"unused"}, []string{"api", "ordinary"}, ""},
		{"explicit service", []string{"trends"}, nil, []string{"trends"}, ""},
		{"explicit group", []string{"explicit"}, nil, []string{"trends"}, ""},
		{"mixed group", []string{"mixed"}, nil, []string{"api", "ordinary", "trends"}, ""},
		{"mixed targets", []string{"*", "trends", "trends"}, nil, []string{"api", "ordinary", "trends"}, ""},
		{"invalid", []string{"unknown"}, nil, nil, "unknown service or group"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			got, err := p.ExpandStartupTargets(tt.targets, tt.profiles)
			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("error = %v, want %s", err, tt.wantErr)
				}
				return
			}
			if err != nil || !slices.Equal(got, tt.want) {
				t.Fatalf("targets = %v, %v; want %v", got, err, tt.want)
			}
		})
	}
	targets, err := p.ExpandStartupTargets(nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	order, err := p.StartOrder(targets)
	if err != nil || !slices.Equal(order, []string{"internal", "api", "ordinary"}) {
		t.Fatalf("gated dependency must remain selectable: %v, %v", order, err)
	}
	all, err := p.ExpandTargets([]string{"all"})
	if err != nil || !slices.Equal(all, p.ServiceNames()) {
		t.Fatalf("stop expansion must remain unfiltered: %v, %v", all, err)
	}
}

func TestMergeProfiles(t *testing.T) {
	got := MergeProfiles([]string{"trends", "base"}, []string{"extra", "base"}, []string{"trends", "reports"})
	if !slices.Equal(got, []string{"base", "extra", "reports", "trends"}) {
		t.Fatalf("profiles = %v", got)
	}
}
