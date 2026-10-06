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
  well above the 400-line budget; single PR by user choice.
- RDD: off (global) at feature start.

## Tasks

### Phase R — Rust core

- [x] R0 Workspace skeleton (`Cargo.toml`, `crates/`), CI job (fmt, clippy,
  test), `ROCKET_BIN` override in Go e2e harness.
- [x] R1 `rocket-domain` + ports traits (port `internal/domain`, `internal/ports`).
- [x] R2 `rocket-manifest` (strict YAML, dotenv, JSON Schema) with
  `testdata/manifests` golden tests.
- [x] R3 Adapters: paths, sqlite (rusqlite, WAL, compatible JSON), events bus,
  logs follower.
- [x] R4 Adapters: process (pgid via nix), probe (TCP/HTTP/lsof), task, compose.
- [x] R5 App use cases: up/down/restart/status/reconcile/gc (port app tests).
- [x] R6 App jobs: run/setup/deploy/cancel/TTL/job logs.
- [x] R7 API server (axum over unix socket + token TCP, SSE), daemon
  composition root, `daemon.json`, flock.
- [x] R8 (R8a done: `rocket-client` crate) CLI (clap) with identical tree, exit codes, JSON.
- [x] R9 Scaffold (`rocket init`) + agentdocs.
- [x] R10 Parity: Go e2e suite green against Rust binary.
- [x] R11 Release: goreleaser/cargo build, Homebrew cask.
- [x] R12 Remove Go code; update AGENTS.md/README.

### Phase T — Tauri desktop

- [x] T0 Scaffold `apps/desktop`: Tauri v2, React, Vite, TS, Tailwind, shadcn.
- [x] T1 Tauri backend: `rocket-client` commands, SSE → frontend events,
  daemon auto-start, 401 token refresh.
- [x] T2 XEAN theme, shell layout, sidebar, navigation, shortcuts.
- [x] T3 Project detail, services table, inspector, logs.
- [x] T4 Ports, Jobs (log follow, stale-snapshot guard), Owners.
- [x] T5 Settings, tray, deploy confirmation, app menus.
- [x] T7 Auto-updater: tauri-plugin-updater with signed GitHub-release
  `latest.json`, check on launch + periodic + menu, install & relaunch,
  release workflow emits signed updater artifacts, cask `auto_updates true`.
  Requested 2026-10-06 after PR #1 opened; key at `~/.tauri/rocket-updater.key`
  (not in repo; user sets CI secret).
- [x] T6 Packaging (.app/.dmg), cask, remove `macos/`.

## Route log

| Task | Route | Trigger evidence | Commit | Review |
|---|---|---|---|---|
| R0 | delegated (Sonnet) | 6 files, CI + harness | 5af21ff | RDD off |
| R1 | delegated (Sonnet) | ~800 Go lines → crate + tests | 89d2434 | RDD off |
| R8a | delegated (Sonnet) | client + DTOs, 19 endpoints, SSE | 1e44e17 | RDD off |
| T0+T1 | delegated (Sonnet) | scaffold + bridge, many files | 9e87ae3 | RDD off |
| T2+T3 | delegated (Sonnet) | UI multi-file | 6636070 | RDD off |
| T4+T5 | delegated (Sonnet) | UI + tray/menu multi-file | 4dd8f63 | RDD off |
| R2 | delegated (Sonnet) | yaml.v3 port, 313-case Go oracle | c595f78 | RDD off |
| R3+R4 | delegated (Sonnet) | 7 adapters, Go state.db fixture | 413fcd6 | RDD off |
| R5 | delegated (Sonnet) | 2.4k Go app lines, 43 tests | e1dbe64 | RDD off |
| R6 | delegated (Sonnet) | jobs.go; no RED observed, parent mutation check caught | c5d50a6 | RDD off |
| R7 | delegated (Sonnet) | api + daemon, 36 Go goldens | 8f6d739 | RDD off |
| R8b+R9 | delegated (Sonnet) + parent version fix | CLI, scaffold, agentdocs | 648bb74 | RDD off |
| R11+R12 | delegated (Sonnet) | e2e port, goreleaser rust, Go removal | 166063c, 47ee958, d3f9a5b | RDD off |
| T6 | delegated (Sonnet) + parent README fix | sidecar, dmg release, app cask, macos/ removal | 4c94bdf | RDD off |

## Progress

- 2026-10-06: Released v1.0.0. PR #1 merged (e83d5ff), tag v1.0.0. Release
  assets: CLI darwin/linux amd64+arm64, checksums, Rocket_1.0.0_universal.dmg,
  Rocket.app.tar.gz + .sig, latest.json (1.0.0, darwin-aarch64/x86_64). Casks
  updated by CI (376051b, 963ecc9). Verified: /releases/latest/download/
  latest.json resolves to 1.0.0; darwin_arm64 checksum OK; `rocket version
  1.0.0`. Desktop job needed one rerun (secret added after tag push).

- 2026-10-06: CI fixes on PR #1: clippy `manual_chunks` on Rust 1.99
  (d2d0ad5); daemon e2e gated behind `e2e` feature (macOS runner http.server
  slowness, same policy as Go) and script fixtures written from a child
  process to avoid ETXTBSY (43c3421). Version bumped to 1.0.0 (be51bed) for
  the v1.0.0 launch; tag after merge.

- 2026-10-06: T7 done (delegated, Sonnet): tauri-plugin-updater 2.13.1,
  signed latest.json on GitHub releases, check 10s after launch + every 6h,
  menu/tray/Settings, stale-daemon notice. Local smoke: 0.1.1 app found
  0.1.99, signature verified; tampered sig and archive rejected. Pending: user
  adds `TAURI_SIGNING_PRIVATE_KEY` repo secret; real release not run.

- 2026-10-06: All tasks done. Rust e2e suite (`cargo test -p rocket-cli
  --features e2e`) 7/7 green with mutation check; goreleaser snapshot built
  darwin/linux amd64+arm64; Windows archives dropped (bundled sqlite cannot
  cross-compile; process adapter was a stub). Desktop .app + aarch64 dmg built
  locally; sidecar fallback proven under sandbox-exec. Not run: release
  workflow, cask commit, `brew install --cask`, universal dmg, Linux desktop.

- 2026-10-06: Rust core R2–R10 done. Parity gate: `ROCKET_BIN=target/debug/rocket
  go test -tags integration ./cmd/rocket/` → ok (24.4s, all 8 e2e tests);
  `cargo test --workspace` green. Known deviations: `--json` prints `[]`
  where Go printed `null` for nil slices; clap usage-error wording differs;
  no `completion` command; JSON `<>&` escaping matched per Go encoder.

- 2026-10-06: Desktop T0–T5 done against Go daemon: Tauri 2.12, React 19,
  Vite 8, Tailwind 4.3, shadcn 4 (base-ui), react-query, zustand, ts-rs
  bindings; tray + native menus; 74 vitest + 19 Rust tests. Screenshots in
  `.verification/` (gitignored). Not hand-verified: tray/menu clicks.

- 2026-10-06: R0+R1 done: workspace (tokio, axum, rusqlite bundled, nix,
  time, async-trait for dyn ports), domain crate with Go-JSON byte-compat
  tests (36 tests green); Go e2e passes with ROCKET_BIN override.
- 2026-10-06: Explorer mapped Go (~8.3k src lines) and Swift app (~4.1k src).
  Branch `feat/rust-tauri-port` created.

## Next step

None; feature shipped in v1.0.0. Follow-ups: `--json` nil-slice `null`
parity decision; Linux desktop build; Windows support.
