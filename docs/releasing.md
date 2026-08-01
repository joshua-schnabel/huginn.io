# Releasing a New Version

The **only manual decision is the version number.** Everything else — validation,
the multi-arch image on DockerHub and ghcr.io, the git tag, the GitHub Release,
and reopening the changelog for the next cycle — is automated.

You never edit `main` or a tag by hand, and no bot ever pushes to `main` or `dev`.

---

## TL;DR

```
1. On dev:  ## [Unreleased]   →   ## [X.Y.Z] - YYYY-MM-DD   (in CHANGELOG.md)
2. Open a PR  dev → main       (the version gate validates X.Y.Z before merge)
3. Merge it.                   (everything below happens automatically)
```

---

## Step by step

### 1. Pick the version and update the changelog (on `dev`)

Decide `X.Y.Z` per [SemVer](https://semver.org):

- **patch** (`0.1.0 → 0.1.1`) — bug fixes only
- **minor** (`0.1.0 → 0.2.0`) — new features, backwards-compatible
- **major** (`0.9.0 → 1.0.0`) — breaking changes

In `CHANGELOG.md`, rename the top `## [Unreleased]` heading to the release
heading, keeping the accumulated entries under it:

```diff
- ## [Unreleased]
+ ## [0.2.0] - 2026-08-01
```

Get that change onto `dev` the normal way (a `feature/*` or `chore/*` PR, or as
part of the release PR itself). The version **must** be:

- **valid SemVer**, and
- **strictly greater** than the latest `vX.Y.Z` git tag.

### 2. Open the release PR: `dev → main`

```bash
gh pr create --base main --head dev --title "release: v0.2.0"
```

The **version gate** runs on this PR and **blocks the merge** if the version is
invalid or not greater than the last tag — so a typo or a forgotten bump fails
here, before anything ships. All the usual gates (fmt, clippy, tests,
cargo-deny, coverage, Trivy, integration) run too.

### 3. Merge

Merge the PR once it's green. That's the last thing you do by hand.

---

## What happens automatically after the merge

```
merge dev → main
      │
      ▼
ci.yml (main push)                          → DockerHub  :latest  :0.2.0
      ├─ builds nothing new — tags the already-scanned digests
      ├─ mirrors the exact image to ghcr.io → ghcr.io/.../huginn :latest :0.2.0
      └─ creates the git tag                → v0.2.0
                                                 │
                                                 ▼ (tag push)
release.yml
      ├─ GitHub Release v0.2.0  (notes taken from the CHANGELOG.md section)
      └─ opens a PR into dev that:
           • reopens a fresh ## [Unreleased]
           • fixes the compare links
           • bumps Cargo.toml to 0.2.0
```

- The published image is **byte-identical** to the one that was scanned and
  integration-tested — it is never rebuilt for publishing.
- `0.x` versions and any pre-release (`-rc.1`, `-beta`, …) are flagged as a
  **pre-release** on GitHub.
- The dev housekeeping PR **auto-merges** if a `RELEASE_PAT` secret is configured
  (see below); otherwise it stays open for you to merge with one click.

---

## Verify a release

```bash
# Image is on both registries, multi-arch (two entries), same digests:
docker buildx imagetools inspect docker.io/jschnabel/huginn:0.2.0
docker buildx imagetools inspect ghcr.io/joshua-schnabel/huginn:0.2.0

# Tag and GitHub Release exist:
git ls-remote --tags origin v0.2.0
gh release view v0.2.0

# dev was reopened for the next cycle (fresh Unreleased + Cargo bump):
gh pr list --base dev --search "prepare next cycle"
```

---

## Rules & gotchas

- **Don't hand-push `v*` tags.** Tags are created only by the pipeline after
  every gate passes. A manually pushed tag would create a Release with an image
  that skipped the gates.
- **`Cargo.toml` and `CHANGELOG.md` stay in sync automatically** — the version
  gate validates the changelog version, and the housekeeping PR bumps
  `Cargo.toml` to match. You don't edit `Cargo.toml`'s version by hand.
- **First release ever:** with no existing tag, the gate only checks that the
  version is valid SemVer (there's nothing to be "greater than").
- **A `main` push without a version bump** is safe: the tag already exists, so
  the tag/release steps are skipped (idempotent). Nothing breaks; nothing new is
  released.
- **Re-running a release** (re-run of a `main` push) is safe for the same reason.
