# Public release of the rocket CLI

## Objective

Publish rocket as a public open-source CLI at `github.com/xean-io/rocket`,
installable via Homebrew, `go install`, and GitHub Release archives.

## Problem / why

The repo had no remote, no commits, a module path (`github.com/lemonsalve/rocket`)
that did not match its future home, and no release automation.

## Scope

- Repo hygiene: `.gitignore`, LICENSE, module path `github.com/xean-io/rocket`,
  scrub local absolute paths from docs, default branch `main`.
- Release tooling: `.goreleaser.yaml` (cross-platform, `CGO_ENABLED=0`, version
  via ldflags, Homebrew cask in `xean-io/homebrew-tap`), release + CI workflows.
- Publish: create public repos, push, tag `v0.1.0`, verify the release.

Out of scope: signing/notarizing the macOS app in `macos/`.

## Constraints

- Tap publishing needs a PAT secret (`TAP_GITHUB_TOKEN`) the user creates;
  the release must still succeed without it (cask upload skipped).
- Delivery strategy: `single-pr` not applicable (initial import on `main`).

## Tasks

- [x] T1 Repo hygiene (route: inline, mechanical rename via `sd`) — commit 5c93b0c
- [x] T2 Release tooling + README install docs (route: inline, new config files) — commit 5b30276
- [x] T3 Publish repos, push, tag `v0.1.0`, verify release (route: inline, bash state) — CI fix df0b91f, tag v0.1.0

## Checks

- `go build ./... && go vet ./... && go test ./...`
- `goreleaser check` and `goreleaser release --snapshot --clean`
- `gitleaks detect` before first push
- Release workflow green; `go install github.com/xean-io/rocket/cmd/rocket@v0.1.0` works

## Progress / evidence

- Local: `go build/vet/test ./...` ok; `go test -tags integration ./...` ok.
- `goreleaser check` ok; `goreleaser release --snapshot --clean` built 6 archives,
  `rocket --version` printed the injected snapshot version, cask rendered.
- `gitleaks` (staged + dir): no leaks. Absolute local paths scrubbed from `odd/`.
- Repos created: `xean-io/rocket` (public), `xean-io/homebrew-tap` (public); `main` pushed.
- RDD: off (global) — no native review.
- First CI run failed on macOS only: e2e fixture `static` (python3 http.server)
  not healthy within 20s on the GitHub runner. Integration tests now run on Linux
  only (df0b91f); CI green on ubuntu + macOS.
- Release workflow for `v0.1.0` green: 6 archives + checksums published.
  Downloaded darwin_arm64 binary prints `rocket version 0.1.0`.
  `go install ...@v0.1.0` works but prints `0.1.0-dev` (no ldflags in go install).
- Homebrew cask upload skipped: `TAP_GITHUB_TOKEN` secret not set yet.

## Next step

User creates a fine-grained PAT (contents: read/write on `xean-io/homebrew-tap`),
saves it as secret `TAP_GITHUB_TOKEN` in `xean-io/rocket`, then tags `v0.1.1`
(re-running `v0.1.0` would collide with the already uploaded assets).
