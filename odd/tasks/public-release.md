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

- [ ] T1 Repo hygiene (route: inline, mechanical rename via `sd`)
- [ ] T2 Release tooling + README install docs (route: inline, new config files)
- [ ] T3 Publish repos, push, tag `v0.1.0`, verify release (route: inline, bash state)

## Checks

- `go build ./... && go vet ./... && go test ./...`
- `goreleaser check` and `goreleaser release --snapshot --clean`
- `gitleaks detect` before first push
- Release workflow green; `go install github.com/xean-io/rocket/cmd/rocket@v0.1.0` works

## Progress / evidence

(pending)

## Next step

T1.
