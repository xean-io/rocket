# rocketd HTTP API (v1)

rocketd is the single supervisor. The CLI, AI agents (`rocket … --json`) and the
macOS app all talk to it through this API. Contract version: `v1` (reported by
`GET /v1/health` as `"api": "v1"`).

## Transport

| Item | Value |
|---|---|
| Socket | `$ROCKET_HOME/rocketd.sock` (default `~/.rocket/rocketd.sock`), mode `0600` |
| Protocol | HTTP/1.1 over the unix socket; any `Host` works (the CLI uses `http://rocketd`) |
| Bodies | JSON, `Content-Type: application/json`; unknown request fields are rejected |
| Streams | Server-Sent Events (`text/event-stream`) |
| TCP | `http://127.0.0.1:<random port>`, same API, every request needs `Authorization: Bearer <token>`; address and token are in `$ROCKET_HOME/daemon.json` (mode `0600`) — see [macOS app client contract](#macos-app-client-contract) |
| Other files | `state.db` (SQLite), `logs/<project>/<service>.log`, `logs/<project>/jobs/<job-id>.log`, `rocketd.pid`, `rocketd.log`, `rocketd.lock`, `daemon.json` under `$ROCKET_HOME` |

The unix socket is token-free (its `0600` mode is the access control); the TCP
listener exists for GUI clients that cannot speak unix sockets comfortably.

```sh
curl --unix-socket ~/.rocket/rocketd.sock http://rocketd/v1/health
curl --unix-socket ~/.rocket/rocketd.sock -N 'http://rocketd/v1/events?types=service.state'
```

## Errors

Every non-2xx response has this body:

```json
{ "error": "invalid request: unknown service or group \"nope\" in project shop", "code": "invalid" }
```

| HTTP | `code` | Meaning |
|---|---|---|
| 400 | `invalid` | bad body, unknown service/group/env, invalid ttl, invalid rocket.yaml |
| 401 | `unauthorized` | TCP listener only: missing or wrong bearer token |
| 404 | `not_found` | unknown project name, service or job id |
| 409 | `conflict` | project name already registered elsewhere, project still running |
| 428 | `confirmation_required` | deploy refused: `confirm: true` without `yes`, or agent owner without `yes` + `allow_agent_deploy` (the CLI exits 3) |
| 500 | `internal` | anything else |

A partial failure inside `up` (one service failed) is **not** an HTTP error: it is
`200` with `"action": "failed"` / `"skipped"` on the affected services.

## Projects

`project` fields/params accept either an **absolute path** to the directory
holding `rocket.yaml` (the project is auto-registered on first use) or a
**registered name**.

## Endpoints

| Method & path | Request | Response |
|---|---|---|
| `GET /v1/health` | – | `HealthInfo` |
| `POST /v1/up` | `UpRequest` | `UpResult` |
| `POST /v1/restart` | `UpRequest` | `UpResult` (stop then start) |
| `POST /v1/down` | `DownRequest` | `DownResult` |
| `GET /v1/ps?project=&all=true` | query | `StatusResult` |
| `GET /v1/logs?project=&service=&tail=100` | query | `LogsResult` |
| `GET /v1/logs?…&follow=true` | query | SSE of `log.line` events (tail first, then live) |
| `GET /v1/ports` | – | `PortsResult` |
| `POST /v1/gc` | – | `GCResult` |
| `GET /v1/projects` | – | `{"projects": [ProjectRef]}` |
| `POST /v1/projects` | `{"path": "/abs/dir"}` | `ProjectRef` |
| `DELETE /v1/projects/{name}` | – | `{"removed": "name"}` |
| `GET /v1/status?project=` | query | `Summary` (one-shot overview for agents) |
| `POST /v1/jobs` | `JobRequest` | `Job` (status `running`; persisted and returned before asynchronous prerequisite startup) |
| `GET /v1/jobs?project=&all=true&limit=50` | query | `{"jobs": [Job]}` newest first |
| `GET /v1/jobs/{id}` | – | `Job` |
| `GET /v1/jobs/{id}?wait=true` | – | `Job`, blocks until the job is terminal |
| `GET /v1/jobs/{id}/logs?tail=100` | query | `JobLogsResult` |
| `GET /v1/jobs/{id}/logs?follow=true[&tail=N]` | query | SSE of `job.log` (whole log, or last N lines, then live) ending with one `job.state`; the server closes the stream after it |
| `POST /v1/jobs/{id}/cancel` | – | `Job` after its process group stopped (no-op when finished) |
| `GET /v1/events?project=&service=&job=&types=a,b` | query | SSE stream of `Event` |
| `POST /v1/shutdown` | – | `{"ok": true, "pid": 123}` then the daemon exits |

`up`, `restart`, `down`, `gc`, job start and cancel run to completion even if
the client disconnects. `up` blocks until every service is healthy or failed.

## Shapes

### HealthInfo

```json
{ "ok": true, "api": "v1", "version": "0.1.0-dev", "pid": 4242,
  "started_at": "2026-10-05T00:14:10Z", "home": "/Users/me/.rocket",
  "socket": "/Users/me/.rocket/rocketd.sock", "http": "http://127.0.0.1:53124" }
```

`http` is the token-protected TCP listener (omitted when it could not bind).

### UpRequest

```json
{ "project": "/Users/me/code/nuvara", "services": ["core"], "env": "dev",
  "profiles": ["trends"], "owner": "agent:claude-123", "ttl": "30m" }
```

`services` takes services, groups or `"*"`; empty means a wildcard selection.
Wildcard members exclude profile-gated services unless one of their profiles
is enabled by the selected environment or the optional `profiles` list.
Explicit service/group members and dependencies remain selectable regardless
of profiles. Compose receives the sorted, unique union of environment,
requested and service profiles. `restart` uses the same selection and rejects
unknown environments or invalid TTLs before stopping services. `env` defaults
to the project's `default_env`; `owner` defaults to `"user"`; `ttl` is a Go
duration (optional).

### UpResult

Services are listed in start order (dependencies first).

Optional `hints: [string]` accompanies confirmed new Compose volumes when
`setup.migrate` or `setup.assets` is configured. Names are deduplicated and
sorted; hints recommend `rocket migrate` / `rocket setup assets` when their
prerequisites are ready. Commands target the registered project with `-p` and
include `--env` when startup selected a nondefault environment; shell arguments
are quoted as needed. Rocket uses normalized, selected-service mounts and
successful exact-name Docker inventories before and after startup. Existing,
external, bind and anonymous mounts are excluded; config/inventory errors never
imply creation. A failed startup may return a hint only when after-startup
evidence confirms the volume exists. Idempotent `up` does not repeat it.
Config, inventory and startup commands share the target environment and working
directory, including any selected Docker endpoint/context.

```json
{
  "project": "nuvara",
  "env": "dev",
  "services": [
    { "service": "api", "action": "already_running", "state": "running", "health": "healthy",
      "pid": 8635, "ports": { "http": 3002 }, "owner": "user",
      "log_path": "/Users/me/.rocket/logs/nuvara/api.log" },
    { "service": "web", "action": "started", "state": "running", "health": "healthy",
      "pid": 8714, "ports": { "http": 3100 }, "owner": "agent:claude-123",
      "expires_at": "2026-10-05T00:44:10Z",
      "remaps": [ { "name": "http", "from": 3000, "to": 3100, "env": "PORT",
                    "holder": { "pid": 77, "command": "node", "cwd": "/Users/me/code/shop" },
                    "reason": "in use by node (pid 77) in /Users/me/code/shop" } ],
      "log_path": "/Users/me/.rocket/logs/nuvara/web.log" },
    { "service": "causation", "action": "failed", "state": "failed", "health": "unhealthy",
      "error": "port 3003 (http) for nuvara/causation is busy: …" }
  ]
}
```

`action`: `started` | `already_running` | `failed` | `skipped` (a dependency failed).

Manifest ports support a scalar variable name (`env: PORT`) or a mapping of
variables to templates (`env: {API_BIND: "0.0.0.0:{port}"}`). All templates
contain `{port}`, replaced with the resolved port before startup. Mapping values
may instead be `{default: "http://127.0.0.1:{port}"}` to preserve a nonempty
inherited/dotenv/service value. Remapping requires an unconditional binding.
The wire fields `remaps[].env` and `conflicts[].env` remain strings: for a mapping
they report the alphabetically first unconditional variable; default-only
bindings have no representative and are not remappable. Environment values and
binding templates are not returned in these fields.

Manifest `ports.<name>.probe` is an optional boolean, defaulting to `true`.
`false` retains leasing, remapping and environment injection, but excludes that
port from Rocket readiness checks. The default health target is the first
eligible port by name. An explicit `health.port` must name a declared port with
probing enabled. With no eligible port, local services use the short start
grace; Compose's `up --wait` still runs its own readiness checks. Run/port/remap
JSON shapes are unchanged.

Service `env` values may contain `{service.port}` references. These add implicit
dependencies and expand from the provider's live resolved port in the requested
environment before consumer startup. Explicit dependencies remain supported and
are deduplicated against references. Unknown services/ports and cycles are
manifest errors. Tokens contain no whitespace and split at the last dot; shell
`${...}` expressions remain literal. A failed provider skips its consumer.
Before starting any service, `up` rejects a live service in another environment
with `invalid` (HTTP 400). Same-environment consumers remain idempotent; a
restart is required to refresh their environment after a provider port change.

### DownRequest / DownResult

```json
{ "project": "/Users/me/code/nuvara", "services": ["web"], "owner": "agent:x", "everywhere": false }
```

- no `services` and no `owner` → whole project (compose projects are also `down`ed)
- `owner` → only runs started by that owner
- `everywhere: true` → all projects (`project` ignored, `services` not allowed)

Stop selection never filters profiles; wildcard and whole-project/everywhere
operations include all active gated services. Compose runs persist optional
`profiles: [string]` metadata for reconstructing their launch selection during
stop. Older runs without that field use the manifest's environment/service
profiles.
- whole project, `owner` and `everywhere` also cancel matching **running jobs**
  (`canceled_jobs`); naming services never touches jobs

```json
{ "stopped": [ Run, … ], "compose_down": ["rocket-nuvara-dev"], "canceled_jobs": ["j3f9a0c12be"], "errors": [] }
```

### Run (used by ps, down, events)

```json
{
  "project": "rocket-fixture", "service": "static", "env": "dev", "kind": "run",
  "state": "running", "health": "healthy",
  "pid": 8635, "pgid": 8635,
  "compose_project": "", "container_id": "",
  "owner": "user", "expires_at": null,
  "started_at": "2026-10-05T00:14:10.938611Z", "stopped_at": null, "exit_code": null,
  "ports": { "http": 18431 },
  "log_path": "/Users/me/.rocket/logs/rocket-fixture/static.log",
  "error": ""
}
```

Fields are shown here for completeness; empty/zero values (`""`, `0`, `null`) are omitted on the wire. `kind`: `run` | `task` | `compose`.
`state`: `starting` | `running` | `stopping` | `stopped` | `exited` | `failed` | `dead`
(`dead` = found gone during reconcile). `health`: `unknown` | `healthy` | `unhealthy`
(services without ports are `healthy` once they survive a short start grace).

### StatusResult

`{"services": [Run, …]}`. With `project`, services declared in `rocket.yaml`
but never started appear with `"state": "stopped"`. Without `project` (or
`all=true`) every known run across projects is returned.

### LogsResult

```json
{ "project": "rocket-fixture", "service": "sleeper",
  "lines": ["=== rocket: starting sleeper (run, env dev, owner user) at 2026-10-05T00:14:11Z ===", "greeting=hello"] }
```

### PortsResult

```json
{ "ports": [ { "port": 18431, "project": "rocket-fixture", "service": "static", "port_name": "http",
               "created_at": "2026-10-05T00:14:10Z", "owner": "user", "state": "running",
               "pid": 8635, "env": "dev" } ] }
```

### GCResult

```json
{ "actions": [ { "action": "released_lease", "project": "ghost", "service": "x", "port": 9999 },
               { "action": "pruned", "project": "nuvara", "service": "web", "detail": "stopped" } ] }
```

`action`: `marked_dead` | `released_lease` | `expired` | `expired_job` (`detail` =
job id; `project` and `service` identify its project/name) | `stopped_orphan` |
`pruned` | `lost_job` (`detail` = job id) | `pruned_job` (finished jobs beyond the
newest 100 per project; log deleted). Failed Compose records with a tracked
project remain available for whole-project `down` to remove stopped containers;
GC prunes them after that cleanup completes.

### ProjectRef

```json
{ "name": "nuvara", "path": "/Users/me/code/nuvara", "added_at": "2026-10-05T00:14:10Z" }
```

### JobRequest

```json
{ "project": "/Users/me/code/nuvara", "kind": "pipeline", "name": "test:api", "env": "dev", "profiles": ["trends"], "args": ["-run", "TestLogin"],
  "owner": "agent:claude-123", "ttl": "30m", "yes": false, "allow_agent_deploy": false }
```

| `kind` | `name` | Steps |
|---|---|---|
| `pipeline` | pipeline name | `pipelines.<name>`; unknown names fall back to `task <name>` when the project has a Taskfile (else 400) |
| `setup` | `""` | `setup.doctor` → `setup.install` → `setup.migrate`, skipping undefined (400 when none) |
| `setup` | any setup key | `setup.<name>` |
| `deploy` | env name | `envs.<env>.deploy` (`task` or `run`) — gated, see below |

`args` are appended to the **last** step (`task <t> -- args…`, or `"$@"` for
`run` steps). Steps run sequentially in the project root with the project's
dotenv merged into the daemon environment (`ROCKET_*` never leaks); the first
non-zero exit stops the job.

For setup/pipeline jobs, `env` defaults to `default_env` and must name a declared
environment. Deploy jobs use `name` as their environment; a supplied `env` must
match it. Invalid environments are rejected before job persistence or startup.
Setup/pipeline requests also accept optional `profiles: [string]`. The job stores
the sorted, unique union of the selected environment's configured profiles and
the requested profiles. Pipeline prerequisite `up` uses that selection: wildcard
members filter gated services, while explicit members and required dependencies
remain selectable. Each Compose prerequisite additionally enables its own
service profiles. Existing live services keep their original profile selection.
Profiles select prerequisites; step child environments retain their normal
inherited/dotenv settings without a forced `COMPOSE_PROFILES` override. Setup
jobs record the selection even though they have no prerequisites. Deploy jobs
reject nonempty requested `profiles` with 400 `invalid`; their environment is
already selected by the deployment target. The CLI exposes `--env` and repeatable
`--profile` on `run`, `setup`, `doctor`, `install` and `migrate`.
All service and job children receive forced `COMPOSE_PROJECT_NAME` equal to
`rocket-<project>-<env>` after inherited/dotenv/service/port merging. Every job
step uses that same environment, so Taskfile Compose commands target Rocket's
project. The job owner remains metadata and is not injected into child env.

Manifest pipelines accept either a legacy step array or
`{needs: [service_or_group], steps: [...]}`. Before steps, prerequisites run
through normal `up` dependency ordering and readiness checks with the job's
owner/environment. Failures finish the job as `failed` with a prerequisite error
and no executed steps. Ready services remain running afterward; existing owners
and deadlines are unchanged. Cancellation interrupts queued startup, readiness
and Compose operations, stopping only the service whose startup was interrupted.
Whole-project `down` cancels jobs before selecting services and also removes
tracked failed Compose attempts; it never removes volumes.

`JobRequest.ttl` accepts a positive Go duration and persists optional
`Job.expires_at`, measured from job creation. Newly started prerequisites inherit
that exact absolute deadline, including time spent waiting for other services.
Expiry cancels queued/startup/step execution with status `canceled` and error
`ttl expired`. It runs independently of project startup locks and readiness
checks. The CLI exposes `--ttl` for pipeline, setup and deploy jobs; a blocking
expired job returns exit code 2. Expired persisted jobs are canceled on daemon
restart even when no step PID was saved; live adopted steps retain their original
deadline. An unexpired vanished step remains `lost`. Reconciliation returns an
optional `expired_jobs` list of job IDs alongside `lost_jobs`.

Deploy gate (checked before anything runs; refusal = 428
`confirmation_required`): an env with `confirm: true` needs `"yes": true`; an
owner starting with `agent:` additionally needs `"allow_agent_deploy": true`,
which the CLI sets only when `ROCKET_ALLOW_DEPLOY=1` is in the caller's
environment. This is a guardrail against accidents, not a security boundary.

### Job

```json
{
  "id": "j3f9a0c12be", "project": "nuvara", "name": "ci", "kind": "pipeline", "env": "dev", "profiles": ["base", "trends"], "owner": "agent:claude-123",
  "steps": [ { "task": "lint" }, { "run": "pnpm test" } ], "args": ["-v"],
  "status": "failed", "step": 2, "pid": 9120, "pgid": 9120, "exit_code": 1,
  "started_at": "2026-10-05T00:14:10Z", "finished_at": "2026-10-05T00:14:42Z", "duration_ms": 32011,
  "log_path": "/Users/me/.rocket/logs/nuvara/jobs/j3f9a0c12be.log",
  "error": "step 2 (run pnpm test) exited with code 1"
}
```

`status`: `running` | `succeeded` | `failed` | `canceled` | `lost` (the step's
process vanished while no daemon watched it, e.g. daemon crash; exit code
unknown). `env` is the selected environment for all new jobs; older history may
omit it. `duration_ms` is live while running.
`profiles` is an optional snapshot of the effective prerequisite startup
selection, suitable for resubmitting on a rerun; it is omitted for empty
selections and deployment jobs, and may be absent in older history.
`expires_at` is optional and uses an RFC3339 timestamp, like service TTLs.
`exit_code` is the last executed step's code (often 143 after cancellation;
children that handle SIGTERM can return a different code).
It is omitted when prerequisites fail or are canceled before any step executes.

Job log-follow streams send `: ping` comments every 15 seconds while running.
Heartbeat, log and state frames are serialized. At completion they drain all
remaining output, including a final line without a newline, then send one
terminal `job.state` event and close; no heartbeat or log follows that event.

### JobLogsResult

```json
{ "job": "j3f9a0c12be", "project": "nuvara", "lines": ["=== rocket: step 1/2: task lint ===", "…"] }
```

### JobOutcome (CLI only)

Blocking `rocket run|setup|doctor|install|migrate|deploy --json` prints this
(built from `GET /v1/jobs/{id}?wait=true` + the last 50 log lines). `exit_code`
is always present (`null` when unknown). The CLI exits 0 on `succeeded`, 2
otherwise.

```json
{ "job": "j3f9a0c12be", "project": "nuvara", "kind": "pipeline", "name": "ci", "status": "failed",
  "exit_code": 1, "duration_ms": 32011, "log_path": "…/jobs/j3f9a0c12be.log", "tail": ["…"],
  "error": "step 2 (run pnpm test) exited with code 1" }
```

### Summary

```json
{
  "project": { "name": "nuvara", "root": "/Users/me/code/nuvara", "default_env": "dev",
               "envs": ["dev", "prod", "stage"], "pipelines": ["ci", "test:api"], "deploy_envs": ["prod", "stage"] },
  "services": [ Run, … ],
  "jobs": [ Job, … ],
  "conflicts": [
    { "kind": "port_remapped", "project": "nuvara", "service": "web", "port_name": "http", "port": 3100,
      "default": 3000, "env": "PORT", "remappable": true, "detail": "running on 3100 instead of 3000 (injected via PORT)" },
    { "kind": "port_busy", "project": "nuvara", "service": "causation", "port_name": "http", "port": 3003,
      "default": 3003, "remappable": false, "holder": { "pid": 77, "command": "node" },
      "detail": "in use by node (pid 77); `rocket up causation` will fail (no env var to remap)" }
  ]
}
```

`services` are the `ps` rows, `jobs` only running jobs. Without `project`,
every project's runs and running jobs are listed, `project` is omitted and
`conflicts` is empty (conflicts need a manifest).

When present, `project.envs`, `project.pipelines` and `project.deploy_envs` are
mandatory sorted name arrays, using `[]` for empty lists. They list declared
environments and manifest pipelines; deployment environments include only those
with a configured `deploy`. Taskfile fallback tasks are not added to the pipeline
list. Older daemons may omit these fields: clients should distinguish absent
metadata from an explicitly empty list.

## Event stream (SSE)

Each event is framed as:

```
event: service.state
data: {"type":"service.state","time":"…","project":"nuvara","service":"api","state":"running","run":{…Run…}}

```

Every stream opens with a `: ok` comment (so clients like URLSession surface the response immediately), then a `: ping` comment every 15s. Filters (all optional, combined with AND):
`project`, `service`, `job`, `types` (comma-separated). Slow consumers may miss events
(the bus never blocks the daemon); re-sync with `GET /v1/ps` after reconnecting.

| `type` | Fields |
|---|---|
| `service.state` | `project`, `service`, `state`, `run` (full Run) |
| `log.line` | `project`, `service`, `line` (one line, no trailing newline) |
| `port.leased` | `project`, `service`, `lease` (`{port, project, service, port_name, created_at}`) |
| `port.released` | same as `port.leased` |
| `job.state` | `project`, `job_id`, `status`, `job` (full Job) — on start, on every step start and when finished |
| `job.log` | `project`, `job_id`, `line` |

Log events come from following `logs/<project>/<service>.log`; compose
services only log rocket's own `docker compose up` output there (container
logs are served by `GET /v1/logs` via `docker compose logs`).

## macOS app client contract

Rocket.app (and any other GUI) is a pure API client: it never spawns or kills
processes itself.

**Discovery.** While rocketd runs, `$ROCKET_HOME/daemon.json` (default
`~/.rocket/daemon.json`, mode `0600`, written atomically, removed on clean
shutdown) holds:

```json
{
  "version": "0.1.0-dev",
  "api": "v1",
  "pid": 4242,
  "socket": "/Users/me/.rocket/rocketd.sock",
  "http": "http://127.0.0.1:53124",
  "token": "64 hex chars (32 random bytes, new on every daemon start)",
  "rocket_bin": "/Users/me/go/bin/rocket",
  "started_at": "2026-10-05T00:14:10Z"
}
```

**Auth.** Send `Authorization: Bearer <token>` on every TCP request (SSE
included). Missing/wrong token → `401` with
`{"error": "...", "code": "unauthorized"}` and `WWW-Authenticate: Bearer`.
Tokens are compared in constant time. No CORS headers are served.

**Liveness.** Treat the daemon as running only if `GET /v1/health` answers
`200` with the token. A `daemon.json` left by a crashed daemon (pid gone,
connection refused, or `401` because the token changed) is stale: start the
daemon with `<rocket_bin> daemon start` (fall back to `rocket` on `$PATH`; it
returns once the daemon answers, idempotent), then re-read `daemon.json`. Also
re-read it after any `401`.

**Live state.** Load `GET /v1/projects`, `GET /v1/status?project=<name>` (or
`GET /v1/ps?all=true`), `GET /v1/ports`, `GET /v1/jobs?all=true`, then keep one
`GET /v1/events` SSE stream open and apply `service.state`, `port.leased`,
`port.released`, `job.state` (and `log.line` / `job.log` when a log view is
open — or open the dedicated `GET /v1/logs?…&follow=true` /
`GET /v1/jobs/{id}/logs?follow=true` streams). Events can be dropped for slow
consumers; on reconnect, reload the snapshots. Ignore `: ping` comment lines.

**Actions.** `POST /v1/up|restart|down` (pass `"owner": "user"`), `POST /v1/jobs`,
`POST /v1/jobs/{id}/cancel`, `POST /v1/gc`. A deploy from the app must ask the
human first and then send `"yes": true`.
