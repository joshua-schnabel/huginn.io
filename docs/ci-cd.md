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
| Dependency Audit | ✅ | ✅ | ✅ | ✅ |
| Code Coverage ≥ 80% | ✅ | ✅ | ✅ | ✅ |
| System Integration Test | ✅ | ✅ | ✅ | ✅ |
| Trivy CVE Scan (SARIF) | ❌ | ✅ | ❌ | ✅ |
| Trivy blocking scan | ❌ | ✅ 🚫 | ❌ | ✅ |
| Build Docker Image | ✅ | ✅ | ✅ | ✅ |
| Publish to DockerHub | ❌ | ❌ | ✅ :dev | ✅ :latest |

🚫 = Blocks the PR if fixable CRITICAL/HIGH CVEs are found

---

## Workflow Files

### `ci.yml` — Quality Gate
Runs on all pull requests and pushes to `dev`/`main`.

- **check**: `cargo fmt --check` + `cargo clippy -D warnings`
- **test**: `cargo test --all` on Rust stable *and* beta (`fail-fast: false`)
- **audit**: `cargo audit` — checks for known vulnerabilities in dependencies
- **coverage**: `cargo llvm-cov` — fails if any file falls below 80 % region coverage
- **system-integration**: Docker Compose test (InfluxDB + hugin-dev, curl assertions)

None of these jobs use production secrets. The system integration test uses a hardcoded test-only InfluxDB token.

### `security.yml` — Trivy CVE Scan
Runs only on pull requests targeting `main` and pushes to `main`.

**Two-pass strategy:**

1. **Full scan → SARIF**: All CRITICAL/HIGH/MEDIUM findings (including unfixed) are uploaded to the GitHub Security tab. Results appear inline on the pull request.
2. **Blocking scan**: Only fixable (`ignore-unfixed: true`) CRITICAL/HIGH CVEs. Returns exit code 1 if any are found → PR is blocked.

This means:
- CVEs with no available fix are **visible but do not block** (informational)
- CVEs that *can* be fixed **always block** the merge to main

### `docker.yml` — Build & Publish
- **PR events**: Validates that the Dockerfile builds successfully. Uses GitHub Actions layer cache. No credentials, no push.
- **Push to `dev`**: Builds multi-platform image and pushes to DockerHub with the `:dev` tag.
- **Push to `main`**: Pushes `:latest` + `:x.y.z` (version read from `CHANGELOG.md`). Creates a matching git tag if it doesn't exist yet.

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
| Required status checks | `Format & Lint`, `Tests (stable)`, `Tests (beta)`, `Dependency Audit`, `Code Coverage (≥ 80%)`, `System Integration Test` |
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
