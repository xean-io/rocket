# Rocket.app

Native macOS client for `rocketd` (SwiftUI, Liquid Glass, macOS 26+). It is a
pure API client: it reads `$ROCKET_HOME/daemon.json`, talks to the daemon's
token-protected TCP listener, and never spawns services itself (it only runs
`rocket daemon start|stop`).

## Layout

| Path | What |
|---|---|
| `Package.swift` | SwiftPM package (Swift 6 language mode, strict concurrency) |
| `Sources/RocketKit` | Models mirroring the Go JSON, `DaemonConfig` loader, `RocketClient` actor, SSE parser + reconnecting `EventStream`, `RocketStore` (pure reducers), `RocketController` (connection + actions) |
| `Sources/Rocket` | The app: scenes, commands, XEAN theme, container/presentational views |
| `Tests/RocketKitTests` | Swift Testing suites with Go-shaped fixtures from the API README |
| `Resources/AppIcon.icon` | Icon Composer bundle (bone rocket, violet flame, lacquer ground, violet glow) |
| `Resources/Assets.xcassets` | `AccentColor` (XEAN violet) |
| `Resources/AppIcon.icns` | Prebuilt fallback icon for machines without Xcode |
| `script/build_and_run.sh` | Entry point for bundle assembly and receipt-scoped launch/cleanup |
| `script/build_rocket.py` | Builds/signs bundles; isolates verification homes, fixtures and process receipts |

## Build, run, test

```sh
cd macos
swift build                          # debug build
swift test                           # RocketKit tests
script/build_and_run.sh              # release build -> build/Rocket.app (ad-hoc signed)
script/build_and_run.sh --run        # ...and open it
script/build_and_run.sh --debug --run
script/build_and_run.sh --verify     # separate bundle/home, fixture + SSE check, then cleanup
script/build_and_run.sh --logs       # open, then stream only that app PID's logs
script/build_and_run.sh --self-check # headless connectivity probe (see below)
python3 script/test_build_safety.py   # static/mocked script safety checks, no processes or network
```

The Codex Run action uses `macos/script/build_and_run.sh --debug --run` from
the repository root to rebuild and launch the app.

Normal builds refuse to replace a running bundle. Run/log modes stop only a
matching process from `build/run-receipt.json`; PID, executable, owner and start
time must match. An app launched outside this script remains untouched.

Verification always creates a distinct `build/verification/<id>/Rocket Verify
<id>.app`, its own copied Go binary and an owned `0700` `/tmp/rkv-*` home. It
registers only a copied harmless fixture, starts its preview service, and sends
fixture jobs until the Swift client confirms its daemon identity and SSE events.
GUI launches pass `ROCKET_HOME` and `ROCKET_BIN` explicitly through `open --env`.
The verification marker makes the app refuse startup before creating its
controller when the explicit private home is absent, invalid or resolves elsewhere.

For window/popover inspection, retain a successful launch:

```sh
ROCKET_BIN=/absolute/path/to/rocket script/build_and_run.sh --debug --verify --keep-running --appearance light
script/build_and_run.sh --cleanup /absolute/path/to/build/verification/<id>/receipt.json
```

`--appearance light|dark` changes only that verification instance. `--fixture`
accepts a harmless manifest inside Rocket and copies it into the verification
directory; the printed fixture path is safe to edit for picker/pipeline checks.
The printed receipt identifies the exact app and daemon PIDs/start times. Cleanup
stops the app first, downs its private services/jobs, verifies process/lease/job
cleanup, then stops its daemon and checks socket/info removal. Volumes are never
removed. A failed verification attempts cleanup and retains its receipt and
original error. Default `--verify` cleans successful launches too.

When inspecting with computer-use, bind the exact printed bundle while its
receipt-owned PID is still alive, and capture only its windows or popovers.

Headless probe, useful against a scratch daemon:

```sh
ROCKET_HOME=/tmp/rk build/Rocket.app/Contents/MacOS/Rocket --self-check [--timeout 30] [--exercise]
```

It connects (starting rocketd if needed), lists projects/services/ports/jobs,
waits for one SSE event, loads a log tail and a job log, and with `--exercise`
also restarts one running service and re-runs one pipeline. Exit 0 = OK.

Debug launch hooks (UserDefaults arguments):
`-RocketInitialSection ports|jobs|owners`, `-RocketSelectService project/service`.

## Behaviour

- Connection: load `daemon.json` → `GET /v1/health`. If that fails, run
  `<rocket_bin> daemon start` (falls back to `rocket` on `$PATH`,
  `/opt/homebrew/bin`, `/usr/local/bin`, `~/go/bin`), then retry with backoff.
  Any `401` re-reads `daemon.json` (tokens rotate on every daemon start).
- Live state: `GET /v1/projects`, `GET /v1/ps?all=true`, `GET /v1/status?project=`
  per project, `GET /v1/ports`, `GET /v1/jobs?all=true`, then one `GET /v1/events`
  stream. Snapshots are reloaded after every stream reconnect.
- Actions send `"owner": "user"`. Stopping a whole project, stopping everything
  and "Stop all for owner" ask first. New deployments and deployment reruns share
  an explicit confirmation sheet; cancellation sends no request, and confirmation
  sends `"yes": true`.
- Shortcuts: ⌘R restart, ⌘. stop, ⇧⌘U up, ⌘L logs inspector, ⇧⌘R refresh, ⌘1–4 sections.
- The sidebar exposes Add Project (⌘O), Refresh and Settings. Project rows keep
  their Reveal in Finder and confirmed Remove actions in the context menu.
- Sidebar icons and the selected row use XEAN colors independently of the system
  accent. Arrow keys navigate projects and daemon sections; ⌘1–4 also remain available.
- Services, Ports and Jobs table selections use the same violet accent. A small
  AppKit drawing bridge preserves native table selection, columns and context menus;
  status glyphs and errors keep their semantic colors.
- Detail headers stay at the top in empty and populated views. Empty states fill
  the remaining pane, with shared adaptive text colors for light and dark mode.
- Status facts use plain content capsules and semantic glyph colors. Custom
  Liquid Glass is reserved for interactive controls and uses XEAN violet as its accent.
- The environment picker uses the declared list from project status. A removed
  selection resets to the project default across all panes and commands. With an
  older daemon that omits the list, it offers the default and environments seen on runs.
- The project toolbar's Run Pipeline menu lists declared pipelines and submits
  the selected environment. The returned job appears immediately in Jobs with
  its logs open. Deploy… lists configured deployment environments separately.
  Pipeline/setup reruns preserve their environment, profiles and arguments;
  deployment reruns use their target environment after confirmation.
- The Jobs inspector reads one ordered `/v1/jobs/<id>/logs?follow=true&tail=300`
  stream. Global job log events do not append to that buffer, so a queued event
  cannot replay the tail; legitimate repeated lines remain. The stream stops
  after the daemon's final state. Leaving the view or changing the daemon
  connection cancels it immediately. Refresh reopens the tail explicitly.

## Known limitations

- No `.xcodeproj`: SwiftPM only, so no SwiftUI previews or asset-catalog
  compilation inside the package. The build script compiles the icon and accent
  colour with `xcrun actool` when Xcode is installed; with Command Line Tools
  only it copies `Resources/AppIcon.icns` (flat icon, system accent).
- Ad-hoc signature only: no notarization, no hardened runtime, no sandbox
  (the app reads `~/.rocket` and launches the `rocket` CLI).
- `URLSession` delivers an SSE response only after the first body bytes, which
  rocketd sends with the first event or the 15s ping; the app shows "Live"
  optimistically until a drop.
