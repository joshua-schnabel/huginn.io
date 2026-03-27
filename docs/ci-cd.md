# CI/CD Pipeline

This document describes the branch model, CI/CD pipeline design, and required GitHub repository configuration for hugin.dev.

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

| Job | feature→dev PR | dev→main PR | push dev | push main |
|---|:---:|:---:|:---:|:---:|
| Format & Lint | ✅ | ✅ | ✅ | ✅ |
| Tests (stable) | ✅ | ✅ | ✅ | ✅ |
| Tests (beta) | ✅ | ✅ | ✅ | ✅ |
| Supply-Chain (cargo-deny) | ✅ | ✅ | ✅ | ✅ |
| Code Coverage ≥ 80% | ✅ | ✅ | ✅ | ✅ |
| System Integration Test | ✅ | ✅ | ✅ | ✅ |
| **Semgrep SAST** | ✅ 🚫* | ✅ 🚫* | ✅ | ✅ |
| Trivy CVE Scan (SARIF) | ❌ | ✅ | ❌ | ✅ |
| Trivy blocking scan | ❌ | ✅ 🚫 | ❌ | ✅ |
| Build Docker Image | ✅ | ✅ | ✅ | ✅ |
| Publish to DockerHub | ❌ | ❌ | ✅ :dev + :0.1.0-dev | ✅ :latest + :0.1.0 |

🚫 = Blocks the PR  
🚫* = Blocks only on ERROR-severity findings (hardcoded secrets, critical code patterns)

---

## Workflow Files

### `ci.yml` — Quality Gate
Runs on all pull requests and pushes to `dev`/`main`.

- **check**: `cargo fmt --check` + `cargo clippy -D warnings`
- **test**: `cargo test --all` on Rust stable *and* beta (`fail-fast: false`)
- **supply-chain**: `cargo deny check` — advisory CVEs + licenses + banned crates + registry sources
- **coverage**: `cargo llvm-cov` — fails if any file falls below 80 % region coverage
- **system-integration**: Docker Compose test (InfluxDB + hugin-dev, curl assertions)

None of these jobs use production secrets.

### `sast.yml` — Semgrep Source Code Analysis
Runs on **all** pull requests and pushes (feature→dev and dev→main).

**Two-pass strategy:**
1. **Full scan → SARIF**: All findings uploaded to GitHub Security tab (exit 0, always runs)
2. **Blocking scan** (`--error`): Only ERROR-severity findings → exit code 1 → **PR blocked**

Rulesets used:
- `p/rust` — Rust-specific security patterns (unsafe code, integer overflow, format string injection, …)
- `p/secrets` — Hardcoded secrets, API keys, tokens in source files

### `security.yml` — Trivy CVE Scan
Runs only on pull requests targeting `main` and pushes to `main`.

**Two-pass strategy:**

1. **Full scan → SARIF**: All CRITICAL/HIGH/MEDIUM findings (including unfixed) → GitHub Security tab
2. **Blocking scan**: Only fixable (`ignore-unfixed: true`) CRITICAL/HIGH CVEs → exit 1 → PR blocked

### `docker.yml` — Build & Publish
- **PR events**: Validates that the Dockerfile builds successfully. No credentials, no push.
- **Push to `dev`**: Pushes `:dev` **and** `:x.y.z-dev` (e.g. `0.1.0-dev`) to DockerHub
- **Push to `main`**: Pushes `:latest` + `:x.y.z`. Creates git tag `vx.y.z`.

---

## Security Tools Overview

| Tool | Layer | Finds | Blocks |
|---|---|---|:---:|
| `cargo deny` | Dependencies | Known CVEs (RustSec), bad licenses, unknown registries | ✅ |
| Semgrep `p/rust` | Source code | Unsafe patterns, logic errors, taint flows | ✅ ERROR-level |
| Semgrep `p/secrets` | Source code | Hardcoded API keys, tokens, passwords | ✅ ERROR-level |
| Trivy | Docker image | OS + library CVEs (fixable only) | ✅ only PR→main |

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

## Security Design

### Token / Secret Protection

| Scenario | DOCKERHUB_TOKEN used? | Reason |
|---|:---:|---|
| Feature branch PR to dev | ❌ | `pull_request` event — `if: github.event_name == 'push'` excludes it |
| dev → main PR | ❌ | Same |
| Push to dev | ✅ | Only authenticated pushes to DockerHub |
| Push to main | ✅ | Only authenticated pushes to DockerHub |

Using `pull_request` (not `pull_request_target`) ensures that PRs from external forks run with **no access to repository secrets** by GitHub's own security model.

### Trivy `ignore-unfixed: true`

This flag filters out CVEs that have no patch available. Without it, the scan would block PRs for issues that the project cannot resolve (upstream not fixed yet). With it:
- **Fixable** CRITICAL/HIGH → ❌ MR blocked
- **Unfixed** CRITICAL/HIGH → ⚠️ visible in Security tab, does not block

---

## Required GitHub Branch Protection Rules

These rules must be configured manually in **Settings → Branches**.

### `dev` branch

| Setting | Value |
|---|---|
| Require a pull request before merging | ✅ |
| Required approvals | 1 |
| Dismiss stale reviews when new commits are pushed | ✅ |
| Require status checks to pass | ✅ |
| Required status checks | `Format & Lint`, `Tests (stable)`, `Tests (beta)`, `Supply-Chain Security`, `Code Coverage (≥ 80%)`, `System Integration Test`, `Semgrep SAST` |
| Do not allow bypassing the above settings | ✅ |

### `main` branch

| Setting | Value |
|---|---|
| Require a pull request before merging | ✅ |
| Required approvals | 1 |
| Dismiss stale reviews | ✅ |
| Require status checks to pass | ✅ |
| Required status checks | All `dev` checks + `Trivy CVE Scan`, `Build Docker Image` |
| Require linear history | ✅ (clean merge commit) |
| Do not allow bypassing | ✅ |

---

## DockerHub Setup

Two repository secrets must be configured in **Settings → Secrets → Actions**:

| Secret | Value |
|---|---|
| `DOCKERHUB_USERNAME` | Your DockerHub username |
| `DOCKERHUB_TOKEN` | A DockerHub Access Token (not your password) — create at hub.docker.com → Account Settings → Security |

The published image name will be: `<DOCKERHUB_USERNAME>/hugin-dev`

---

## Running the Security Scan Locally

```bash
# Build the image
docker build -t hugin-dev:local .

# Full scan (informational)
docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
  aquasec/trivy:latest image --severity CRITICAL,HIGH,MEDIUM hugin-dev:local

# Blocking scan (same criteria as CI)
docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
  aquasec/trivy:latest image \
  --severity CRITICAL,HIGH \
  --ignore-unfixed \
  --exit-code 1 \
  hugin-dev:local
```

---

## Adding a New Release

1. Update the version in `CHANGELOG.md` (top entry: `## [x.y.z] - YYYY-MM-DD`)
2. Merge the release PR into `main`
3. The `docker.yml` pipeline reads the version, creates a git tag `vx.y.z`, and publishes `:x.y.z` + `:latest` to DockerHub automatically
