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
| Tests (beta) | ✅ | ✅ | ✅ | ✅ |
| Supply-Chain (cargo-deny) | ✅ | ✅ | ✅ | ✅ |
| Code Coverage ≥ 80% | ✅ | ✅ | ✅ | ✅ |
| **Semgrep SAST** | ✅ 🚫* | ✅ | ✅ | ❌ |
| Build image (per arch, native) | ✅ | ✅ | ✅ | ✅ |
| Trivy CVE Scan (SARIF) | ✅ | ✅ | ✅ | ✅ |
| Trivy blocking scan | ✅ 🚫 | ✅ 🚫 | ✅ 🚫 | ✅ 🚫 |
| System Integration Test | ✅ | ✅ | ✅ | ✅ |
| Publish to DockerHub | ❌ | ✅ :dev + :0.1.0-dev | ✅ :latest + :0.1.0 | ✅ semver tags |

🚫 = Blocks the PR  
🚫* = Blocks only on ERROR-severity findings (hardcoded secrets, critical code patterns)

CI runs on **every** pull request, not only those targeting `main`/`dev`. It was
previously scoped to those two, so feature branches had no gate — which is how
`feature/refactoring` and `dev` once diverged into two different "CI fixes", one
of which silently dropped the keep-alive in `run()` and left the daemon exiting
at startup.

The image is **built exactly once per architecture** (in the `image` job) and
that single tarball is what Trivy scans, what the system integration test loads,
and what gets pushed. Build, scan, test and publish all operate on the *same
bytes* — so nothing unscanned can ever reach DockerHub, and the published image
is provably the one that passed the scan. This is why Trivy now runs inside
`ci.yml` rather than rebuilding its own image in `security.yml`.

---

## Workflow Files

### `ci.yml` — Quality Gate and Publish
Runs on every pull request, on pushes to `dev`/`main`, and on `v*.*.*` tags.

- **check**: `cargo fmt --check` + `cargo clippy -D warnings`
- **test**: `cargo test --all` on Rust stable *and* beta (`fail-fast: false`)
- **supply-chain**: `cargo deny check` — advisory CVEs + licenses + banned crates + registry sources
- **coverage**: `cargo llvm-cov --fail-under-lines 80` — **workspace-aggregate line** coverage. Not per-file, not regions; see `docs/testing.md`. The `cargo-llvm-cov` binary is compiled once and cached (pinned version); `cargo-deny` is deliberately left on the latest release so it keeps detecting new advisory classes.
- **image** (matrix, one per architecture on its **native** runner — amd64 on `ubuntu-latest`, arm64 on `ubuntu-24.04-arm`, no QEMU): builds the image **exactly once** into a local tarball (`outputs: type=docker,dest=image.tar`), then does everything against *that one file* — Trivy scans it (SARIF + a blocking pass on fixable CRITICAL/HIGH), `docker load` + `docker compose --no-build` runs the system integration test on it, and — push only — `skopeo` copies those exact bytes straight from the tarball to the registry by digest. Replaces the three separate image builds (integration test, Trivy, push) the pipeline used to do. Native runners replace QEMU emulation, which had turned the arm64 build into the pipeline's dominant cost.
- **publish**: downloads the per-arch digests, assembles the multi-arch manifest with `docker buildx imagetools create`, tags it, and creates the release git tag on main. `needs: [image]` (which needs every gate above), and `if: github.event_name == 'push'` — so it never runs on a PR and never sees credentials there.

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
> image was never the one that shipped. It now runs inside `ci.yml`'s `image`
> job against the exact tarball that gets published (see above), on every PR and
> every push. `security.yml` is Semgrep-only as a result.

---

## Security Tools Overview

| Tool | Layer | Finds | Blocks |
|---|---|---|:---:|
| `cargo deny` | Dependencies | Known CVEs (RustSec), bad licenses, unknown registries | ✅ every PR + push |
| Semgrep `p/rust` | Source code | Unsafe patterns, logic errors, taint flows | ✅ ERROR-level |
| Semgrep `p/secrets` | Source code | Hardcoded API keys, tokens, passwords | ✅ ERROR-level |
| Trivy | Docker image | OS + library CVEs (fixable only) | ✅ every PR + push (in the `image` job) |

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

1. Update the version in `CHANGELOG.md` (top entry: `## [x.y.z] - YYYY-MM-DD`)
2. Merge the release PR into `main`
3. The `publish` job in `ci.yml` reads the version, creates the git tag `vx.y.z`, and assembles the multi-arch manifest from the already-built, already-scanned per-arch images — but only after fmt, clippy, tests, cargo-deny, coverage, the Trivy scan and the system integration test have all passed. It publishes no new bytes; it only tags the digests the `image` job pushed.

> The version lives in both `Cargo.toml` and `CHANGELOG.md`, and only the latter
> drives the tag. They can silently disagree; the CHANGELOG wins.
