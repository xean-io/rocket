# Feature: rocket-v1-gaps

Locator: `odd/tasks/rocket-v1-gaps.md` · Engram project `rocket`, topic `odd/rocket-v1-gaps/tasks`

## Objective

Complete G1–G13 for the Go CLI/daemon and native SwiftUI app, preserving legacy manifests,
additive v1 JSON contracts, and exit codes 0/1/2/3. Design context: `odd/plan.md` and
`internal/adapters/api/README.md`; predecessor: `odd/tasks/rocket-mvp.md`.

## Decisions and constraints

- One delegated Go writer, task by task, followed by one Swift writer after the API stabilizes.
  Parent owns this ledger, Engram synchronization, review, and the real XEAN smoke check.
- No Git initialization, commits, branches, or other Git mutations. An existing `.git` was observed.
- Never touch nuvara, signal unrelated processes, use the real Rocket home in tests, or delete Docker volumes.
- Runtime homes use short `/tmp/rk*` paths. The existing app/daemon remain untouched.
- Only the pilot project's `rocket.yaml` receives hand edits outside Rocket.
  Normal generated Cargo/Next.js build output during real XEAN verification is explicitly authorized.
- Port references add startup dependencies. Templates are evaluated at consumer startup;
  changing a provider's port later requires restarting consumers.
- Pipeline prerequisites remain running. Newly started services inherit the job owner and TTL deadline;
  existing ownership and TTLs are preserved. XEAN migrations remain manual prerequisites.
- Preserve an explicitly configured `CREATIVE_PUBLIC_BASE_URL`; use the templated local URL only when empty.
- Glass is confined to controls; XEAN violet `#9F63B9` is the sole accent. Status colors remain semantic.

## Tasks and acceptance

- [x] G1 Port env templates: legacy scalar and template mappings, default bindings, remapping,
  schema validation; API/AI-worker shell wrappers removed.
- [x] G2 Cross-service port refs: validation/cycles, implicit startup dependencies, remapped same-env
  provider ports, failure propagation; XEAN frontend URLs wired to API/commerce.
- [x] G3 Inject forced Compose identity into local services and jobs; remove XEAN assets override.
- [x] G4 Pipeline `needs`: legacy arrays and object form, asynchronous prerequisite startup,
  owner/TTL preservation, failure/cancel handling; XEAN test pipelines need infra.
- [x] G5 `probe: false`: lease/remap retained, eligible readiness selection, invalid explicit target rejected.
- [x] G6 Wildcard profile filtering and repeatable `--profile`; explicit targets/dependencies supported;
  down-all still stops every active service.
- [x] G7 Init maps `setup` to install, preferring an explicit install task.
- [x] G8 Observed new named Compose volumes produce CLI/JSON bootstrap hints; no false claims for
  existing/external volumes or bind mounts.
- [x] G9 Job TTL (including prerequisite startup and reconciliation), `expired_job` GC actions;
  serialized 15-second log-follow heartbeats with complete terminal drainage.
- [x] G10 Sorted environment/pipeline/deploy metadata in status; job env/profile selection;
  outside-project down-all suggests everywhere with stable error/exit behavior.
- [x] G11 Swift declared-env picker, removed-selection reset, older-daemon compatibility.
- [x] G12 Run-pipeline control and job navigation; shared explicit deployment confirmation sheet;
  no submission on cancel or without confirmation.
- [ ] G13 Isolated PID-scoped app verification; Rocket-only menu-bar/light/dark screenshots;
  observed styling/layout defects corrected.

## Verification policy

For each logic task, record targeted RED before implementation and GREEN afterward. Parent reviews
each slice before continuing. Regression commands: both vet configurations, `go test ./...`,
`go test -race ./internal/...`, `go test -tags integration ./...`, Linux/Windows builds, plus
Swift build/test. Process/network tests stay behind integration tags. Compose tests use unique
project names and retain all volumes. App `--verify` is deferred until G13 makes it isolated and safe.

Real XEAN smoke: temporary home, owned holders on 3000 and 8080, status → up infra → up control-plane
→ ps/ports/status → down-all → test-daemon shutdown. Observe targeted injected URLs without exposing
secrets. Preserve holders until cleanup and preserve all data volumes. Unmet migrations are reported
as gaps. Deployment tests use harmless fixture commands.

## Evidence

### Baseline — 2026-10-04 (America/Bogota)

- Context and Engram MVP mirror read before implementation.
- `go test ./...`: passed.
- `cd macos && swift build && swift test`: passed; 34 Swift tests.
- Docker: no running containers; retained `rocket-xean-spectrai-dev_{minio,nats,postgres,redis}-data` volumes.
- Existing app verified by accessibility inspection only; no services or jobs started in its real home.

### Task evidence

### G1 — Port environment templates

- RED: targeted manifest/app tests rejected mappings with `cannot unmarshal !!map into string`;
  schema assertion found only the legacy string form. After correcting an initial test typo,
  app assertions separately observed missing template injection.
- GREEN: `go test ./internal/manifest ./internal/app -run
  'TestPortEnvTemplate|TestUpPortEnvTemplates|TestUpDefaultPortBindingCannotRemap' -count=1` passed.
  Coverage includes scalars, remapped task bindings, inherited/dotenv/service override precedence,
  empty defaults, representative JSON `env`, default-only remap rejection, malformed/duplicate bindings.
- Parent reviewed model/parser/schema/injection/tests and converted XEAN API/AI-worker wrappers to
  `rust:api` / `bun:ai-worker` task services. Creative public URL is a default binding.
- Full Go regression matrix (both vets, unit, internal race, tagged integration, Linux/Windows builds)
  passed. Swift build and 34 existing tests passed. Commands/results/logs: `.verification/G1/`.
- XEAN `status --json` loaded with exit 0 and zero errors in an isolated `/tmp/rkg1-*` home;
  only its daemon was stopped afterward. Real runtime smoke remains scheduled after G10.
- App `--verify` skipped until the G13 script isolates the bundle and process cleanup.

### G2 — Cross-service references

- RED: parser/app assertions found missing dependency edges, accepted unknown refs/cycles,
  consumers starting despite provider failure, and silent reuse from another environment.
  Subsequent assertions exposed rewriting mixed shell `${api.http}` text and failed Compose alias lookup.
- GREEN: `go test ./internal/manifest ./internal/app -run
  'TestPortReference|TestUpCrossServicePortReferences|TestUpPortReference' -count=1` passed.
  Observed explicit-dependency deduplication, dotted names, remaps, failure propagation, environment
  preflight, unchanged same-env consumers, restart refresh, and fresh/existing Compose aliases.
- `go test -tags integration ./cmd/rocket -run TestPortReferencesPropagateRealRemap -count=1 -v`
  passed: owned holder 18431 → provider 18531 → consumer URL 18531; reverse shutdown,
  released leases, and owned daemon cleanup.
- Parent reviewed dependency/parser/runtime/token handling and wired XEAN `XEAN_API_BASE_URL`,
  `MEDUSA_BACKEND_URL`, and `PUBLIC_MEDUSA_BACKEND_URL` to API/commerce references.
  Isolated XEAN status loaded with exit 0 and zero errors; owned daemon/socket cleaned.
- Full Go matrix and Swift build/test (34 tests) passed; evidence `.verification/G2/`.
  App launch remains deferred to G13. Existing general reconciliation Compose-alias behavior
  was left unchanged; only loaded-service startup/reference checks were corrected.
- Runtime blocker discovered after baseline: XEAN dev containers and existing listeners on 3000/8080
  are active. They remain untouched. Required real smoke awaits the user's choice to retain them
  (record blocked) or stop them themselves. Isolated fixture verification remains available.

### G3 — Compose identity

- RED: children received override identities, nondeploy jobs had empty envs, and invalid/mismatched
  environment requests persisted jobs. GREEN: `go test ./internal/app -run
  'TestServicesForceCompose|TestJobsForceCompose|TestJobsRejectInvalidEnvironment' -count=1` passed.
- Forced identity is the final environment layer for task/run services and every job step;
  inherited/dotenv/service/port attempts cannot override it. Default/selected envs, setup sequence,
  task fallback, deploy targets, owner exclusion, and pre-persistence rejection were observed.
- `go test ./internal/adapters/sqlite -run TestJobsRoundTripAndOrdering -count=1` passed,
  including nondeploy selected-env persistence. Existing JSON persistence needs no schema migration.
- Parent reviewed env selection/injection and stop-target reconstruction, and replaced XEAN assets
  with `task: infra:assets`. Isolated XEAN status passed with zero errors and owned daemon cleanup.
- Full Go matrix and Swift build/test (34 tests) passed; evidence `.verification/G3/`.
  CLI env selection remains assigned to G10; safe app launch remains assigned to G13.

### G4 — Pipeline prerequisites

- RED: object pipelines were unsupported; prerequisites/TTL were ignored, failures allowed steps,
  and invalid TTL/targets persisted jobs. Additional RED caught cancellation waiting on a project
  lock, repeated Compose cleanup reporting stopped records twice, and accepted null prerequisites.
- GREEN: `go test ./internal/manifest -run TestPipelineNeeds -count=1` and
  `go test ./internal/app ./internal/manifest -run
  'TestPipelineNeeds|TestDownCancelsPipeline|TestUpCancellation' -count=1` passed, also with `-race`.
  SQLite roundtrip observed `expires_at` persistence.
- Observed persistence/return before blocked readiness, exact absolute deadline inheritance after
  advancing the fake clock, preserved existing owner/deadline, failure before steps, cancellation
  during readiness/queued locks/Compose startup, and Down cancellation before run snapshots.
  Failed Compose cleanup is whole-project only and idempotent; no volume removal.
- Tagged `TestPipelineNeedsStartAndCancelRealPrerequisites` passed: real daemon/process startup,
  job owner/Compose identity, ready prerequisites left running, blocked process group canceled
  before steps, preserved unrelated ready prerequisite, and released leases.
- Parent reviewed parser strictness, gates, async lifecycle, cleanup and tests. XEAN test:e2e and
  test:resilience now declare `needs: [infra]`; migration remains explicitly manual.
- Full Go matrix and Swift build/test (34 tests) passed; isolated XEAN status had zero errors,
  and owned daemon PID/socket/info were gone after stop. Evidence `.verification/G4/`.
- TTL parsing/persistence and inheritance are implemented; automatic job expiry/CLI TTL/heartbeats
  remain G9. App launch remains deferred to G13. Real XEAN runtime smoke remains blocked by live services.

### G5 — Probe control

- RED: `go test ./internal/domain ./internal/manifest ./internal/app -run
  'TestHealthCheckForProbeSelection|TestPortProbe|TestUpProbeSettings|TestUpDisabledProbe' -count=1`
  selected disabled debug 9100 instead of eligible HTTP 8180, probed all-disabled ports,
  and accepted an undeclared explicit target. Parser/schema also lacked the option.
- GREEN: the same targeted command and targeted `-race` passed. Observed default-on behavior,
  explicit booleans/aliases, mixed HTTP/TCP selection, all-disabled readiness, disabled/unknown
  explicit-target rejection, invalid null/string/number/collection rejection, and retained
  leases/remaps/environment injection for disabled ports.
- Tagged probe/Compose command fixtures passed. Listener/HTTP/TCP tests moved behind integration;
  pure ParseLsof remains unit. The harmless command trace observed `compose up -d --wait` unchanged.
- Parent reviewed nullable default-on model, parser/schema and eligible resolution. Full Go matrix,
  Swift build/test (34 tests), and isolated XEAN status passed. Owned daemon PID/socket/info removed.
  Evidence `.verification/G5/`; no G5 behavior gaps found. App verification remains deferred to G13.

### G6 — Startup profiles

- RED: wildcard Up included gated reporting/trends; requested Compose profile `extra` was missing;
  wildcard Restart changed unselected services. Invalid env/TTL stopped healthy runs, and an
  all-gated wildcard restart selected a gated service. Tagged CLI failed with unknown `--profile`.
- GREEN: targeted graph/app/SQLite profile tests and race tests passed, including persisted profiles,
  legacy reconstruction, environment/request union, explicit targets/dependencies, and pre-Down
  validation. Empty filtered restart leaves gated runs untouched.
- Tagged `TestCLIProfilesAndStopAll` passed: repeated flags on Up/Restart, preserved unselected
  reports, and unfiltered all/everywhere cleanup of owned process groups, listeners and leases.
- Parent reviewed startup-only expansion and retained/unioned Compose profiles for shutdown.
  Full Go matrix and Swift build/test (34 tests) passed; isolated XEAN status had zero errors and
  its owned daemon PID/socket/info were removed. Evidence `.verification/G6/`.
- No G6 behavior gaps found. Safe app verification remains deferred to G13; real XEAN smoke is
  blocked by existing services, which remain untouched.

### G7 — Init install fallback

- RED: `go test ./internal/scaffold -run TestDetectSetupInstallTaskPrecedence -count=1`
  found missing `setup.install` and an unintended root `setup` pipeline.
- GREEN: the same command passed. Observed explicit install precedence in both YAML orders,
  setup fallback, internal-task exclusion, unchanged included namespaced tasks, and valid
  render → manifest parsing. Root setup remains available through task fallback.
- Parent reviewed deterministic classification. Full Go matrix and Swift build/test (34 tests)
  passed; isolated XEAN status loaded with zero errors and its owned PID/socket/info were removed.
  Evidence `.verification/G7/`. No G7 behavior gaps found; app launch deferred to G13.

### G8 — First-run volume hints

- RED: targeted tests found missing first-run hints. Parent review then found volume inventory
  used a different environment/directory than Compose startup; the tagged regression observed
  a known fixture volume falsely absent and an inventory error omitted.
- GREEN: pure normalized-config/parser, fake app, human-output and targeted race tests passed.
  Tagged command tests passed after inventory adopted the same target environment/directory.
  Exact-name successful inventories establish absence/presence; errors provide no evidence.
- Observed mounted nonexternal named-volume filtering, fresh → present confirmation, existing
  suppression, deduplication, idempotent Up, configured-action selection, and failed startup
  claims only with confirmed creation. Bind, external, anonymous and unrelated mounts are excluded.
- Actual `go test -tags integration_docker ./internal/adapters/compose/ -count=1 -v` passed
  lifecycle and fresh/existing/external coverage with unique projects. This run preceded the final
  target-environment correction; its tagged regression and Docker-tag compilation passed afterward.
  Retained six volumes: `rocket-volume-itest-18db8346b1fd1de8_{custom,data,external}` and
  `rocket-volume-itest-18db8351ea2d3508_{custom,data,external}`. Parent independently observed
  zero containers/networks for both projects and all six retained volumes; none were deleted.
- Full Go matrix and Swift build/test (34 tests) passed; isolated XEAN status had zero errors and
  owned PID/socket/info were removed. Evidence `.verification/G8/`.
- Nondefault-env hint commands will gain `--env` alongside the G10 CLI flags. App verification
  remains deferred to G13; real XEAN smoke remains blocked by existing services.

### G9 — Job expiry and streaming

- RED: expired live/queued/startup jobs kept running; expired no-PID records became lost; live
  adopted jobs missed expiry. Late readiness and steps ran after their deadline and reported success.
  Reconciliation reported lost IDs despite canceled state when liveness crossed the deadline.
  GC discarded failed Compose cleanup trackers; loaded Compose aliases were marked dead;
  CloseJob dropped non-newline output. The production stream had no ping within 18 seconds.
- GREEN: targeted domain/app/log tests and race tests passed. Independent timers use the injected
  clock's remaining duration; locked transition checks prevent post-deadline work. Cancellation
  retains one reason, signals a step once, and uses a fresh cleanup context. Expired records are
  canceled before lost/adoption classification; GC identifies jobs through `expired_job.detail`.
- Tagged `TestJobTTLRealStepsStartupAndDaemonRestart` passed (8.995s): real step/startup expiry,
  exact prerequisite deadlines, released leases/process groups, expired no-PID restart handling,
  adopted deadline preservation, partial SIGTERM output before one terminal event, blocking exit 2,
  setup/deploy flags and stable invalid-TTL errors. All fixtures use isolated homes and harmless deploys.
- API heartbeat/trailing-log integration passed at the production 15-second interval (15.609s).
  Parent reran `go test -race -tags integration ./internal/adapters/api -run
  TestJobFollowHeartbeatAndTrailingTerminalOrdering -count=1 -v`: passed, no race,
  partial drainage then exactly one terminal event with no frames afterward.
- Parent reviewed timers, cleanup, deadline boundaries, single-writer SSE and GC retention.
  The G2 loaded-manifest Compose alias reconciliation gap is now fixed. Missing/unavailable manifests
  retain the legacy service-name fallback; aliases cannot be inferred without their manifest.
- Socket-based API SSE tests moved behind `integration`; health/token tests use recorders.
  Full Go matrix and Swift build/test (34 tests) passed; isolated XEAN status had zero errors and
  owned daemon PID/socket/info were removed. Evidence `.verification/G9/`, including SSE race log.
- App launch remains deferred to G13. Real XEAN runtime smoke remains blocked by existing services.

### G10 — Project metadata and job selection

- RED: targeted assertions found absent metadata arrays, dropped requested/job/SQLite profiles,
  wrong wildcard prerequisite selection, accepted deployment profiles, incomplete project/env
  hints, and missing outside-project guidance. Tagged CLI initially rejected `--env`.
- GREEN: targeted metadata/profile/persistence/hint/guidance tests passed; both vet configurations,
  unit and internal race tests passed. Metadata preserves legacy fields and emits sorted non-null
  `envs`, `pipelines` and `deploy_envs` arrays, including `[]` for empty lists.
- Tagged `TestJobSelectionMetadataAndDownGuidance` passed in 3.558s: pipeline/setup/doctor/install/migrate environment/profile selection,
  preserved effective profile snapshots, harmless deploy confirmation and rejection of unsupported
  deploy flags without submission, and unchanged JSON error shape/exit 1 for outside-project down.
  An intermediate assertion wrongly expected JSON for unsupported Cobra flags; it was corrected to
  verify existing flag-error behavior and absence of a job request.
- Job profiles are the sorted environment/request union used for prerequisites, with each Compose
  service adding its own profiles. Existing services keep their launch selection. No forced
  `COMPOSE_PROFILES` is added to job step environments. Deploy rejects nonempty requested profiles.
- Volume hints now target the registered project and quote a nondefault `--env`; this completes
  the G8 nondefault-environment follow-up. Typed missing-manifest detection limits the everywhere
  suggestion to implicit outside-project lookup, preserving malformed/explicit-path behavior.
- Parent reviewed source, tests and additive API/CLI documentation. Full Go matrix, Linux/Windows
  builds and Swift build/test (34 tests) passed. Isolated real XEAN status loaded with zero errors;
  only the owned daemon was stopped, with PID/socket/info confirmed gone. Evidence `.verification/G10/`.
- App launch remains deferred to G13. Read-only inventory still found user-owned listeners on
  3000/8080 and four `rocket-xean-spectrai-dev` containers; required real smoke remains blocked,
  and these services/data remain untouched.

### G11 — Declared environment picker

- RED: `cd macos && swift test --filter EnvironmentTests` compiled and failed four tests/six
  assertions: declared metadata was dropped, an authoritative empty list fell back to default/seen
  values, historical environments leaked into options and an unstarted declared environment was
  absent. Separate compiling placeholder validation helpers then failed two tests/five assertions
  for stale/removed selections before the selection logic was implemented.
- GREEN: the targeted six Swift Testing tests passed; `swift build` and all 40 Swift tests passed.
  Optional `ProjectInfo.envs` distinguishes older absent metadata from authoritative `[]`; snapshots
  replace declarations, and only older metadata uses default plus seen environments.
- Parent reviewed model/store, pure selection validation, global Root observation, picker and
  command submission. Every selected project's options are observed even on another pane; removed
  selections reset to default and actions validate again before sending.
- Full requested Go matrix and Swift build/test passed; isolated XEAN status had zero errors,
  and owned daemon PID/socket/info were gone after cleanup. Evidence `.verification/G11/`.
- No G11 logic gaps found. Visible manifest-removal demonstration remains scheduled for G13;
  the existing unsafe launch script was never executed. Real XEAN smoke remains blocked.

### G12 — Pipelines and explicit deployment confirmation

- RED: `swift test --filter JobActionTests` compiled and failed eight tests/24 issues: unconfirmed
  reruns sent requests, reruns dropped environment/profiles and the returned job, terminal state
  was overwritten, metadata/job fields were discarded, and new submission APIs lacked behavior.
  A nested `#require` test compile issue was corrected before observing assertion failures.
  Additional assertions reproduced missing action-list storage and stale full/status-only events.
- Parent review reproduced a delayed empty history GET removing a newer submitted/SSE job.
  `swift test --filter staleEmptyHistorySnapshotCannotRemoveANewerJob` failed both parameterized
  cases, then passed after refresh captured a monotonic local job revision. Unchanged snapshots
  still replace old history; newer local/event mutations make an older response inapplicable.
- GREEN: targeted JobAction tests passed, `swift build` passed, and all 53 Swift tests passed.
  Request-body tests observed owner user, selected environment, rerun profiles/args, optional TTL,
  backward-compatible fields, cancellation with zero requests and confirmed deployment `yes: true`.
- Parent reviewed the declared Run Pipeline menu, separate deployment list, shared Root sheet,
  controller confirmation guards, immediate stored response and `showJob` history/log navigation.
  Terminal jobs survive older running POST/snapshot/events; submission errors return no navigation
  target. No extra history GET runs before showing the returned job.
- Full Go matrix and Swift build/test passed; isolated XEAN status had zero errors and owned
  daemon PID/socket/info were gone after stop. Evidence `.verification/G12/`.
- Visible pipeline navigation and shared-sheet cancellation/confirmation checks remain scheduled
  for G13; no unsafe script or real app/daemon interaction occurred. Real XEAN smoke remains blocked.

### Final Docker verification follow-up

- `ROCKET_HOME=/tmp/rkvd-69ij7q2l go test -tags integration_docker
  ./internal/adapters/compose/ -count=1 -v` passed on the final Go source (16.449s),
  including fresh/existing/external volume behavior after the G8 inventory-environment fix.
- Parent observed zero containers and networks for `rocket-itest-18db86c6fbd2f428` and
  `rocket-volume-itest-18db86c833c9af88`. All three named fixture volumes remain retained
  (`_custom`, `_data`, `_external`); no volumes were deleted.
- Evidence `.verification/Final/compose-docker.log`, `compose-docker-result.json` and
  independent `compose-cleanup.json`. This closes the earlier G8 final-source Docker rerun gap;
  it does not substitute for the blocked real XEAN smoke.

### G13 — Isolation and visual verification (menu-bar check pending)

- RED before implementation: five static launcher assertions found name-based process control,
  implicit GUI environment, missing separate bundle/marker, absent receipt cleanup/retained mode,
  and subsystem-only logs. Swift verification safety compiled and failed two of three test functions
  with 12 assertions for missing/invalid homes and escaping paths. Cleanup review added two failing
  assertions for running jobs and retained socket/info files before their checks were implemented.
- GREEN: shell/Python syntax, nine static/mocked launcher tests and all 56 Swift tests passed.
  Parent reviewed explicit LaunchServices environment, private 0700 home, marker preflight before
  controller creation, per-instance appearance and exact executable/UID/start-time PID receipts.
  Parent read a test-owned process through libproc and observed the expected PID/UID/identity.
- Full Go/Swift matrix and isolated XEAN status passed (`.verification/G13/`). Default
  `ROCKET_BIN=.verification/G13/rocket macos/script/build_and_run.sh --verify` passed with eight
  Swift SSE events, then cleaned app 66803/daemon 66791. Parent independently observed receipt
  cleaned and absent socket/info/PID files; habitual app 15527 remained alive.
- Retained light launch `6728cf64` uses app 67664, daemon 67658, `/tmp/rkv-kyyhurw6`, copied harmless
  fixture and nine initial SSE events. Only its window and sheets were captured through CUA.
  Declared dev/smoke/stage were visible; smoke slow-check navigated immediately to a running job
  and logs. Stored job `jc4599c63cf` confirmed env smoke, owner user and success.
- New deployment cancellation left three jobs/zero deploys; confirmed harmless stage created
  `je24c286d78` as user and navigated to Jobs. Run Again showed the shared sheet, and cancellation
  retained four jobs/one deploy. Public observations: `ui-jobs-observations.jsonl`.
- Removing smoke from the copied manifest while Jobs was open, then Refresh, reset the project
  picker to dev. Options then contained only Default(dev), dev and stage. Original external XEAN
  manifest was not involved.
- Observed defects: history columns collapsed under the inspector; rendered job logs duplicated
  start/step/output (12 UI lines versus eight authoritative daemon lines). Swift writer is correcting
  compact history and ordered finite job-log ownership with assertion RED, preserving legitimate
  repeated text. These are pending re-verification; G13 remains unchecked.
- Native CUA app AX does not expose the status icon; Control-F8 had no observable result and binding
  SystemUIServer timed out. A concise asynchronous request asks the user to open the verification
  popup; all screenshots remain Rocket-only. Menu-bar and final dark checks remain pending.

- Visual-defect follow-up RED: realistic tail/global-event handoff failed three tests/seven
  assertions, controller stream wiring failed three assertions, and terminal body closure failed
  one assertion. GREEN: six Swift Testing functions (including parameterized cancellation cases)
  cover repeated lines, stale source IDs, one authenticated request, trailing logs before terminal,
  premature EOF without replay and explicit body teardown. Full Swift suite reached 62 tests/10
  suites; native history source now uses four essential columns below 780pt, with omitted facts
  retained in the inspector. Parent reviewed source; fresh runtime checks remain pending.
- Canonical `go build -o bin/rocket ./cmd/rocket` passed. It does not restart the habitual daemon
  or replace the running habitual macOS bundle.

- Connection review added assertion RED for direct disconnect/reconnect while the inspector
  remained selected (two parameterized cases/four assertions). Controller-owned follow tasks now
  cancel before changing clients and forward selecting-view cancellation. GREEN: targeted seven
  functions, full 63 Swift tests/10 suites, Swift build, nine launcher safety tests and Bash syntax.
  Parent reviewed cancellation/source ordering. The final requested Go/Swift matrix passed in
  `.verification/G13-final/`; unchanged Go commands were cached as shown in their logs.
- Receipt-scoped cleanup of the original light bundle6728cf64 passed before the corrected launch;
  its app/daemon were stopped without signaling the habitual app or XEAN services.

- Corrected light bundle1e3fe544 passed isolated verification (app77352/daemon77343,
  home `/tmp/rkv-7cswl1h8`, five initial SSE events). Cropped CUA screenshots showed immediate
  running-job navigation and readable Job/Project/Exit/Duration columns with the inspector open.
  Harmless smoke job `j223a1e169d` completed as owner user; UI's ten lines exactly matched
  `rocket job j223a1e169d logs --tail 300 --json`, retaining both repeated output lines and
  the final partial line before success. Refresh retained the same ten lines without replay.
  An initial observation command used an unsupported logs flag; the documented job command
  corrected it, without any runtime mutation. Evidence `ui-slow-check-logs.json`/`ui-jobs.json`.

- Dark bundlee659add6 passed isolated verification (app78557/daemon78551,
  `/tmp/rkv-3xe8eoug`, six initial SSE events). Rocket-only CUA screenshots showed readable
  project controls and compact Jobs/inspector, with plain content surfaces, glass controls,
  violet accent and semantic status colors. Harmless dev check `j7d7b9e2671` completed as user.
- Light/dark receipt cleanup passed. Parent independently confirmed all four verification
  receipts marked cleaned, all their app/daemon PIDs absent, all socket/info files absent and
  no fixture listener18771. Habitual app15527, user listeners70014/70330 and the same four
  XEAN dev containers matched the saved baseline exactly. No volume was deleted. Evidence
  `.verification/G13-final/cleanup-and-preserved-runtime.json`.
- Final isolated actual-manifest `status --json` returned exit0/zero errors; test daemon stopped
  with PID/socket/info absent. Evidence `.verification/G13-final/xean-status*`.

## Remaining verification gaps

- G13 remains unchecked solely because the actual menu-bar extra popup was not observed.
  Native CUA exposes Rocket windows but not its status icon; the system host timed out and
  the user-open request received no reply before cleanup. No full-screen capture or surrogate
  popup was used. Light/dark windows, pipeline navigation, confirmation sheets and isolation
  have been observed; source implementation and automated checks are complete.
- Real XEAN `up infra`/`up control-plane` smoke with owned holders3000/8080 is unrun because
  those ports and the matching Compose project already belong to active user services. No
  service was adopted, stopped or restarted. Tagged harmless fixtures cover remap/reference
  logic, and isolated real manifest loading passes; this does not prove the requested real
  runtime scenario. Migrations/assets remain a documented manual prerequisite and were not run.
- Canonical Go binary is rebuilt; the habitual running app/daemon and Git repository remain
  untouched. macOS changes are in source and separately verified bundles.


## Follow-up: opening the updated app against the old daemon

The user's startup report reproduced four parser errors from habitual daemonPID37464,
started before the v1 binary rebuild. Current canonical binary accepts the real manifest
in isolation. No v1 manifest rollback is needed; activation requires a graceful daemon-only
replacement. The user explicitly authorized graceful daemon-only replacement. New daemonPID54436
loads the manifest without errors; the app reconnects and Refresh shows no parser alert.
Local PIDs and container/volume identities were preserved. Docker inspection showed that
the four Compose containers had exited nearly two hours before activation; reconciliation
now reports their actual dead state. No service start/stop was performed.
Diagnosis and restart/adoption evidence: `odd/tasks/rocket-daemon-upgrade.md`.
