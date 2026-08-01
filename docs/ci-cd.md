# CI/CD Pipeline

This document describes the branch model, CI/CD pipeline design, and required GitHub repository configuration for huginn.io.

---

## Branch Model

```
feature/my-feature
       │
       │  pull request
       ▼
      dev  ──────────────── push → DockerHub :dev
       │
       │  pull request
       ▼
      main ──────────────── push → DockerHub :latest + :x.y.z
```

| Branch | Purpose | Protected |
|---|---|:---:|
| `main` | Production releases | ✅ |
| `dev` | Integration / staging | ✅ |
| `feature/*` | Feature development | ❌ |

**Rule:** No direct pushes to `main` or `dev`. All changes go through a pull request.

---

## Jobs per Trigger

| Job | any PR | push dev | push main | push tag `v*` |
|---|:---:|:---:|:---:|:---:|
| Format & Lint | ✅ | ✅ | ✅ | ✅ |
| Tests (stable) | ✅ | ✅ | ✅ | ✅ |
| Tests (beta, canary — non-blocking) | ✅ | ✅ | ✅ | ✅ |
| Supply-Chain (cargo-deny) | ✅ | ✅ | ✅ | ✅ |
| Code Coverage ≥ 80% | ✅ | ✅ | ✅ | ✅ |
| Version gate (SemVer + successor) | ✅ 🚫† | ➖ | ✅ 🚫 | ➖ |
| **Semgrep SAST** | ✅ 🚫* | ✅ | ✅ | ❌ |
| Build image (per arch, native) | ✅ | ✅ | ✅ | ✅ |
| Trivy CVE Scan (SARIF) | ✅ | ✅ | ✅ | ✅ |
| Trivy blocking scan | ✅ 🚫 | ✅ 🚫 | ✅ 🚫 | ✅ 🚫 |
| System Integration Test | ✅ | ✅ | ✅ | ✅ |
| Publish → DockerHub **+ ghcr.io** | ❌ | ✅ :dev + :0.1.0-dev | ✅ :latest + :0.1.0 | ✅ semver tags |
| GitHub Release (`release.yml`) | ❌ | ❌ | ❌ | ✅ |
| Dev housekeeping PR (`release.yml`) | ❌ | ❌ | ❌ | ✅ |

🚫 = Blocks the PR  
🚫* = Blocks only on ERROR-severity findings (hardcoded secrets, critical code patterns)  
🚫† = Only enforced on PRs whose base is `main` (the release PR); a no-op pass elsewhere  
➖ = Runs but is a deliberate no-op (so it can gate `build` without skipping it)

CI runs on **every** pull request, not only those targeting `main`/`dev`. It was
previously scoped to those two, so feature branches had no gate — which is how
`feature/refactoring` and `dev` once diverged into two different "CI fixes", one
of which silently dropped the keep-alive in `run()` and left the daemon exiting
at startup.

The image is **built exactly once per architecture** (the `build` job) and
uploaded as an artifact. Every later job — `scan` (Trivy), `integration`
(compose test), `push` (skopeo) — consumes *that same tarball*, so the bytes
scanned, tested and published are byte-for-byte identical. `push` needs both
`scan` and `integration`, and `publish` needs `push`, so nothing unscanned can
ever reach DockerHub and the published image is provably the one that passed the
scan. This is also why Trivy now lives in `ci.yml` rather than rebuilding its own
image in `security.yml`.

---

## Workflow Files

### `ci.yml` — Quality Gate and Publish
Runs on every pull request, on pushes to `dev`/`main`, and on `v*.*.*` tags.

- **check**: `cargo fmt --check` + `cargo clippy -D warnings`
- **test**: `cargo test --all` on Rust stable *and* beta (`fail-fast: false`). Beta is a canary for the next compiler and is **non-blocking** (`continue-on-error`): it warns of an upcoming toolchain break without gating a merge.
- **supply-chain**: `cargo deny check` — advisory CVEs + licenses + banned crates + registry sources
- **coverage**: `cargo llvm-cov --fail-under-lines 80` — **workspace-aggregate line** coverage. Not per-file, not regions; see `docs/testing.md`. The `cargo-llvm-cov` binary is compiled once and cached (pinned version); `cargo-deny` is deliberately left on the latest release so it keeps detecting new advisory classes.
- **version-gate**: reads the top version from `CHANGELOG.md` and enforces that it is valid SemVer **and** strictly greater than the latest `v*` git tag. It only enforces for a release context — a PR whose base is `main`, or a push to `main` — and is a no-op pass on every other event. `build` lists it in `needs`, so an invalid release version fails **before** the expensive image build. (It must always run rather than be job-level skipped: a skipped `needs` job would skip `build` too.)
- **build** (matrix, one per architecture on its **native** runner — amd64 on `ubuntu-latest`, arm64 on `ubuntu-24.04-arm`, no QEMU): builds the image **exactly once** into a local tarball (`outputs: type=docker,dest=image.tar`) and uploads it as the `image-<arch>` artifact. Native runners replace QEMU emulation, which had turned the arm64 build into the pipeline's dominant cost.
- **scan** (matrix): downloads the artifact and runs Trivy against it — a full SARIF pass (all CRITICAL/HIGH/MEDIUM → Security tab) and a blocking pass (fixable CRITICAL/HIGH → fails the job). Never rebuilds the image.
- **integration** (matrix, native runner): downloads the artifact, `docker load` + `docker compose --no-build`, and runs the system integration test against the loaded image.
- **push** (matrix, `if: push`): `needs: [scan, integration]`; `skopeo` copies the *scanned+tested* tarball straight to the registry by digest and records the digest. Skipped on PRs, so credentials are never reachable there.
- **publish** (`if: push`): `needs: [push]`; downloads the per-arch digests, assembles the multi-arch DockerHub manifest with `docker buildx imagetools create`, then **mirrors that finished manifest to `ghcr.io` with `skopeo copy --all`** (manifest list + all arch blobs → identical digests, so the ghcr image is byte-identical to the scanned/tested one — no second build), deletes the DockerHub staging tags, and creates the release git tag on main. It publishes **no new bytes** — only tags the digests `push` uploaded, on both registries.

**Publish lives in `ci.yml` on purpose.** As its own `docker.yml` workflow it
triggered on `push` in parallel with CI and depended on nothing, so a commit
with failing tests, clippy or cargo-deny still shipped `:latest` — every
documented gate was bypassable on the only path that reaches users. A
`workflow_run` trigger would fix the ordering but introduces its own trap:
`github.ref` there resolves to the default branch, so each
`github.ref == 'refs/heads/dev'` check would silently read false. A job with
`needs` has neither problem.

`publish` also declares `permissions: contents: write`. It inherits
`contents: read` otherwise, and the release step's `git push origin vX.Y.Z`
returns 403 — which is why the repo went so long with no tags at all.

### `security.yml` — Semgrep SAST

**Semgrep** runs on all pushes and all PRs.

1. **Full scan → SARIF**: All findings uploaded to GitHub Security tab (exit 0, always runs)
2. **Blocking scan** (`--error`): Only ERROR-severity findings → exit code 1 → **PR blocked**

Rulesets: `p/rust` (Rust-specific security patterns) and `p/secrets` (hardcoded secrets, API keys, tokens).

> **Trivy used to live here** and built its *own* image to scan — so the scanned
> image was never the one that shipped. It now runs inside `ci.yml`'s `scan`
> job against the exact tarball that gets published (see above), on every PR and
> every push. `security.yml` is Semgrep-only as a result.

### `release.yml` — GitHub Release + next-cycle prep

Triggered **only by the release tag push** (`v*.*.*`) that `ci.yml`'s `publish`
job creates — so it runs *after* the image is out, and is kept separate from the
image pipeline on purpose.

- **github-release**: extracts the notes for the tagged version from its
  `CHANGELOG.md` section and creates the GitHub Release. `0.x` and any
  `-prerelease` version is flagged as a pre-release. Idempotent (skips if the
  release already exists).
- **prepare-dev**: opens an **auto-merging PR into `dev`** that reopens a fresh
  `## [Unreleased]` block, repoints the compare links at the new tag, and bumps
  `Cargo.toml`'s workspace version to the released version. It never pushes to
  `main` or `dev` directly — only to a `release/*` branch (which `auto-pr.yml`
  ignores) that then flows through normal CI.

---

## Security Tools Overview

| Tool | Layer | Finds | Blocks |
|---|---|---|:---:|
| `cargo deny` | Dependencies | Known CVEs (RustSec), bad licenses, unknown registries | ✅ every PR + push |
| Semgrep `p/rust` | Source code | Unsafe patterns, logic errors, taint flows | ✅ ERROR-level |
| Semgrep `p/secrets` | Source code | Hardcoded API keys, tokens, passwords | ✅ ERROR-level |
| Trivy | Docker image | OS + library CVEs (fixable only) | ✅ every PR + push (in the `scan` job) |

`deny.toml` must stay parseable, not merely present. `severity-threshold`,
`unlicensed` and `copyleft` were removed in cargo-deny ≥ 0.14, and CI installs
the latest version — so the config failed to *load* and the supply-chain gate
was not running at all, while reporting nothing. Once repaired it immediately
surfaced four real advisories. If you edit `deny.toml`, run `cargo deny check`
locally: a config error and a clean scan look very different, and only one of
them is good news.

### deny.toml Customization

To allow an advisory (accepted risk or false positive), add it to `deny.toml`:

```toml
[advisories]
ignore = ["RUSTSEC-2024-0001"]   # add reason in a comment
```

To allow an additional license:

```toml
[licenses]
allow = [
    # ... existing entries ...
    "MPL-2.0",    # add new license here
]
```

To ban a specific crate:

```toml
[bans]
deny = [
    { name = "openssl", reason = "use rustls instead" },
]
```

---

## Adding a New Release

> A standalone step-by-step lives in **[releasing.md](releasing.md)**. The short
> version:

The **only manual step is choosing the version number.** Everything downstream —
tag, image, GitHub Release, and reopening the changelog — is automated. You never
edit `main` or a tag by hand; no bot ever pushes to `main` or `dev`.

1. **On `dev`**, rename the `## [Unreleased]` heading in `CHANGELOG.md` to
   `## [X.Y.Z] - YYYY-MM-DD` (the accumulated entries stay under it). Pick `X.Y.Z`
   per SemVer.
2. **Open the release PR `dev → main`.** The **version gate** validates `X.Y.Z`
   *before* the merge is allowed: it must be valid SemVer and strictly greater
   than the latest `v*` tag. A typo or a non-increasing version fails the PR here.
3. **Merge into `main`.** `ci.yml` then — only after fmt, clippy, tests,
   cargo-deny, coverage, the Trivy scan and the integration test have all passed —
   publishes the multi-arch image to **DockerHub** (`:latest` + `:X.Y.Z`),
   **mirrors it to `ghcr.io`**, and creates the git tag `vX.Y.Z`. No new bytes are
   built; it only tags the already-scanned digests.
4. **`release.yml` fires on that tag push** and: creates the **GitHub Release**
   (notes from the `CHANGELOG.md` section, `0.x`/`-rc` flagged as pre-release),
   and opens an **auto-merging PR into `dev`** that reopens a fresh
   `## [Unreleased]`, fixes the compare links, and bumps `Cargo.toml` to `X.Y.Z`.

> `Cargo.toml` and `CHANGELOG.md` used to drift silently. Now the version gate
> validates the changelog version, and the `release.yml` housekeeping PR bumps
> `Cargo.toml` to match — so they stay in step without manual effort.

> **One-time setup:** after the first publish, set the `huginn` package in
> GitHub → Packages to **Public** (it defaults to private), otherwise
> `docker pull ghcr.io/…` needs auth.
