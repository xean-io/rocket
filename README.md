# rocket

One owner for every dev process across your projects.

`rocket` is a CLI plus a daemon (`rocketd`, same binary) that starts, tracks and
stops dev services, compose containers, test/CI jobs and deploys for all your
projects. It wraps your existing `Taskfile.yml` and compose files through a small
`rocket.yaml`; nothing is migrated. Humans and AI agents use the same commands,
so nobody leaves orphan processes holding ports.

- **No orphans:** every process runs under the daemon in its own process group;
  `down`, TTLs and `gc` clean up by owner.
- **No port fights:** global port leases across projects; a busy port is remapped
  and injected through the env var the service already reads (`${PORT:-3000}`).
- **Agent-safe:** owners (`ROCKET_OWNER=agent:<id>`), `--ttl`, `--json` everywhere,
  and a deploy gate agents cannot pass alone.

## Install

```sh
# Homebrew (macOS / Linux)
brew tap xean-io/rocket https://github.com/xean-io/rocket
brew install --cask xean-io/rocket/rocket

# or build from source (Rust stable, see rust-toolchain.toml)
git clone https://github.com/xean-io/rocket && cd rocket
cargo install --locked --path crates/rocket-cli

rocket daemon start          # optional: any command auto-starts the daemon
```

Prebuilt archives for macOS and Linux (amd64/arm64) are on the
[releases page](https://github.com/xean-io/rocket/releases). Windows is not
shipped yet.

## Desktop app

Rocket.app (Tauri + React, in `apps/desktop`) is a window, menu bar item and
native menu over the same daemon API as the CLI. It bundles the `rocket` CLI as
a sidecar, so it works on its own; an installed `rocket` on `PATH` (or
`$ROCKET_BIN`) takes priority and the bundled one is the last fallback, so the
CLI and the app share one daemon.

```sh
# Homebrew (macOS, universal)
brew tap xean-io/rocket https://github.com/xean-io/rocket
brew install --cask xean-io/rocket/rocket-app
```

Or download `Rocket_<version>_universal.dmg` from the
[releases page](https://github.com/xean-io/rocket/releases) and drag Rocket to
Applications. The app is ad-hoc signed, not notarized: the cask clears the
quarantine flag; after a manual dmg install run
`xattr -dr com.apple.quarantine /Applications/Rocket.app` (or right-click >
Open). `rocket app` opens it from the terminal.

Build from source (Rust, Node 24, pnpm 10):

```sh
cd apps/desktop
pnpm install --frozen-lockfile
pnpm tauri dev                          # hot-reloading dev app
pnpm tauri build --bundles app,dmg      # target/release/bundle/{macos,dmg}
```

`pnpm tauri build` first runs `scripts/prepare-sidecar.mjs`, which compiles
`rocket-cli` and places it in `src-tauri/binaries/` for Tauri's `externalBin`.

### Updates

The app updates itself. About 10 seconds after launch, and then every 6 hours
while it runs, it reads `latest.json` from the newest GitHub release and, when
a newer version exists, shows "Rocket X is available" with the release notes,
**Install and Restart** and **Later** (Later hides that version until the next
launch). The download is verified against the updater public key embedded in the
app before anything is installed, then Rocket replaces itself and relaunches.
Automatic checks are quiet (offline is silent) and can be turned off in
Settings > Updates, which also has **Check now**. **Rocket > Check for
Updates...** and the menu bar item do the same, and always answer. Debug builds
never check automatically.

An update also replaces the bundled `rocket` CLI. A daemon that is already
running keeps the old version until you restart it; when the app sees that its
daemon was started from the bundled CLI and is older, Settings shows "Daemon is
running X; restart it to use Y" next to **Restart Daemon...**. Rocket never
restarts the daemon on its own. Homebrew installs the cask with `auto_updates`,
so `brew upgrade` leaves the app to update itself.

For testing, `ROCKET_UPDATER_ENDPOINT` replaces the manifest URL (signatures are
still verified; plain `http` is only accepted by builds that opt in through
`plugins.updater.dangerousInsecureTransportProtocol`).
`ROCKET_UPDATER_SMOKE=download` is a test-only hook: it checks, downloads and
verifies the signature, logs the result and exits; it never installs.

### Building and testing

```sh
cargo build -p rocket-cli                     # target/debug/rocket
cargo test --workspace                        # unit tests: hermetic, no daemon or network
cargo test -p rocket-cli --features e2e       # end-to-end: real daemon and processes (needs python3)
```

The desktop app is covered in [Desktop app](#desktop-app) below.

### Releasing

Push a `vX.Y.Z` tag; the `release` workflow builds the Rust binary for macOS and
Linux with cargo-zigbuild, runs GoReleaser, publishes the GitHub release and
commits the updated cask to `Casks/rocket.rb` on `main` (this repo is its own
Homebrew tap). The same workflow then builds the universal desktop dmg, uploads
it to the release as `Rocket_<version>_universal.dmg` and commits
`Casks/rocket-app.rb`. It also uploads the signed updater archive
(`Rocket.app.tar.gz` and its `.sig`) and the `latest.json` the app reads; the job
fails fast without the `TAURI_SIGNING_PRIVATE_KEY` secret (see AGENTS.md). Dry run locally (needs rustup, zig and cargo-zigbuild) with
`goreleaser release --snapshot --clean --skip=publish`.

State lives in `~/.rocket` (override with `ROCKET_HOME`).

## Quick start

```sh
cd ~/code/my-app
rocket init                  # detect Taskfile, compose files, .env -> rocket.yaml
rocket up                    # start everything (dependencies first, health-checked)
rocket status                # services, ports, running jobs, port conflicts
rocket run test              # pipeline, or any Taskfile task as a fallback
rocket down --all
```

## rocket.yaml

```yaml
version: 1
name: shop                       # registry name; default: directory name
dotenv: [.env]                   # merged into every service and job (never exported globally)
default_env: dev                 # default: dev
envs:
  dev:   { compose: [compose.yaml] }
  smoke: { compose: [docker-compose.prod.yml], profiles: [release] }
  prod:  { deploy: { task: "release:ci", confirm: true } }   # or deploy: { run: "...", confirm: true }
setup:                           # rocket setup = doctor -> install -> migrate (undefined ones skipped)
  install: { task: install }
  migrate: { task: "db:migrate" }
services:
  postgres: { compose: postgres, profiles: [deps], ports: { main: { default: 5432, env: POSTGRES_PORT } } }
  api:
    run: "pnpm dev"              # or task: dev  (long-running), or compose: <service>
    cwd: apps/api
    depends_on: [postgres]
    env: { NODE_ENV: development }
    ports: { http: { default: 3000, env: PORT } }   # env = how rocket injects a remapped port
    health: { http: /health, timeout: 60s }         # or tcp: true; default: TCP on the first eligible port
groups:
  core: [postgres, api]
  all: ["*"]
pipelines:                       # sequential steps, stop at the first failure
  test: [{ task: test }]
  ci:   [{ task: lint }, { run: "pnpm test" }]
```

`rocket schema` prints the JSON Schema. Compose projects are always named
`rocket-<project>-<env>`, so two files sharing a `name:` no longer share volumes.
Rocket forces that same `COMPOSE_PROJECT_NAME` in service and job environments
after all inherited, dotenv, service and port values. Taskfile `docker compose`
commands therefore use Rocket's project without a manual override. Setup and
pipeline jobs default to `default_env`; deployments use their target environment.

Port `env` also accepts templates for services that read a host and port or URL:

```yaml
ports:
  http:
    default: 8080
    env:
      API_BIND: "0.0.0.0:{port}"
      CREATIVE_PUBLIC_BASE_URL: { default: "http://127.0.0.1:{port}" }
```

Rocket substitutes the resolved port after leasing/remapping. Plain template
bindings always override the inherited, dotenv and service environment; a
`default` binding applies only when the merged value is empty. Every template
must contain `{port}`. A busy port can be remapped only when it has an
unconditional binding (or the legacy scalar `env: PORT`); default-only bindings
do not authorize remapping. No shell wrapper is needed for host/port injection.

A port may declare `probe: false` to reserve/remap it without using it for
Rocket readiness checks (for example, a debug port that opens only on demand).
The default is `true`. Readiness uses the first eligible port in alphabetical
order, or an explicit `health.port`; an explicit disabled or undeclared target
is invalid. With all ports disabled, local services use the short start grace
and Compose services still use `docker compose up --wait` for their own checks.

Service environment values can reference another service's resolved port:

```yaml
web:
  task: web:dev
  env:
    API_URL: "http://127.0.0.1:{api.http}"
```

The reference adds `api` as a startup dependency and uses its live port after
remapping in the selected environment. Unknown services/ports and dependency
cycles fail manifest validation. The last dot separates the service name from
the port name; tokens contain no whitespace. Shell `${...}` expressions remain
literal. Already-running services must use the selected environment. Repeating
`up` preserves an existing consumer's environment; restart it to pick up a
provider's changed port.

Pipelines can start prerequisite services or groups before running their steps:

```yaml
pipelines:
  test:e2e:
    needs: [infra, api]
    steps:
      - task: test:e2e
```

The existing step-array form remains supported. Prerequisites use normal
dependency ordering and readiness checks, the job owner and selected environment.
A failed prerequisite prevents every step. Existing services keep their owner
and TTL; newly started services inherit the job's absolute deadline when supplied.
Ready prerequisites remain running after the job finishes or is canceled.
Cancellation stops an interrupted startup; whole-project `down --all` also cleans
tracked failed Compose attempts without removing volumes.

## CLI

Every command accepts `--json` and `-p <project name|path>` (default: the
`rocket.yaml` found from the current directory). Exit codes: `0` ok, `1` error,
`2` partial failure or failed job, `3` confirmation required. JSON errors are
`{"error": "...", "code": "..."}`.

| Command | What it does |
|---|---|
| `rocket init [--force] [--print]` | scaffold `rocket.yaml` from Taskfile/compose/.env |
| `rocket up [svc\|group…] [--env E] [--profile P] [--owner O] [--ttl 30m]` | start (idempotent), dependencies first |
| `rocket down [svc…\|--all\|--owner O\|--everywhere]` | stop in reverse order; whole-project/owner/everywhere also cancel running jobs |
| `rocket restart <svc…> [--env E] [--profile P]` | stop then start |
| `rocket ps [--all-projects]` · `rocket status` | service table · one-shot summary with jobs and port conflicts |
| `rocket logs <svc> [-f] [--tail N]` | service output |
| `rocket ports` · `rocket gc` | global port map · reconcile, expire TTLs, prune |
| `rocket run <pipeline\|task> [--env E] [--profile P] [--detach] [--ttl 30m] [-- args]` | run a job; blocks and streams by default |
| `rocket setup [name]` · `rocket doctor\|install\|migrate` `[--env E] [--profile P] [--ttl 30m]` | setup jobs |
| `rocket deploy <env> [--yes] [--ttl 30m]` | deploy job; `confirm: true` needs `--yes` |
| `rocket jobs [--all-projects]` · `rocket job <id> [logs [-f]]` · `rocket job cancel <id>` | job history, logs, cancel |
| `rocket projects add\|ls\|rm` | global project registry |
| `rocket daemon start\|stop\|status\|run` | manage `rocketd` |
| `rocket agent install [--target claude\|agents\|both] [--global] [--print]` | teach AI agents to use rocket |
| `rocket app` | open Rocket.app (macOS, `$ROCKET_APP` overrides the app name or path) |
| `rocket schema` | JSON Schema of `rocket.yaml` |

All job commands accept a positive Go duration in `--ttl`. The deadline starts
when the job is persisted and includes prerequisite startup and every step.
Expiry cancels the job with error `ttl expired`; blocking commands return exit
code 2. Deadlines survive daemon restart, including jobs interrupted before their
first step. An adopted running step keeps its original deadline. `gc` reports
`expired_job` with the job ID in `detail`; ready prerequisites follow their own
persisted deadlines. Job log following sends a heartbeat every 15 seconds and
drains trailing output before one terminal event.

Setup and pipeline jobs default to `default_env`; select another declared
environment with `--env`. Repeat `--profile` to enable prerequisite startup
profiles. Jobs persist the sorted union of the environment and requested
profiles for history and reruns. Pipeline `needs` wildcards use that selection;
explicit prerequisites and their required dependencies remain selectable.
Step environment merging keeps its existing behavior, with forced
`COMPOSE_PROJECT_NAME` and inherited/dotenv `COMPOSE_PROFILES`. Deployment targets
select their own environment and do not accept the new profile flags.

`rocket status --json` includes sorted `project.envs`, `project.pipelines` and
`project.deploy_envs` name arrays; empty lists are `[]`. Pipelines are declared
manifest pipelines, and deployment environments are those with a configured
`deploy`. Running `down --all` outside a project suggests `--everywhere` while
retaining the normal error and exit code 1; explicit invalid project selections
retain their project-specific error.

`rocket init` maps a root Taskfile task named `setup` to `setup.install` when
there is no explicit `install` task. When both exist, `install` wins. Root
`setup` is not generated as a pipeline; namespaced tasks keep their existing
classification.

Startup wildcards (no targets, `*`, or a group containing `*`) include a gated
service only when one of its `profiles` is enabled by the selected environment
or a requested profile. Repeat `--profile` to enable multiple profiles, for
example `rocket up all --profile trends --profile reports`. Explicit service
and group members, plus required dependencies, remain selectable. Compose
receives the sorted, unique environment/request/service profiles. `restart`
uses the same rules; `down --all`, wildcard stops and `--everywhere` stop every
selected active service regardless of profiles.

When a Compose service creates named data volumes, `up` prints a first-run hint
for configured `setup.migrate` and `setup.assets` actions. Hint commands include
`-p <project>` so they work outside its directory, and `--env <environment>` for
a nondefault selection. Rocket compares successful volume inventories before
and after startup using Compose's normalized service mounts. Existing,
external, bind and anonymous mounts do not trigger the hint; inspection errors
provide no creation evidence. Volumes remain intact when services stop.

`rocket run --json` returns `{job, status, exit_code, duration_ms, log_path, tail}`
(last 50 log lines). The daemon API (unix socket, plus a token-protected TCP
listener for the desktop app) is documented in
[`crates/rocket-api/README.md`](crates/rocket-api/README.md).

## AI agents

`rocket agent install` writes a Claude Code skill (`.claude/skills/rocket/SKILL.md`,
or `~/.claude/skills` with `--global`) and a marked block in the project's
`AGENTS.md`/`CLAUDE.md` (re-running updates it in place). The rules it teaches:

- use rocket for every process, service, test and CI operation — never `&`,
  `nohup`, `docker compose up` or `task dev` directly;
- `export ROCKET_OWNER=agent:<session-id>`; pass `--ttl` for exploratory runs;
- `rocket run <targeted pipeline>` instead of full suites; always `--json`;
- `rocket down --owner agent:<session-id>` before finishing.

Deploys by an `agent:*` owner are refused unless a human passes `--yes` **and**
sets `ROCKET_ALLOW_DEPLOY=1` — a guardrail against accidents, not a security
boundary.

## Platforms

macOS and Linux are supported. Windows builds, but process supervision is not
implemented yet (the process adapter returns "unsupported"). The desktop app
is built and released for macOS.
