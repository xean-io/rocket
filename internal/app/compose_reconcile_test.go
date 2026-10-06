package app

import (
	"context"
	"testing"

	"github.com/xean-io/rocket/internal/domain"
)

func TestReconcileComposeUsesLoadedAlias(t *testing.T) {
	h := newHarness(t)
	p := nuvara()
	svc := p.Services["postgres"]
	svc.Compose = "database"
	p.Services["postgres"] = svc
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{p.Root: p}}
	h.up(t, UpRequest{Services: []string{"postgres"}})
	res, err := h.app.Reconcile(context.Background())
	if err != nil || len(res.Dead) != 0 || len(res.Adopted) != 1 {
		t.Fatalf("live Compose alias marked dead: %+v, %v", res, err)
	}
	if got := h.run(t, p.Name, "postgres"); !got.State.Active() {
		t.Fatalf("alias run no longer active: %+v", got)
	}
}

func TestReconcileComposeMissingManifestKeepsLegacyFallback(t *testing.T) {
	h := newHarness(t)
	h.up(t, UpRequest{Services: []string{"postgres"}})
	h.app.d.Manifests = fakeLoader{projects: map[string]*domain.Project{}}
	res, err := h.app.Reconcile(context.Background())
	if err != nil || len(res.Dead) != 0 || len(res.Adopted) != 1 {
		t.Fatalf("missing manifest lost the stored service fallback: %+v, %v", res, err)
	}
}
