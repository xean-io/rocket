# Feature: rocket-daemon-upgrade

## Problem

Opening the current macOS app connects to the habitual daemon started before the
v1 manifest reader changes. GET /v1/status returns HTTP400 for port binding maps
at lines44/61 and pipeline objects at lines100/101 of the XEAN manifest.
The on-disk binary was rebuilt after daemon startup. Starting an already running
daemon is intentionally idempotent; reopening the app does not load new Go code.

## Constraints and decisions

- Preserve the existing manifest and all supervised services, containers and volumes.
- No Git changes, nuvara access, broad process signals or external hand edits.
- Read habitual daemon state only during diagnosis. Tests use private short /tmp homes.
- The user's earlier instruction forbids stopping the real daemon. Activation requires
  explicit permission for graceful daemon-only stop/start after verification is complete.
- Use the current canonical binary and the existing shutdown/reconciliation mechanism.
  Do not change parsers or roll back supported manifest forms to mask stale runtime code.

## Tasks and acceptance

- [x] D1 Observe the actual startup failure and identify stale daemon runtime.
- [x] D2 Verify current binary accepts the real manifest in an isolated home; verify
  tagged restart/adoption behavior with owned fixtures and leave no test processes.
- [x] D3 With user authorization, gracefully replace only the habitual daemon, then
  observe status without parse errors and retain supervised process/container identities.

## Evidence

- Read-only habitual GET /v1/health: PID37464, version0.1.0-dev,
  started2026-10-05T00:55:22.808471Z. The version is unchanged across development builds.
- GET /v1/status?project=xean-spectrai: HTTP400 with the exact four YAML
  errors shown in the screenshot. GET /v1/ps lists seven healthy/running services and
  commerce already failed; GET /v1/jobs reports no running jobs. No mutation occurred.
- AppPID51225. Running local groups: ai-worker70078, api70328, control-plane70012.
  Docker identities: minio1de080c0973c, nats18628d8140d4, postgres8f636c19d88b,
  redis0c535610bdcb. These identities are the activation preservation baseline.

- Current isolated XEAN status: exit0, errors[]; own home `/tmp/rks-6_zej50o`.
  Owned daemon stopped, with PID/socket/discovery files confirmed absent. Canonical
  `bin/rocket` SHA256 equals the freshly built and verified binary.
- Tagged `TestJobTTLRealStepsStartupAndDaemonRestart` plus CLI profile tests passed
  (11.798s); graceful shutdown retained the test-owned running job for adoption.
- Tagged `TestEndToEnd` passed (6.672s), including adoption of surviving services,
  remaps, TTL and owned fixture cleanup. Evidence `.verification/Daemon-upgrade/`.
- Activation is prepared: graceful `bin/rocket daemon stop`, then
  `bin/rocket daemon start` using the same habitual home. This calls the daemon's
  shutdown endpoint and retains supervised children; it does not call service down.
  Compare the preservation baseline and GET /v1/status afterward. App SSE reconnect
  reloads discovery when the daemon's port/token change. User permission is pending.

- User explicitly authorized: "Sí, reinicia solo el daemon". Graceful daemon-only
  stop/start succeeded: oldPID37464 exited; newPID54436 started2026-10-05T13:31:13Z.
  No service down/restart or Docker start/stop operation was issued.
- Habitual GET /v1/status returned HTTP200/errors[], declared envs[dev,stage] and
  nine pipelines. CUA observed automatic app reconnection, then sidebar Refresh
  without the YAML alert; screenshot captured only the Rocket window. AppPID51225
  was not restarted.
- All three actual local processes retained their PIDs/PGIDs, ports and owners:
  ai-worker70078, api70328, control-plane70012. All four container identities and
  four named data volumes remained unchanged.
- Preservation guard initially failed because the old daemon's cached state called
  Compose containers running. Independent Docker inspection proved all four had
  exited at2026-10-05T11:37:04Z, nearly two hours before activation. Reconciliation
  correctly marks them dead; they were not stopped by this operation. The earlier
  seven-running count was stale cached state. Commerce was already failed.
- Evidence `.verification/Daemon-upgrade/{habitual-before.json,habitual-after.json,
  habitual-status.json,habitual-stop.log,habitual-start.log,activation-validation.json}`.
  Source parser/app changes were unnecessary; existing regression/adoption tests
  and isolated current-binary loading passed before activation.
