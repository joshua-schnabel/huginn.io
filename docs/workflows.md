# Workflows

A map of every workflow in `.github/workflows/`, written for both humans and AI
agents. Each entry says **when it runs**, **what it does**, and the **gotchas**.
The pipeline's rationale is [`ci-cd.md`](ci-cd.md); cutting a release is
[`releasing.md`](releasing.md).

## Overview

| Workflow | Trigger | Purpose |
|---|---|---|
| `ci.yml` | every PR · push to `dev`/`main` · `v*.*.*` tags | Quality gates, build-once image, publish to Docker Hub + ghcr |
| `security.yml` | every PR · every push | ShellCheck, actionlint, Semgrep SAST |
| `auto-pr.yml` | push to any non-protected branch | Open a draft PR into `dev`; delete mis-named branches |
| `dependabot-auto-merge.yml` | Dependabot PRs | Auto-merge patch and minor bumps |
| `release.yml` | `v*.*.*` tag push · manual | GitHub Release, SBOM, test report, housekeeping PR |
| `release-dispatch.yml` | manual, owner-only | One-click release: pick patch/minor/major |

Dependency updates are configured in `.github/dependabot.yml` — one grouped PR
per ecosystem (cargo, github-actions, docker), all targeting `dev`. That is
Dependabot config, not a workflow.

## `ci.yml` — quality gates and publish

**Runs on** every pull request, pushes to `dev`/`main`, and `v*.*.*` tags.

Each job has one responsibility; later jobs depend on earlier ones via `needs`.

1. **`check`** — `cargo fmt --check` and `cargo clippy -D warnings`.
2. **`test`** — `cargo test --all --locked` on stable **and** beta. Beta is a
   non-blocking canary (`continue-on-error`): it warns of an upcoming toolchain
   break without gating a merge.
3. **`supply-chain`** — `cargo deny check`: advisories, licences, banned crates,
   registry sources.
4. **`coverage`** — `cargo llvm-cov --fail-under-lines 80`, **workspace-aggregate
   line** coverage, not per-file and not regions. Uploads `lcov.info` and writes
   the percentage into the job summary, computed from the file's `LF`/`LH`
   records. The `cargo-llvm-cov` binary is pinned and cached; `cargo-deny` is
   deliberately left on the latest release so it keeps picking up new advisory
   classes.
5. **`version-gate`** — the top `CHANGELOG.md` version must be valid SemVer and
   strictly greater than the last `v*` tag. Enforces **only** in a release
   context (a PR whose base is `main`, or a push to `main`) and is a no-op pass
   otherwise. It must always *run*: a skipped `needs` job would skip `build` too.
   `build` lists it, so an invalid release version fails before the expensive
   image build.
6. **`build`** (matrix, per architecture, **native** runner) — builds the image
   exactly once into `image.tar` and uploads it as an artefact. Native runners
   replaced QEMU, which had made the arm64 build the pipeline's dominant cost.
7. **`scan`** (matrix) — Trivy against that artefact: a full SARIF pass to the
   Security tab, a blocking pass on fixable CRITICAL/HIGH reading
   `.trivyignore.yaml`, and a CycloneDX SBOM kept for 90 days. Never rebuilds.
8. **`integration`** (matrix, native runner) — `docker load` plus
   `docker compose --no-build`, then `scripts/integration-test.sh` against the
   loaded image.
9. **`push`** (matrix, `if: push`) — needs `scan` **and** `integration`; skopeo
   copies the scanned tarball to a staging tag by digest and records the digest.
   Skipped on PRs, so registry credentials are never reachable there.
10. **`publish`** (`if: push`) — assembles the multi-arch manifest from the
    digests, mirrors it to ghcr with `skopeo copy --all`, deletes the staging
    tags, and creates the git tag `vX.Y.Z`. It publishes **no new bytes**.

**Gotchas**

- `publish` is the only job in this file whose checkout keeps its credentials,
  because it pushes the tag. It runs no cargo — see
  [`ci-cd.md`](ci-cd.md#what-can-reach-a-credential).
- The tag is pushed with `RELEASE_PAT` where available. With `GITHUB_TOKEN`,
  GitHub's recursion guard means `release.yml` never fires.
- `publish` declares `contents: write`; inheriting `contents: read` makes
  `git push origin vX.Y.Z` return 403, which is why the repository went so long
  with no tags at all.

## `security.yml` — ShellCheck, actionlint, Semgrep

**Runs on** every PR and every push. It needs no build, so feedback arrives on
feature branches before a PR exists.

- **`shellcheck`** — `shellcheck --severity=warning scripts/*.sh`. Semgrep has no
  registry ruleset for shell (`p/bash` and `p/shell` are both 404), and the
  scripts here drive the integration suite, the release version stamp and the
  release test report.
- **`actionlint`** — catches what YAML validity cannot: an unknown `uses:` input,
  a bad `needs`, a shell error inside a `run:` block. It also shellchecks those
  `run:` blocks, which is the half ShellCheck above does not cover.
- **`semgrep`** — a full pass to SARIF that never blocks, then a blocking pass on
  ERROR severity. Rulesets `p/rust` and `p/secrets`.

**Gotchas**

- `security-events: write` is scoped to the `semgrep` job. ShellCheck and
  actionlint only read the tree, and actionlint additionally mounts it into a
  container.
- Findings suppressed by a reviewed in-code `// nosemgrep: <rule>` comment are
  stripped before the SARIF upload. GitHub ignores SARIF's `suppressions`
  property, so they would otherwise stay open in the Security tab forever.
- Both the Semgrep and actionlint images are pinned by digest and Dependabot
  does not update `container:`/`docker run` references — they need a manual bump.
- Trivy is **not** here. It lives in `ci.yml`'s `scan` job so it scans the exact
  image that ships; when it was here it built its own image, so the scanned image
  was never the published one.

## `auto-pr.yml` — draft PR opener and branch janitor

**Runs on** a push to any branch except `main`, `dev`, `dependabot/**` and
`release/**`.

- A branch matching `feature|fix|chore|docs|test/…` gets a **draft PR into
  `dev`**, if one does not already exist.
- A branch that does not match is **deleted**. That is the naming enforcement.

**Gotcha:** it uses the built-in `GITHUB_TOKEN`, so the PRs it opens **do not
trigger `ci.yml`**. When you need CI on one, close and reopen it as yourself, or
push a further commit.

## `dependabot-auto-merge.yml` — hands-off dependency bumps

**Runs on** `pull_request`, but acts only when the author is `dependabot[bot]`.

- Reads the bump type via `dependabot/fetch-metadata`.
- **Patch and minor** get `gh pr merge --auto --squash` into `dev`; the merge
  completes once the required checks are green.
- **Major** bumps are left for review — CI green does not prove a major is
  behaviourally safe. Because Dependabot groups per ecosystem, a group containing
  a major waits as a whole, which is the intended effect.

**Gotcha:** auto-merge does nothing unless "Allow auto-merge" is enabled on the
repository.

## `release.yml` — Release, SBOM, and next-cycle prep

**Runs on** a `v*.*.*` tag push (created by `ci.yml`'s `publish`), or manually
via `workflow_dispatch` with an existing tag.

1. **`resolve`** — validates the tag and splits it into `tag` and `version`.
   Both triggers land here, so no later job has to know which one fired, and the
   dispatch input is validated once before it reaches a shell, a checkout and a
   registry reference.
2. **`test-report`** (`contents: read`) — re-runs the full suite with coverage at
   the tag and uploads `test-report.md` and `test-summary.md`. It is a separate
   job because it runs cargo, and the job that attaches the report can write to
   the repository. Report generation is best-effort: a formatting bug must not
   withhold a Release whose tests passed.
3. **`github-release`** (`contents: write`) — refuses a tag that is not on
   `main`, then creates the Release: notes from the version's `CHANGELOG.md`
   section, container pull commands, the manifest digest (best-effort), and the
   test summary if there is one. `0.x` and any `-prerelease` are flagged
   pre-release. Then generates an SBOM from the **published** tag and attaches
   it. Idempotent throughout.
4. **`prepare-dev`** — opens an **auto-merging PR into `dev`** that reopens a
   fresh `## [Unreleased]`, repoints the compare links, and bumps the workspace
   version through `scripts/set-workspace-version.sh`. It pushes only to a
   `release/*` branch, which `auto-pr.yml` ignores.

**Gotchas**

- Never hand-push a `v*` tag; it would produce a Release around every gate.
- Steps read `needs.resolve.outputs.tag`, not `github.ref_name` or `github.sha`.
  On a dispatch the latter two describe the branch the button was pressed on,
  which would make "refuse tags that are not on main" vacuous and the test report
  describe code the release does not contain.
- `prepare-dev` needs `RELEASE_PAT` so its PR triggers CI; without it the PR is
  opened but auto-merge hangs.
- The version bump writes `Cargo.toml` **and** `Cargo.lock`. Bumping only the
  manifest makes every `--locked` job fail before a test runs.

## `release-dispatch.yml` — one-click release, owner-only

**Runs on** manual `workflow_dispatch` with a `bump` input
(`patch`/`minor`/`major`).

- The first step asserts `github.actor == github.repository_owner`; anyone else
  gets a hard error.
- Computes the next version from the higher of the last `v*` tag and the top
  changelog version, read through the validating
  `scripts/changelog-version.sh`.
- Refuses an **empty** `## [Unreleased]` and an already-existing tag. A version
  that documents nothing is worse than no release: the changelog is what tells an
  operator whether to upgrade.
- Stamps the changelog and the workspace version, then opens an auto-merging PR
  into `main` with `--merge`, not `--squash` — `dev → main` is the one place this
  repository keeps history.

From the merge on, the normal path takes over. It is an entry point, not a second
release path: it produces the same PR the manual flow does, and every gate still
runs on it.

## Related

- [`ci-cd.md`](ci-cd.md) — why the pipeline is shaped this way, and the repository settings it needs
- [`releasing.md`](releasing.md) — the release runbook, both paths
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — branching and commit conventions
