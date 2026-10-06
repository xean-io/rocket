package api

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/xean-io/rocket/internal/app"
)

func TestErrorMapping(t *testing.T) {
	tests := []struct {
		err    error
		status int
		code   string
	}{
		{fmt.Errorf("%w: bad", app.ErrInvalid), http.StatusBadRequest, "invalid"},
		{fmt.Errorf("%w: gone", app.ErrNotFound), http.StatusNotFound, "not_found"},
		{fmt.Errorf("%w: taken", app.ErrConflict), http.StatusConflict, "conflict"},
		{fmt.Errorf("%w: needs --yes", app.ErrConfirmation), http.StatusPreconditionRequired, "confirmation_required"},
		{errors.New("boom"), http.StatusInternalServerError, "internal"},
	}
	for _, tt := range tests {
		rec := httptest.NewRecorder()
		writeErr(rec, tt.err)
		var body ErrorBody
		_ = json.Unmarshal(rec.Body.Bytes(), &body)
		if rec.Code != tt.status || body.Code != tt.code || body.Error != tt.err.Error() {
			t.Errorf("%v -> %d %+v", tt.err, rec.Code, body)
		}
	}
}

func TestHealthEndpoint(t *testing.T) {
	rec := httptest.NewRecorder()
	(&Server{Info: HealthInfo{OK: true, API: Version, PID: 7}}).Handler().ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/v1/health", nil))
	var info HealthInfo
	_ = json.NewDecoder(rec.Body).Decode(&info)
	if rec.Code != http.StatusOK || !info.OK || info.API != "v1" || info.PID != 7 {
		t.Fatalf("info %+v", info)
	}
}
