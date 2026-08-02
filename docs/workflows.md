# GitHub Actions Workflows

A map of every workflow in `.github/workflows/`, written for both humans and AI
agents. Each entry says **when it runs**, **what it does**, and the **gotchas**.
For the full pipeline rationale see [ci-cd.md](ci-cd.md); for releasing see
[releasing.md](releasing.md).

## Overview

| Workflow | Trigger | Purpose |
|---|---|---|
| `ci.yml` | every PR · push to `dev`/`main` · `v*.*.*` tags | Quality gate + build-once image + publish (DockerHub + ghcr) |
| `security.yml` | every PR · every push | Semgrep SAST (SARIF + blocking on ERROR) |
| `auto-pr.yml` | push to any non-protected branch | Open a draft PR into `dev`; delete mis-named branches |
| `dependabot-auto-merge.yml` | Dependabot PRs | Enable auto-merge for patch/minor dependency bumps |
| `release.yml` | `v*.*.*` tag push | Create the GitHub Release + reopen the changelog on `dev` |
| `release-dispatch.yml` | manual (`workflow_dispatch`) | One-click release: pick patch/minor/major (owner-only) |

> Dependency updates are configured in `.github/dependabot.yml` (one grouped PR
> per ecosystem — cargo + github-actions), which is Dependabot config, not a
> workflow.

---

## `ci.yml` — quality gate + publish

**Runs on** every pull request, pushes to `dev`/`main`, and `v*.*.*` tags.

**Jobs** (each has one responsibility; later jobs depend on earlier via `needs`):

1. `check` — `cargo fmt --check` + `cargo clippy -D warnings`.
2. `test` — `cargo test` on stable **and** beta. Beta is a non-blocking canary
   (`continue-on-error`).
3. `supply-chain` — `cargo deny check` (CVEs, licenses, banned crates, sources).
4. `coverage` — `cargo llvm-cov` with `--fail-under-lines 80` (workspace lines).
5. `version-gate` — validates the top `CHANGELOG.md` version is valid SemVer and
   strictly greater than the last `v*` tag. **Only enforces in a release context**
   (PR into `main`, or push to `main`); a no-op pass otherwise. It must always run
   (a skipped `needs` job would skip `build` too).
6. `build` (matrix, one per arch on a **native** runner) — builds the image
   **exactly once** into `image.tar` and uploads it as an artifact.
7. `scan` — Trivy against that artifact (full SARIF pass + blocking pass on
   fixable CRITICAL/HIGH).
8. `integration` — `docker load` + `docker compose --no-build` against that same
   artifact + the system integration test.
9. `push` (`if: push`) — `skopeo` copies the scanned/tested tarball to the
   registry **by digest**; skipped on PRs (credentials never reachable there).
10. `publish` (`if: push`) — assembles the multi-arch DockerHub manifest, mirrors
    it to `ghcr.io` (`skopeo copy --all`, byte-identical), deletes staging tags,
    and creates the git tag `vX.Y.Z`.

**Key point:** the image is built once; scan, integration and publish all consume
the *same* tarball, so what ships is provably what was scanned and tested.

---

## `security.yml` — Semgrep SAST

**Runs on** every PR and every push.

- **Full scan → SARIF**: all findings uploaded to the GitHub Security tab (never
  fails the run). Findings suppressed by a reviewed in-code `// nosemgrep:
  <rule>` comment are stripped before upload — GitHub ignores the SARIF
  `suppressions` property, so they would otherwise stay open forever. The
  in-code comment (with its rationale) is the authoritative acceptance record;
  see `docs/hardening.md`.
- **Blocking scan** (`--error`): only ERROR-severity findings fail the run and
  block the PR.
- Rulesets: `p/rust` + `p/secrets`.

Trivy is **not** here — it lives in `ci.yml`'s `scan` job so it scans the exact
image that ships.

---

## `auto-pr.yml` — draft PR opener / branch janitor

**Runs on** a push to any branch except `main`, `dev`, `dependabot/**`,
`release/**`.

- If the branch name matches `feature|fix|chore|docs|test/…`, it opens a **draft
  PR into `dev`** (title derived from the branch) if one doesn't already exist.
- If it doesn't match, the branch is **deleted** (naming enforcement).

**Gotcha:** it uses the built-in `GITHUB_TOKEN`, so PRs it opens **do not trigger
`ci.yml`**. When you need CI on such a PR, reopen it from an account/PAT (close +
reopen), or push a new commit as yourself.

---

## `dependabot-auto-merge.yml` — hands-off dependency bumps

**Runs on** `pull_request` events, but only acts when the author is
`dependabot[bot]`.

- Reads the bump type via `dependabot/fetch-metadata`.
- For **patch/minor** bumps, enables auto-merge (`gh pr merge --auto --squash`)
  into `dev`; the merge completes once the ruleset's required checks are green.
- **Major** bumps are left open for manual review — CI green doesn't prove a major
  is behaviourally safe.

---

## `release.yml` — GitHub Release + next-cycle prep

**Runs on** a `v*.*.*` **tag push** (which `ci.yml`'s `publish` creates).

- `github-release` — re-runs the full test suite with coverage on the tagged
  commit (`cargo llvm-cov`, same pins as ci.yml's coverage job — a failure here
  means the tag points at something CI never gated, and aborts), then creates
  the GitHub Release: notes = the version's `CHANGELOG.md` section + container
  pull commands with the DockerHub manifest digest (fetched from the public
  registry, best-effort) + a test summary; `test-report.md` is attached as an
  asset (`scripts/test-report.sh`). `0.x`/`-rc` flagged as pre-release;
  idempotent (a re-run skips creation and refreshes the asset).
- `prepare-dev` — opens an **auto-merging PR into `dev`** that reopens a fresh
  `## [Unreleased]`, fixes the compare links, and bumps `Cargo.toml`. It only
  pushes to a `release/*` branch (which `auto-pr.yml` ignores), never to
  `main`/`dev` directly.

**Gotchas:** never hand-push `v*` tags (they must come from the gated pipeline).
The auto-merge relies on the `RELEASE_PAT` secret so the PR triggers CI; without
it the PR opens but you merge it yourself.

---

## `release-dispatch.yml` — one-click release (owner-only)

**Runs on** manual `workflow_dispatch` with a `bump` input (`patch`/`minor`/`major`).

- First step asserts `github.actor == github.repository_owner` — **only the owner
  can run it** (others get a hard error).
- Computes the next version from the last release, refuses an empty
  `## [Unreleased]` or an already-existing tag, stamps `CHANGELOG.md` +
  `Cargo.toml`, and opens an **auto-merging PR into `main`**.
- From the merge on, the normal path takes over: `ci.yml` publishes + tags, then
  `release.yml` runs. See [releasing.md](releasing.md).
