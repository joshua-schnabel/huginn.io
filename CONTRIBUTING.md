# Contributing to hugin.dev

## Branching Strategy

```
feature/xyz ──┐
feature/abc ──┤  PR → dev  ──────────── PR → main
              └► dev (integration)          │
                  │                         │
                  │ push → DockerHub :dev   │ push → DockerHub :latest + :vX.Y.Z
                  │        + :X.Y.Z-dev     │        (version from CHANGELOG.md)
                  └─────────────────────────┘
```

| Branch | Purpose | Protected | Merge from |
|---|---|---|---|
| `main` | Stable releases | ✅ | `dev` only (via PR) |
| `dev` | Integration / latest dev build | ✅ | `feature/*` only (via PR) |
| `feature/<name>` | New features / bugfixes | ❌ | branch from `dev` |

- Never push directly to `main` or `dev`
- Squash-merge feature branches into `dev`
- Merge-commit (no squash) when merging `dev` → `main`

## Branch Naming

```
feature/<short-description>    # new features
fix/<short-description>        # bugfixes
chore/<short-description>      # maintenance, deps, CI
docs/<short-description>       # documentation only
test/<short-description>       # tests only
```

Examples:
- `feature/smtp-tls-probe`
- `fix/scheduler-shutdown-race`
- `chore/update-reqwest-0.12`
- `docs/grafana-dashboard-example`

---

## Commit Messages (Conventional Commits)

Follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<optional scope>): <short description>

[optional body]

[optional footer]
Co-authored-by: ...
```

**Types:**
| Type | When to use |
|---|---|
| `feat` | New feature or probe type |
| `fix` | Bug fix |
| `chore` | Build, deps, CI, tooling |
| `docs` | Documentation only |
| `test` | Tests only (no production code) |
| `refactor` | Refactoring without behavior change |
| `perf` | Performance improvement |
| `style` | Formatting, no logic change |

Examples:
```
feat(probes): add ICMP ping probe type
fix(scheduler): prevent probe loop from exiting on lagged broadcast
chore(deps): update reqwest to 0.12
test(influx): add line protocol escaping edge cases
```

---

## Local Development

### Prerequisites
- Rust (stable) — `rustup install stable`
- Docker + Docker Compose (for integration testing with InfluxDB)
- `cargo-deny` — `cargo install cargo-deny --locked`

### Run tests

See **[docs/testing.md](docs/testing.md)** for the full testing guide (test pyramid, TDD workflow, coverage requirements, naming conventions).

```bash
# All tests (unit + integration)
cargo test --workspace

# Single crate
cargo test -p hugin-probes

# Specific test
cargo test -p hugin-probes fails_on_empty_banner

# Watch mode (requires cargo-watch)
cargo watch -x "test --workspace"

# Coverage report (requires cargo-llvm-cov)
cargo llvm-cov --workspace --open
```

### Linting & Formatting

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
```

All three must be clean before opening a PR — CI enforces this.

### Run locally

```bash
# Copy example config
cp config/config.example.yaml config/config.yaml
# Edit config.yaml with your targets and InfluxDB settings

# Run with pretty output (default)
cargo run -- --config config/config.yaml

# Run with JSON output
cargo run -- --config config/config.yaml --output json

# Run with debug UI
cargo run -- --config config/config.yaml --ui
```

---

## Release Process

Releases are triggered by merging `dev` into `main`. The version is read from `CHANGELOG.md` — no manual git tagging required.

### Steps

1. **Prepare release on `dev`:**

   Edit `CHANGELOG.md` — promote `[Unreleased]` to a versioned entry:

   ```markdown
   ## [Unreleased]

   ## [0.2.0] - 2025-04-10    ← add this line, move items from Unreleased
   ### Added
   - ICMP ping probe
   ### Fixed
   - Scheduler shutdown race condition
   ```

   Also bump the version in `Cargo.toml` (workspace root) to match:
   ```toml
   [workspace.package]
   version = "0.2.0"
   ```

2. **Commit and push:**
   ```bash
   git add CHANGELOG.md Cargo.toml Cargo.lock
   git commit -m "chore: release v0.2.0"
   git push origin dev
   ```

3. **Open PR: `dev` → `main`**
   - Title: `release: v0.2.0`
   - After review and CI green → merge (merge commit, not squash)

4. **CI does the rest automatically:**
   - Reads version `0.2.0` from `CHANGELOG.md`
   - Creates git tag `v0.2.0`
   - Builds Docker image
   - Pushes `your-dockerhub-user/hugin-dev:0.2.0` and `:latest` to DockerHub

---

## CI/CD Overview

| Workflow | Trigger | What happens |
|---|---|---|
| `ci.yml` | push/PR → `dev`, `main` | fmt · clippy · test (stable+beta) · cargo-deny · coverage · system integration |
| `sast.yml` | all PRs + pushes | Semgrep SAST (p/rust + p/secrets) → SARIF + blocking on ERROR |
| `security.yml` | PR/push → `main` | Trivy CVE scan — blocks if fixable CRITICAL/HIGH found |
| `docker.yml` | push → `dev`/`main` | Build + DockerHub publish (`:dev` + `:X.Y.Z-dev` / `:latest` + `:X.Y.Z`) |
| `.gitlab-ci.yml` | all branches (GitLab) | Same quality gates + DinD system integration + DockerHub publish |

---

## Code Style

- `cargo fmt --all` — enforced by CI
- `cargo clippy -- -D warnings` — all clippy warnings are errors in CI
- Comments only where the code is not self-explanatory
- Tests alongside implementation (`#[cfg(test)]`) for unit tests
- Integration tests in `hugin-dev/tests/` following TDD: write test first, then implementation

---

## Questions?

Open an issue or discussion on GitHub.
