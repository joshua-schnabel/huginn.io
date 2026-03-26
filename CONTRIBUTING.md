# Contributing to hugin.dev

Thank you for your interest in contributing! This document explains the development workflow, branching strategy, and release process.

---

## Branching Strategy

```
feature/xyz ──┐
feature/abc ──┤  PR → dev  ──────────── PR → main
              └► dev (integration)          │
                  │                         │
                  │ push → GHCR :dev        │ push → GHCR :latest + :vX.Y.Z
                  │                         │        (version from CHANGELOG.md)
                  └─────────────────────────┘
```

| Branch | Purpose | Protected | Merge from |
|---|---|---|---|
| `main` | Stable releases | ✅ yes | `dev` only (via PR) |
| `dev` | Integration / latest dev build | ✅ yes | `feature/*` only (via PR) |
| `feature/<name>` | New features / bugfixes | ❌ no | branch from `dev` |

**Rules:**
- Never push directly to `main` or `dev`
- All changes go through a Pull Request
- PRs require at least one review before merging
- Squash-merge feature branches into `dev` to keep history clean
- Merge-commit (no squash) when merging `dev` → `main`

---

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
- `cargo-audit` — `cargo install cargo-audit`

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
cargo audit
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
   - Pushes `ghcr.io/OWNER/hugin-dev:0.2.0` and `ghcr.io/OWNER/hugin-dev:latest`

---

## CI/CD Overview

| Workflow | Trigger | What happens |
|---|---|---|
| `ci.yml` | push/PR → `dev`, `main` | fmt ✓ clippy ✓ test (stable+beta) ✓ cargo-audit ✓ |
| `docker.yml` | push → `dev` | build + Trivy scan + push `:dev` to GHCR |
| `docker.yml` | push → `main` | build + Trivy scan + read version from CHANGELOG + create git tag + push `:vX.Y.Z` + `:latest` to GHCR |

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
