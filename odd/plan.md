# Rocket — central dev orchestrator (CLI + daemon + native macOS app)

## Context

Several projects (e.g. `~/Documents/Code/nuvara`, `~/Documents/Code/autodropshipping`) each have their own
Taskfile + multiple compose files. Running "dev / some services / tests / local CI / prod smoke / deploy" means
remembering per-project tasks. Worse: AI agents launch processes ad hoc (`&`, `compose up -d`, watchers), leave them
running, and the next run hits busy ports. Nobody owns the processes.

Evidence from exploration:
- Both repos use `task` as entry point, `dotenv: [.env]`, compose with `${VAR:-default}` ports + healthchecks, profiles
  (`deps`, `apps`, `trends`, `release`, `ops`), separate prod compose, Dokploy deploys.
- Port clash: `:3000` = autodropshipping control-plane AND nuvara web. nuvara also takes default `6379`.
- autodropshipping `compose.yaml` and `docker-compose.yaml` share project name `xean-spectrai` → volume collision risk.
- nuvara `scripts/dev-hybrid.sh` already hand-rolls pgid tracking + `free-ports.sh` — proof the need is real.
- nuvara `CLAUDE.md:56` / `AGENTS.md:8`: agents must run targeted tests, never full suite → rocket must expose targeted runs.

Outcome: one tool, `rocket`, that is the **single owner** of every dev process across all projects, wraps existing
Taskfile/compose (no migration), and is what both humans and AI use. A native macOS app (SwiftUI, Liquid Glass) shows/controls everything live.

Decisions taken: Go CLI/daemon (cross-platform) · native macOS app (SwiftUI) first, Windows/Linux UI later (CLI works
everywhere) · wrap (not replace) Taskfile/compose · no git for now → no work-unit commits, tasks tracked in the feature doc only.

## Architecture

```
rocket (CLI, cobra)  ─┐
Rocket.app (SwiftUI)──┼──► rocketd (daemon, single supervisor)  ──► adapters: process / docker compose / go-task
AI agents (CLI --json)┘   unix socket ~/.rocket/rocketd.sock (CLI)  state: ~/.rocket/state.db (SQLite)
                          + 127.0.0.1:<random> w/ bearer token       logs:  ~/.rocket/logs/<project>/<svc>.log
                            (~/.rocket/daemon.json, 0600) for the app
                          HTTP+JSON API + SSE event stream
```

- One Go module, hexagonal layout (screaming by domain):
  - `internal/domain/` — Project, Service, Group, Env, Pipeline, Run, PortLease, Owner. Pure, no IO.
  - `internal/app/` — use cases: Up, Down, Restart, Status, Logs, RunJob, Deploy, Reconcile, GC, LeasePort.
  - `internal/ports/` — interfaces: `Runner`, `ComposeDriver`, `TaskDriver`, `Store`, `EventBus`, `PortProbe`, `HealthProbe`.
  - `internal/adapters/` — `process` (exec, `Setpgid`, group kill), `compose` (shells `docker compose -p … --env-file …`),
    `task` (shells `task <name>`), `sqlite` (modernc.org/sqlite, no CGO), `probe` (TCP/HTTP/compose health), `api` (http over unix socket).
  - `internal/manifest/` — `rocket.yaml` load, validate, defaults, JSON Schema export.
  - `cmd/rocket/` — CLI (talks to daemon; auto-starts it). `rocket daemon run` = the daemon itself (same binary).
  - `internal/adapters/process` split by build tags: `_unix.go` (pgid) now; `_windows.go` (Job Objects) later.
  - `macos/` — SwiftPM package for Rocket.app (SwiftUI). Pure API client of rocketd; no process logic in Swift.
- CLI never spawns services itself: every start goes through rocketd → no orphans by construction.

## Manifest (`rocket.yaml`, lives in each project root, wraps existing files)

```yaml
version: 1
name: nuvara
dotenv: [.env]
setup:
  doctor:  { task: doctor }
  install: { task: install }
  migrate: { task: "db:migrate" }
envs:
  dev:   { compose: [docker-compose.yml] }                  # default
  smoke: { compose: [docker-compose.prod.yml], profiles: [release] }
  stage: { deploy: { task: "release:trigger", confirm: true } }
  prod:  { deploy: { task: "release:ci",      confirm: true } }
services:
  postgres: { compose: postgres, profiles: [deps], ports: { main: { default: 5435, env: POSTGRES_PORT } } }
  redis:    { compose: redis,    profiles: [deps], ports: { main: { default: 6379, env: REDIS_PORT } } }
  api:      { run: "bun run dev", cwd: apps/api, depends_on: [postgres, redis],
              ports: { http: { default: 3002, env: PORT } }, health: { http: /api/health/live } }
  web:      { run: "bun run dev", cwd: apps/web, depends_on: [api], ports: { http: { default: 3000, env: PORT } } }
  causation:{ task: "dev:causation", ports: { http: { default: 3003 } } }
groups:
  deps: [postgres, redis]
  core: [api, web]
  all:  ["*"]
pipelines:
  test:      [{ task: test }]
  test:api:  [{ task: "test:api" }]
  ci:        [{ task: validate }, { task: lint }, { task: test }]
```

Service kinds: `compose:` (service in env's compose files), `task:` (long-running go-task), `run:` (raw command).
Compose project name forced to `rocket-<project>-<env>` → fixes the shared-name collision.

## Key mechanisms

1. **Process ownership** — local processes started with `Setpgid`; stop = SIGTERM to pgid, grace, SIGKILL. Compose
   services: `up -d --wait <svc>` / `stop`/`down` with forced project name. All runs persisted with pid/pgid/container ids.
2. **Reconcile on daemon start** — compare state.db with reality (pid alive? container running?); adopt or mark dead.
   `rocket gc` kills anything rocket started that has no live owner/lease.
3. **Port registry** — global leases in state.db + live probe (net.Listen / lsof for foreign holder with pid+cwd).
   Conflict → auto-remap to next free port and inject via the declared `env` var (both repos already honor
   `${VAR:-default}` / `PORT`). `rocket ports` shows the global map across projects.
4. **Dependency graph + health** — topo-sort `depends_on`; wait on compose health / TCP / HTTP before dependents.
5. **Owners + TTL (the AI fix)** — every run tagged `owner` (`user`, `agent:<session-id>` via `ROCKET_OWNER` env or
   `--owner`), optional `--ttl 30m`. `rocket down --owner agent:X` cleans everything an agent started; daemon expires TTLs.
6. **Jobs vs services** — one-shot jobs (setup, test, ci, migrate) run under daemon too: exit code, duration, log file,
   bounded tail returned. Deploy steps with `confirm: true` require `--yes` and are refused in agent mode unless the user passes it.

## CLI surface (all commands support `--json`, `-p <project>` default = cwd)

```
rocket init                      # detect Taskfile/compose, scaffold rocket.yaml
rocket projects add|ls|rm        # global project registry
rocket doctor | setup | install  # bootstrap (wraps tasks)
rocket up [svc|group…] [--env dev|smoke] [--owner] [--ttl]   # idempotent
rocket down [svc|group…|--all|--owner X|--everywhere]
rocket restart <svc> | ps [--all-projects] | logs <svc> [-f] [--tail N]
rocket ports | gc | open <svc>
rocket run <pipeline|task> [-- args]   # test, test:api, ci… returns exit code + log tail
rocket deploy <stage|prod> --yes
rocket daemon start|stop|status
rocket agent install             # writes skill + AGENTS.md snippet telling AI to use rocket only
rocket app                       # opens Rocket.app (macOS only for now)
```

## macOS native app — ultra-native, Apple HIG, Liquid Glass

Toolchain (verified): Swift 6.4, macOS 27.2 host, Go 1.26.3, **Xcode 27.0 installed and active** (actool available).
- Build with SwiftPM only (`macos/Package.swift`, swift-tools 6.4, Swift 6 language mode, strict concurrency) +
  `macos/script/build_and_run.sh` that assembles `Rocket.app` (Info.plist, `.icns` via `iconutil`, ad-hoc codesign).
- Deployment target **macOS 26** (Liquid Glass minimum). Load skills `build-macos-apps:liquid-glass`,
  `swiftui-patterns`, `window-management`, `swiftpm-macos`, `build-and-run-macos-app` before T8.
- With Xcode: Icon Composer `.icon` (Liquid Glass layered icon) compiled via `actool` into the bundle; notarization possible later.

Design rules (HIG, no custom chrome):
- System components only: `NavigationSplitView` sidebar (glass automatically), `.toolbar` with `ToolbarItemGroup`,
  `.searchable`, `Table` for services/ports/jobs, `Inspector` for service detail, `Settings` scene, `MenuBarExtra`
  (`.menuBarExtraStyle(.window)`), `commands` with keyboard shortcuts (⌘R restart, ⌘. stop, ⌘L logs).
- Liquid Glass: `glassEffect(.regular.tint(...).interactive())` for floating controls (service action bar, status
  pills), `GlassEffectContainer` + `glassEffectID` for morphing action groups, `.buttonStyle(.glass/.glassProminent)`;
  glass only on the controls layer, never on content (tables, logs) — per HIG.
- SF Symbols (hierarchical/palette rendering, `symbolEffect` for running/pulse states), system fonts (SF Pro / SF Mono
  for logs), Dynamic Type sizes, Reduce Transparency/Motion respected, full VoiceOver labels, light + dark.
- Swift: `@Observable` models, `@MainActor` UI, structured concurrency (`AsyncStream` from SSE), Swift Testing (`@Test`).

XEAN identity (source of truth: `~/Documents/Code/xean/brand/README.md`, `brand/philosophy.md`, `src/styles/tokens.css`).
Palette only — XEAN marks are NOT reused (brand rule: never recolour/redraw); Rocket gets its own SF-Symbol-based icon.
| Token | Hex | Use in Rocket |
|---|---|---|
| `xeanViolet` | `#9F63B9` | the ONLY accent: app `.tint`, prominent glass buttons, glass tint, running pulse |
| `xeanBone` | `#F3EFE6` | primary text on dark; light-mode ground |
| `xeanLacquer` | `#0B0B0B` | dark-mode ground; text on light |
| `xeanSurface` | `#141414` | dark card/inspector surface behind content |
| `xeanBorder` | `#2C2C2C` | hairline separators (dark) |
| `xeanGlow` | `#35293B` | low radial glow behind glass in window background (dark) |
| `xeanStone` | `#9A958C` | secondary text / metadata |
Philosophy "Horizon — discipline of subtraction" applied to UI: restraint, hairline (0.5pt) separators, no ornaments,
hierarchy by scale not weight, violet appears only as glow/accent, quiet monospaced uppercase tracked labels
(SF Mono, `.caption2`, `.tracking`) for metadata (ports, pids, owners). Dark is the signature appearance; light mode
uses Bone ground + Lacquer text. Status semantic colors stay system (`.green/.orange/.red`) for accessibility.

Structure:
- `MenuBarExtra` (rocket symbol + count of running services, quick stop-all) + main `Window` with `NavigationSplitView`.
- Views: Projects (status badges) → Project detail (services grid start/stop/restart, group buttons, env picker) ·
  Ports (global map, conflicts highlighted) · Logs (live tail per service) · Jobs (pipeline history, exit codes, rerun) ·
  Owners (what each agent left running, "stop all for owner").
- `RocketClient` (URLSession async/await) reads `~/.rocket/daemon.json` for port+token; SSE via `URLSession.bytes`
  → `@Observable` store. Container views (state) / presentational views (pure) split.
- Daemon not running → app runs `rocket daemon start` (path from `daemon.json` or `$PATH`).
- Later: Windows/Linux UI (decide then; API contract already shared).

## Tasks (ODD, tracked in `odd/tasks/rocket-mvp.md` + Engram; no git/commits for now)

- **T1** Scaffold module, layout, cobra root, manifest schema/loader/validation + tests (fixtures modeled on nuvara/autodropshipping).
- **T2** Daemon: unix-socket API, auto-start, SQLite store, process adapter (pgid), log capture, `ps/logs/down`.
- **T3** Compose + task adapters, dotenv/env resolution, forced compose project names.
- **T4** Port registry + probe + remap, reconcile on start, `ports`, `gc`.
- **T5** Orchestration: dep graph, health probes, groups, `up/restart`, owners + TTL.
- **T6** Jobs/pipelines (`setup/doctor/run/deploy` with confirm gate).
- **T7** AI layer: `--json` contract, `rocket agent install` (skill + AGENTS snippet), `init` auto-detect.
- **T8** Daemon TCP+token listener for app, then Rocket.app SwiftUI (client, store, views above).
- **T9** Write `rocket.yaml` for autodropshipping only (XEAN, like rocket). nuvara is out of scope — never write there.

MVP usable after T5 (+ `--json`). Test-first per task: domain/app tested with fake ports; adapters with integration
tests against a fixture project (`python3 -m http.server`, `sleep`, busybox compose).

## Verification

- `go test ./...` green; `go vet`; integration tests tagged `integration`.
- Fixture: `rocket up all` → `rocket ps --json` all healthy → `rocket down --all` → `lsof -iTCP -sTCP:LISTEN` shows none of fixture ports, `docker ps` none of `rocket-*`.
- Orphan test: `kill -9` the daemon mid-run → `rocket daemon start` → reconcile reports/cleans; `rocket gc` leaves zero.
- Conflict test: fixture service on :3000 (or any foreign :3000 holder) then autodropshipping `rocket up control-plane` → remapped (e.g. 3100), both reachable, `rocket ports` shows both.
- Owner test: `ROCKET_OWNER=agent:t1 rocket up core --ttl 1m` → after TTL / `rocket down --owner agent:t1` nothing left.
- App: `swift build` + `swift test` in `macos/`; launch Rocket.app, start/stop service from UI, logs stream live,
  CLI changes reflected in UI within 1s; screenshot check via computer-use.
- Cross-platform CLI sanity: `GOOS=linux go build ./...` and `GOOS=windows go build ./...` compile (Windows process
  adapter may be a stub returning "unsupported" until implemented).
