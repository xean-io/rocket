# AGENTS.md — working on rocket itself

rocket is a Go CLI + daemon (one binary) that supervises dev processes. Design:
`odd/plan.md`; progress and evidence: `odd/tasks/rocket-mvp.md`; daemon API
contract: `internal/adapters/api/README.md`.

## Build and test

```sh
go build -o bin/rocket ./cmd/rocket
go vet ./... && go vet -tags integration ./...
go test ./...                         # unit tests, no processes or network
go test -race ./internal/...
go test -tags integration ./...       # e2e: builds the binary, runs a real daemon (needs python3)
go test -tags integration_docker ./internal/adapters/compose/   # needs Docker
GOOS=linux go build ./... && GOOS=windows go build ./...
```

Run targeted tests while iterating (`go test ./internal/app/ -run TestDeployGate`).

## Layout (hexagonal)

| Path | Role |
|---|---|
| `internal/domain` | pure model: projects, services, runs, jobs, events, dependency graph |
| `internal/ports` | interfaces the core depends on |
| `internal/app` | use cases (up/down/status/jobs/summary/reconcile/gc); tested with fakes in `fakes_test.go` |
| `internal/adapters/*` | process (pgid), compose, task, sqlite, probe, logs, events, api (HTTP+SSE) |
| `internal/daemon` | composition root: unix socket + token-protected TCP listener, `daemon.json` |
| `internal/client` | typed API client, daemon auto-start |
| `internal/manifest` | `rocket.yaml` load/validate/schema, dotenv |
| `internal/scaffold` | `rocket init` detection + rendering |
| `internal/agentdocs` | `rocket agent install` content + idempotent block upsert |
| `cmd/rocket` | cobra CLI; `*_integration_test.go` are the e2e tests |
| `testdata/` | fixtures only (`fixture/` for e2e, `init/` for `rocket init`, `manifests/`) |

## Rules

- Test first for app/domain logic: write the failing test, then the code.
- Integration tests use `//go:build integration` and a short temp `ROCKET_HOME`
  under `/tmp` (unix socket paths are limited to ~104 bytes).
- Never touch the real `~/.rocket` or `~/.claude` in tests: set `ROCKET_HOME` /
  `HOME` to temp dirs. Never signal processes rocket did not start.
- Keep `--json` shapes stable and documented in the API README; exit codes are
  0 ok, 1 error, 2 partial/job failure, 3 confirmation required.
- Code, comments and docs in English.
