# Feature: rocket-mvp

Locator: `odd/tasks/rocket-mvp.md` · Engram topic `odd/rocket-mvp/tasks` · Full design: `odd/plan.md`

## Objective
Single supervisor (`rocket` CLI + `rocketd` daemon, Go) owning every dev process across projects, wrapping existing
Taskfile/compose via `rocket.yaml`; native macOS SwiftUI app (Liquid Glass, XEAN identity) to see/control everything.

## Problem / Why
AI agents and humans launch processes ad hoc; orphans + busy ports (e.g. :3000 nuvara vs autodropshipping).

## Constraints
- No git yet → no work-unit commits; RDD native review unavailable (needs git). Evidence recorded here instead.
- Go 1.26.3, Swift 6.4 + Xcode 27.0 (installed 2026-10-04), macOS 27.2 host, deployment target macOS 26.
- Writes outside this repo (autodropshipping rocket.yaml, T9; nuvara out of scope) need explicit user OK.
- Artifacts in English.

## Delivery strategy
exception-ok (no git; single local tree). Forecast >> 400 lines; slices tracked per task below.

## Tasks
- [x] T1 Scaffold Go module, hexagonal layout, cobra root, manifest schema/loader/validation + tests — route: delegated (writer trigger, 2+ files)
- [x] T2 Daemon: unix-socket API, auto-start, SQLite store, process adapter (pgid), log capture, ps/logs/down — delegated
- [x] T3 Compose + task adapters, dotenv/env resolution, forced compose project names — delegated
- [x] T4 Port registry + probe + remap, reconcile on start, ports, gc — delegated
- [x] T5 Orchestration: dep graph, health probes, groups, up/restart, owners + TTL — delegated
- [x] T6 Jobs/pipelines (setup/doctor/run/deploy with confirm gate) — delegated (writer trigger, 2+ files)
- [x] T7 AI layer: --json contract, `rocket status`, `rocket agent install`, `init` auto-detect, repo README/AGENTS.md — delegated
- [x] T8 Daemon TCP+token listener + Rocket.app SwiftUI — delegated (two writers: Go daemon part, Swift app under macos/).
  Evidence: swift build (0 warnings), swift build -c release, swift test 34/34 (parent re-ran: 34 passed), build_and_run.sh → macos/build/Rocket.app,
  codesign adhoc com.xean.rocket, plutil OK, live self-check vs real daemon (auto-start, ps/ports/jobs, SSE, restart + rerun).
  Liquid Glass APIs all compiled on Xcode 27 SDK (no fallbacks); Icon Composer `.icon` compiled via actool.
  Parent fix: SSE stream now opens with `: ok` comment (URLSession stalled 15s until first ping). RED observed in
  internal/adapters/api TestEvents, then GREEN; go vet + go test ./... + integration (cmd, api) green.
  Gaps: no env list in API (picker uses default_env + seen envs); no "start new job" control in app; menu bar extra and
  light mode not visually verified.
- [x] T9 rocket.yaml for autodropshipping ONLY (XEAN project; user decided nuvara is out of scope) + e2e — delegated
  (one writer; user authorized writing `autodropshipping/rocket.yaml` + e2e there). Evidence in "T9" below.

## Acceptance / checks
See `odd/plan.md` → Verification. Per task: `go vet ./...`, `go test ./...`; T8: `swift build`, `swift test`.

## Progress / evidence
T1–T5 implemented in one delegated writer pass (route: delegated, writer trigger — 2+ non-trivial files).
Test-first: RED observed as compile failures for `internal/manifest` and `internal/app` before implementation;
`internal/domain` tests were written in the same batch as the code (RED not separately observed).

Package map: `internal/domain` (model, graph, remap, env merge) · `internal/manifest` (load/validate/schema/dotenv)
· `internal/ports` (interfaces) · `internal/app` (Up/Down/Restart/Status/Logs/Ports/GC/Reconcile/ExpireTTL)
· `internal/adapters/{process,compose,task,sqlite,probe,api,events,logs}` · `internal/daemon` (composition root)
· `internal/client` (unix-socket client + auto-start) · `internal/paths` · `cmd/rocket` (cobra).
API contract: `internal/adapters/api/README.md`. Fixture: `testdata/fixture/` (python http.server + sleep).

Evidence (2026-10-05, macOS, Go 1.26.3):
- `go vet ./...`: ok (also `go vet -tags integration ./...`: ok)
- `go test ./...`: ok, 10 packages with tests; `go test -race ./internal/...`: 10/10 ok
- `go test -tags integration ./...`: ok (e2e `TestEndToEnd` 7 subtests: up all/idempotent, dotenv+logs,
  ports, down --all frees ports, remap 18431→18531 with holder pid / no-env port fails exit 2,
  owners + `--ttl 2s` expiry + `down --owner`, daemon `kill -9` → reconcile adopts live / marks dead → gc → down --everywhere)
- `lsof -nP -iTCP -sTCP:LISTEN | grep python` after integration + smoke: empty; no `rocket daemon` / `sleep 600` left
- `go test -tags integration_docker ./internal/adapters/compose/`: ok (busybox up --wait / Running / stop / down, project `rocket-itest-dev`)
- `GOOS=linux go build ./...`: ok · `GOOS=windows go build ./...`: ok (process adapter is an ErrUnsupported stub)
- Smoke (ROCKET_HOME=/tmp/rksmoke.*): `bin/rocket up all --json` 4×started exit 0 · `ps --json` 4×running ·
  `ports` 18431/18432/18433 · `down --all` exit 0 · `daemon stop` exit 0 · `daemon status` exit 1 afterwards

Known gaps (honest):
- Compose services: `logs -f` only streams rocket's own `compose up` output (container logs via `logs` without -f).
- Port probe/remap is check-then-bind (TOCTOU window); remap may pick a port that another not-yet-started service of
  the same project declares as default.
- `gc` never kills unknown pgids (pid-reuse safety); it stops only active runs of unregistered projects.
- Two checkouts with the same `name:` conflict in the registry (409) unless the old path lost its rocket.yaml.
- Services without ports report `healthy` after a 500ms start grace (no real probe).

### T6–T8 (daemon part) — 2026-10-05, one delegated writer pass (~3.2k new Go lines incl. tests, ~0.5k doc lines;
exceeds the 400-line heuristic because three tasks shipped together with their tests and docs)

Built:
- T6: `domain.Job` (+ `job.state`/`job.log` events, `Deploy.Run`), `jobs` SQLite table (columns + full JSON `data`,
  fixed-width sortable `started_at`), `app.StartJob/WaitJob/GetJob/ListJobs/JobLogs/CancelJob`, sequential steps under
  the process adapter (own pgid, dotenv env, `ROCKET_*` stripped), args appended to the last step (`task -- args` /
  `sh -c '<run> "$@"'`), Taskfile fallback for unknown pipelines, setup sequence doctor→install→migrate, deploy gate
  (`confirm` needs `yes`; `agent:*` owners need `yes` + `ROCKET_ALLOW_DEPLOY=1`; HTTP 428 → CLI exit 3), reconcile marks
  vanished running jobs `lost` (live ones adopted by polling, then `lost` since the exit code is unobservable), `down`
  (whole project / `--owner` / `--everywhere`) cancels matching running jobs, `gc` reports `lost_job` and prunes finished
  jobs beyond 100 per project (logs deleted). Log sink: job logs at `logs/<project>/jobs/<id>.log`, per-follower stop
  (`CloseJob` final poll before the terminal `job.state`), `logs.FollowFile` for gap-free log streaming.
  API: `POST/GET /v1/jobs`, `GET /v1/jobs/{id}[?wait=true]`, `GET /v1/jobs/{id}/logs?tail&follow`,
  `POST /v1/jobs/{id}/cancel`, events filter `job=`. CLI: `run [--detach] [-- args]`, `setup [name]`,
  `doctor|install|migrate`, `deploy <env> [--yes]`, `jobs [--all-projects]`, `job <id> [logs [-f]]`, `job cancel <id>`.
- T7: `rocket status` (`GET /v1/status`: project, services, running jobs, `port_remapped`/`port_busy` conflicts);
  JSON errors always carry `code` (`error` for local failures); `internal/scaffold` + `rocket init [--force] [--print]`
  (compose `${VAR:-default}` ports incl. long syntax, profiles, dev/smoke env guess, override files, shared compose
  name note, Taskfile tasks + includes up to 3 levels, long-running guesses → task services, setup guesses,
  deploy/release tasks excluded from pipelines, hint comment block; output validated with `manifest.Parse` before
  writing); `internal/agentdocs` + `rocket agent install [--target] [--global] [--print]` (skill + marked block,
  idempotent); repo `README.md` + `AGENTS.md`. Fixture `testdata/init/autodropshipping/` (data only).
- T8 daemon: `127.0.0.1:0` listener serving the same handler behind `Authorization: Bearer <64-hex>` (constant-time),
  `$ROCKET_HOME/daemon.json` (0600, atomic, removed before listeners close), `HealthInfo.http`, `rocket app`.

Test-first: RED observed as compile failures before implementation for `internal/app` (jobs/summary tests),
`internal/adapters/{sqlite,logs,task}` (new port methods), `internal/adapters/api` (428 mapping + `job` filter: real
assertion failures), `internal/daemon` (token/daemon.json), `internal/scaffold`, `internal/agentdocs`. Not observed RED:
manifest deploy-validation cases and the e2e tests (written after the code; all passed on first run).

Evidence (2026-10-05, macOS, Go 1.26.3):
- `go vet ./...`: ok · `go vet -tags integration ./...`: ok · `gofmt -l .`: empty
- `go test ./...`: ok, 13 packages with tests · `go test -race -count=1 ./internal/...`: 13/13 ok
- `go test -tags integration -count=1 ./...`: ok (15 packages; `TestEndToEnd` unchanged + new `TestJobsAndAI` 14 subtests:
  pipeline success/failure exit 0/2, args after `--`, unknown pipeline invalid, setup + doctor, human streaming,
  `--detach` + `jobs` + `job cancel` (pgid gone) + `job <id> logs`, deploy gate exit 3 / `--yes` / agent needs
  `ROCKET_ALLOW_DEPLOY=1`, `status` + `down --owner` cancels agent job, TCP 401 without/wrong token and 200 with token +
  daemon.json 0600, daemon `kill -9` → job `lost`, `init` on the autodropshipping-shaped fixture (refuse/--force/--print,
  daemon loads the generated manifest), `agent install` with HOME override idempotent, `daemon stop` removes daemon.json).
  One flaky run found and fixed (daemon.json was removed after the socket closed); 3 consecutive reruns ok.
- `GOOS=linux go build ./...`: ok · `GOOS=windows go build ./...`: ok
- Smoke (`ROCKET_HOME=/tmp/rk.o4nL`, in `testdata/fixture`): `run check --json` exit 0 succeeded · `run broken --json`
  exit 2 (exit_code 3) · `status --json` static running, 0 jobs, 0 conflicts · `deploy stage --json` exit 3
  `confirmation_required` · daemon.json `-rw-------` · `init --print` on the init fixture ok · `down --all` · `daemon stop`
  exit 0 · afterwards no `rocket daemon run`, no fixture processes, no listeners on 18431–18433/18531; tmp home removed.

Known gaps (T6–T8, honest):
- Jobs have no TTL; a forgotten `--detach` job runs until canceled, `down` (project/owner/everywhere) or daemon restart+exit.
- After a daemon restart an adopted job ends `lost` even if its step succeeded, and later steps never run.
- `GET /v1/jobs/{id}/logs?follow=true` sends no `: ping` heartbeats (file-follow loop), unlike `/v1/events`.
- The agent deploy gate trusts the caller's environment (`ROCKET_ALLOW_DEPLOY`); it prevents accidents, not malice.
- `rocket status` without a project reports no conflicts (defaults need each manifest).
- `init` guesses: env names from file names (`smoke` for extra un-suffixed files), task services get no ports,
  port names `main`/`p<target>`; compose `include:`/`extends:` and `${VAR:?err}` are not parsed.
- `rocket app` only works once Rocket.app exists (T8 Swift part); `ROCKET_APP` may point to a built bundle.
- Engram mirror of this document: synced by parent after T9 (topic odd/rocket-mvp/tasks). Parent spot check T9: `rocket status --json` in autodropshipping exit 0.

### T9 — 2026-10-05, autodropshipping `rocket.yaml` + e2e (route: delegated writer)

Written: the pilot project's `rocket.yaml` (only file changed there; untracked, not
committed). `name: xean-spectrai`, `dotenv: [.env]`, env `dev` (compose.yaml, default) + `stage` deploy
`deploy:staging` confirm. No smoke env: docker-compose.yaml is the Dokploy stack (`${VAR:?}` production secrets,
`expose` only). Setup: install→`task setup`, migrate→`task db:migrate`, `assets` (manual, `COMPOSE_PROJECT_NAME`
pinned because `ensure-asset-bucket.sh` runs `docker compose exec` without `-p`); no doctor task exists.
Services: postgres/redis (POSTGRES_PORT/REDIS_PORT), nats/minio (literal ports, not remappable), obscura/trends-api
(profile trends), api (`API_BIND="0.0.0.0:${API_PORT}" task rust:api`, health `/api/v1/health`), worker,
fulfillment-retries, ai-worker (`CREATIVE_PUBLIC_BIND` built from `AI_WORKER_PORT`), commerce (COMMERCE_PORT,
health `/health`), control-plane (`bun run dev`, PORT), storefront (`bun run dev --port "$PORT"`). depends_on mirrors
docker-compose.yaml; web apps have none (they reach api/Medusa at request time). Groups infra, trends, rust, web,
autonomous, all. Pipelines check, test, test:rust, test:bun, test:e2e, test:resilience, audit, api:generate, ci
(api:generate → `git diff --exit-code -- packages/api-client` → check → test).

Evidence (binary built from source, `ROCKET_HOME=/tmp/rkt9/home`, Docker 29.8.1):
- `rocket status --json` in autodropshipping: exit 0, 13 services loaded, no validation errors.
- Baseline before: listeners Control Center 5000/7000, Docker 5432 (`nuvara-ci-pg`, untouched), engram, node 7265 —
  none on rocket ports. Foreign holder started by this check: `python3 -m http.server 3000 --bind 127.0.0.1`.
- `rocket up infra --json`: exit 0, 23.6s; minio/nats/postgres/redis started + healthy; `docker ps` shows
  `rocket-xean-spectrai-dev-*` containers `(healthy)`, label project `rocket-xean-spectrai-dev`.
- `rocket up control-plane --json`: exit 0; remap http 3000→3100 via PORT, holder `Python (pid …) in /private/tmp/rkt9`;
  `curl http://127.0.0.1:3100/` → 307 (auth redirect), holder still 200 on :3000; log shows Next.js 16.3.4 on 3100.
- `rocket ps --json` (5 running healthy), `rocket ports --json` (7 leases incl. 3100), `rocket status --json`
  (conflict `port_remapped` 3000→3100), `rocket logs control-plane --tail 20`: all exit 0.
- Cheap job: `rocket run default --json` (Taskfile fallback, `task --list`): succeeded, exit 0, 89 ms. No pipeline
  run (all need cargo/bun builds > 1 min or write generated files).
- `rocket down --all --json`: exit 0, 1.1s, 5 stopped, `compose_down: [rocket-xean-spectrai-dev]`;
  `rocket daemon stop`: exit 0. Afterwards: no `rocket-*` containers (`docker ps -a`), no `rocket daemon` process,
  listener set identical to baseline (minus the daemon), control-plane pids gone; holder killed; /tmp/rkt9 removed.
- Left on purpose (data): volumes `rocket-xean-spectrai-dev_{minio,nats,postgres,redis}-data`; network removed by down.
  Existing `xean-spectrai_*` volumes are not reused, so a fresh rocket dev DB needs `rocket migrate` + `rocket setup assets`.

Skipped: api/worker/commerce/ai-worker/storefront/trends not started (cargo builds, migrations, image builds);
no `.env.example` read (blocked by a permission deny rule) — dotenv contents unverified.

Gaps / rocket limitations found (not fixed):
- Port injection is a bare number; apps taking `host:port` (API_BIND, CREATIVE_PUBLIC_BIND) need a `run:` shell
  wrapper. A template (e.g. `env: { API_BIND: "0.0.0.0:{port}" }`) would let them stay `task:` services.
- No cross-service port references: a remapped api/commerce port is not propagated to control-plane/storefront.
- Jobs and task services get no `COMPOSE_PROJECT_NAME`, so Taskfile `docker compose` calls hit the original project
  (`xean-spectrai`), not `rocket-<name>-<env>`; rocket could inject it for the default env.
- Pipelines cannot declare required services (test:e2e/test:resilience need infra + migrations).
- Non-compose services with ports are always probed; there is no "lease but don't probe" option.
- `all: ["*"]` includes profile-gated trends services (image builds) — `rocket up` with no args builds them.
- `rocket init` misses `setup: install` when the Taskfile's install task is named `setup`.

## Next step
MVP tasks T1–T9 done. Optional follow-ups: the rocket gaps listed under T9 (port templates, COMPOSE_PROJECT_NAME
for jobs, pipeline service deps).
