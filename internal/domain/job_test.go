package domain

import (
	"testing"
	"time"
)

func TestJobExpired(t *testing.T) {
	deadline := time.Date(2026, 10, 5, 0, 0, 0, 0, time.UTC)
	for _, tc := range []struct {
		name string
		job  Job
		now  time.Time
		want bool
	}{
		{"no deadline", Job{}, deadline.Add(time.Hour), false},
		{"before deadline", Job{ExpiresAt: &deadline}, deadline.Add(-time.Nanosecond), false},
		{"at deadline", Job{ExpiresAt: &deadline}, deadline, true},
		{"after deadline", Job{ExpiresAt: &deadline}, deadline.Add(time.Nanosecond), true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.job.Expired(tc.now); got != tc.want {
				t.Fatalf("Expired(%v) = %v, want %v", tc.now, got, tc.want)
			}
		})
	}
}
