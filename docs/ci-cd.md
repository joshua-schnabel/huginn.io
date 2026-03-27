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

## Adding a New Release

1. Update the version in `CHANGELOG.md` (top entry: `## [x.y.z] - YYYY-MM-DD`)
2. Merge the release PR into `main`
3. The `docker.yml` pipeline reads the version, creates a git tag `vx.y.z`, and publishes `:x.y.z` + `:latest` to DockerHub automatically
