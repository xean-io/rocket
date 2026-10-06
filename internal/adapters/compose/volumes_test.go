package compose

import (
	"slices"
	"testing"
)

func TestNamedVolumesFromConfig(t *testing.T) {
	for _, tt := range []struct {
		name, config string
		want         []string
		wantErr      bool
	}{
		{"mounted named only", `{"services":{"db":{"volumes":[{"type":"volume","source":"data","target":"/data"},{"type":"volume","source":"custom","target":"/custom"},{"type":"volume","source":"external","target":"/external"},{"type":"bind","source":"data","target":"/bind"},{"type":"volume","target":"/anonymous"},{"type":"volume","source":"data","target":"/again"}]},"other":{"volumes":[{"type":"volume","source":"unused","target":"/unused"}]}},"volumes":{"data":{"name":"rocket-test-dev_data"},"custom":{"name":"custom_data"},"external":{"name":"shared_data","external":true},"unused":{"name":"rocket-test-dev_unused"}}}`, []string{"custom_data", "rocket-test-dev_data"}, false},
		{"no volumes", `{"services":{"db":{}}}`, []string{}, false},
		{"unknown selected service", `{"services":{"other":{}}}`, nil, true},
		{"undefined named source", `{"services":{"db":{"volumes":[{"type":"volume","source":"missing"}]}}}`, nil, true},
		{"missing normalized name", `{"services":{"db":{"volumes":[{"type":"volume","source":"data"}]}},"volumes":{"data":{}}}`, nil, true},
		{"bad JSON", `not JSON`, nil, true},
	} {
		t.Run(tt.name, func(t *testing.T) {
			got, err := namedVolumesFromConfig([]byte(tt.config), "db")
			if (err != nil) != tt.wantErr || !slices.Equal(got, tt.want) {
				t.Fatalf("named volumes = %v, %v; want %v, error=%v", got, err, tt.want, tt.wantErr)
			}
		})
	}
}
