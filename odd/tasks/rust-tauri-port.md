# Feature: Rust core + Tauri desktop app

## Objective

Rewrite the Go CLI + daemon (`rocket`) in Rust and replace the SwiftUI macOS app
(`macos/`) with a Tauri v2 app (React + Vite + TypeScript + shadcn/ui), latest
stable versions of everything.

## Why

User request (2026-10-06): single Rust codebase for CLI, daemon and desktop
shell; cross-platform UI via Tauri instead of macOS-only SwiftUI.

## Scope

- Cargo workspace at repo root (`crates/*`, `apps/desktop/src-tauri`).
- Rust binary `rocket` with identical CLI, flags, exit codes, `--json` shapes,
  HTTP/SSE API (`internal/adapters/api/README.md`), `daemon.json`, `ROCKET_HOME`
  layout and SQLite `state.db` (reads existing state; JSON `data` blobs stay
  compatible).
- Tauri app `apps/desktop` with feature parity to `macos/` (sidebar, project
  detail, services table, inspector, ports, jobs + log follow, owners, settings,
  tray/menu bar, deploy confirmation, keyboard shortcuts, XEAN theme).
- Go code and `macos/` removed only after parity is verified.

## Constraints

- Contract source of truth: `internal/adapters/api/README.md`, `AGENTS.md`.
- Parity oracle: existing Go e2e suite (`cmd/rocket/*_integration_test.go`) run
  against the Rust binary via a `ROCKET_BIN` override.
- Never touch real `~/.rocket` / `~/.claude` in tests; never signal processes
  rocket did not start.
- Orchestration: Opus (parent). Writers: Sonnet subagents, one at a time.
- Code, comments, docs and UI copy in English.

## Delivery

- Strategy: `single-pr` (user choice 2026-10-06): one PR from
  `feat/rust-tauri-port` to `main`; work-unit commits per task inside it.
- Forecast: ~18-22k authored changed lines (Rust ~11k + tests ~7k, Tauri ~5k),
  well above the 400-line budget → chained PRs.
- RDD: off (global) at feature start.

## Tasks

### Phase R — Rust core

- [x] R0 Workspace skeleton (`Cargo.toml`, `crates/`), CI job (fmt, clippy,
  test), `ROCKET_BIN` override in Go e2e harness.
- [x] R1 `rocket-domain` + ports traits (port `internal/domain`, `internal/ports`).
- [ ] R2 `rocket-manifest` (strict YAML, dotenv, JSON Schema) with
  `testdata/manifests` golden tests.
- [ ] R3 Adapters: paths, sqlite (rusqlite, WAL, compatible JSON), events bus,
  logs follower.
- [ ] R4 Adapters: process (pgid via nix), probe (TCP/HTTP/lsof), task, compose.
- [ ] R5 App use cases: up/down/restart/status/reconcile/gc (port app tests).
- [ ] R6 App jobs: run/setup/deploy/cancel/TTL/job logs.
- [ ] R7 API server (axum over unix socket + token TCP, SSE), daemon
  composition root, `daemon.json`, flock.
- [ ] R8 `rocket-client` crate + CLI (clap) with identical tree, exit codes, JSON.
- [ ] R9 Scaffold (`rocket init`) + agentdocs.
- [ ] R10 Parity: Go e2e suite green against Rust binary.
- [ ] R11 Release: goreleaser/cargo build, Homebrew cask.
- [ ] R12 Remove Go code; update AGENTS.md/README.

### Phase T — Tauri desktop

- [ ] T0 Scaffold `apps/desktop`: Tauri v2, React, Vite, TS, Tailwind, shadcn.
- [ ] T1 Tauri backend: `rocket-client` commands, SSE → frontend events,
  daemon auto-start, 401 token refresh.
- [ ] T2 XEAN theme, shell layout, sidebar, navigation, shortcuts.
- [ ] T3 Project detail, services table, inspector, logs.
- [ ] T4 Ports, Jobs (log follow, stale-snapshot guard), Owners.
- [ ] T5 Settings, tray, deploy confirmation, app menus.
- [ ] T6 Packaging (.app/.dmg), cask, remove `macos/`.

## Route log

| Task | Route | Trigger evidence | Commit | Review |
|---|---|---|---|---|
| R0 | delegated (Sonnet) | 6 files, CI + harness | 5af21ff | RDD off |
| R1 | delegated (Sonnet) | ~800 Go lines → crate + tests | 89d2434 | RDD off |

## Progress

- 2026-10-06: R0+R1 done: workspace (tokio, axum, rusqlite bundled, nix,
  time, async-trait for dyn ports), domain crate with Go-JSON byte-compat
  tests (36 tests green); Go e2e passes with ROCKET_BIN override.
- 2026-10-06: Explorer mapped Go (~8.3k src lines) and Swift app (~4.1k src).
  Branch `feat/rust-tauri-port` created.

## Next step

Execution order: R8a (`rocket-client` crate only) → T0–T5 against the
current Go daemon (same contract) → R2–R7, R8b (CLI), R9 → R10 parity → R11,
R12, T6. Next: R8a.
