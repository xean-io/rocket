# AGENTS.md — working on rocket itself

rocket is a Rust CLI + daemon (one binary, `rocket`) that supervises dev
processes, plus a Tauri desktop app in `apps/desktop`. Design: `odd/plan.md`;
progress and evidence: `odd/tasks/rust-tauri-port.md`; daemon API contract:
`crates/rocket-api/README.md`.

## Build and test

```sh
cargo build -p rocket-cli                          # target/debug/rocket
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                             # unit tests: hermetic, no daemon or network
cargo test -p rocket-cli --features e2e            # e2e: real daemon + processes (needs python3, free ports 18431-18433/18531)
cargo test -p rocket-adapters -- --ignored         # Docker-backed compose tests (need a Docker daemon)
```

Run targeted tests while iterating (`cargo test -p rocket-app deploy_gate`).

Desktop app (`apps/desktop`, run from that directory): `pnpm install --frozen-lockfile`,
then `pnpm typecheck`, `pnpm lint`, `pnpm test --run`, `pnpm build`. The Tauri crate
(`apps/desktop/src-tauri`, package `rocket-desktop`) embeds the built frontend, so run
`pnpm build` before compiling it. On Linux it also needs the webkit2gtk system
packages; CI checks it on macOS only. `src-tauri/build.rs` writes an empty
placeholder for the `rocket` sidecar so `cargo check/test` work on a fresh clone;
`pnpm tauri build|dev` always rebuilds the real one (`scripts/prepare-sidecar.mjs`).
Package it with `pnpm tauri build --bundles app,dmg`.

## Layout (hexagonal)

| Path | Role |
|---|---|
| `crates/rocket-domain` | pure model: projects, services, runs, jobs, events, dependency graph, and the port traits (`ports`) the core depends on |
| `crates/rocket-manifest` | `rocket.yaml` load/validate/JSON Schema, dotenv |
| `crates/rocket-app` | use cases (up/down/status/jobs/summary/reconcile/gc); tested with the fakes in `src/testing` |
| `crates/rocket-adapters` | process (pgid), compose, task, sqlite, probe, logs, events |
| `crates/rocket-api` | HTTP+SSE API (axum); the contract is `crates/rocket-api/README.md` |
| `crates/rocket-daemon` | composition root: unix socket + token-protected TCP listener, `daemon.json` |
| `crates/rocket-client` | typed API client, daemon auto-start |
| `crates/rocket-scaffold` | `rocket init` detection + rendering |
| `crates/rocket-agentdocs` | `rocket agent install` content + idempotent block upsert |
| `crates/rocket-cli` | clap CLI (`rocket` binary); `tests/e2e` drives the built binary (feature `e2e`) |
| `apps/desktop` | Tauri v2 + React app on top of `rocket-client`; bundles the `rocket` CLI as a sidecar (`scripts/prepare-sidecar.mjs`) |
| `Casks/` | Homebrew casks: `rocket.rb` (CLI, GoReleaser) and `rocket-app.rb` (desktop dmg, release workflow) |
| `testdata/` | fixtures only (`fixture/` for e2e, `init/` for `rocket init`, `manifests/`) |

## Rules

- Test first for app/domain logic: write the failing test, then the code.
- E2e tests live behind the `e2e` cargo feature so `cargo test --workspace` stays
  fast and hermetic. They use a short temp `ROCKET_HOME` under `/tmp` (unix socket
  paths are limited to ~104 bytes) and always clean up the daemon, even on failure.
- Never touch the real `~/.rocket` or `~/.claude` in tests: set `ROCKET_HOME` /
  `HOME` to temp dirs. Never signal processes rocket did not start.
- Keep `--json` shapes stable and documented in `crates/rocket-api/README.md`; exit
  codes are 0 ok, 1 error, 2 partial/job failure, 3 confirmation required.
- Code, comments and docs in English.
