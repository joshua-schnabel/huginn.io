# Contributing to huginn

Thanks for your interest! Here's the short path from idea to merged PR.

> **AI agents/tools:** read [`AGENTS.md`](../AGENTS.md) first — it's the canonical context (architecture, conventions, workflow, and the hard rules) for working in this repo.

## Quick Start

```bash
# Fork + clone, then:
git checkout dev
git checkout -b feature/<your-description>

# Make your changes, then:
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check        # supply-chain: licenses + CVEs

# Commit and push, then open a PR against dev
```

## Branching

| Branch | Purpose | Merge from |
|---|---|---|
| `main` | Stable releases | `dev` only (merge commit) |
| `dev` | Integration / latest | `feature/*` (squash merge) |
| `feature/<name>` | Work-in-progress | branch from `dev` |

Branch naming: `feature/`, `fix/`, `chore/`, `docs/`, `test/`

## Commit Messages ([Conventional Commits](https://www.conventionalcommits.org/))

```
feat(probes): add ICMP ping probe type
fix(scheduler): prevent probe loop exiting on lagged broadcast
chore(deps): update reqwest to 0.12
```

Types: `feat` · `fix` · `chore` · `docs` · `test` · `refactor` · `perf` · `style`

## Where Tests Live

| Location | What goes here |
|---|---|
| `#[cfg(test)]` module in the same file | Unit tests — test a single function or type in isolation |
| `huginn/tests/*.rs` | Integration tests — spin up real servers, mock HTTP endpoints, test the binary end-to-end |

As a rule of thumb: if your test needs `tokio::spawn`, a TCP port, or a `WireMock` server, it belongs in `huginn/tests/`. Everything else is a unit test that lives next to the code.

## Local Development

**Prerequisites:** Rust stable (MSRV **1.88**, see `rust-version` in `Cargo.toml` and [`versioning.md`](versioning.md)), Docker + Compose, `cargo-deny`, optionally `cargo-llvm-cov`.

```bash
cp config/config.example.yaml config/config.yaml

cargo run -- --config config/config.yaml          # pretty output
cargo run -- --config config/config.yaml --output json
HUGINN_UI_ENABLED=true cargo run -- --config config/config.yaml   # debug web UI (no --ui flag; enable via ENV or ui.enabled: true)
```

Full testing guide (TDD workflow, coverage requirements, naming): **[testing.md](testing.md)**

## CI Workflows

| Workflow | Trigger | What it checks |
|---|---|---|
| `ci.yml` | every PR · push → `dev`, `main`, `v*` tags | fmt · clippy · tests (stable+beta) · cargo-deny · coverage ≥80% (workspace lines) · system integration · **then** DockerHub publish |
| `security.yml` | every PR · push → any branch | Semgrep SAST (all) · Trivy CVE scan (every PR + push to `main`; blocks on fixable CRITICAL/HIGH) |

Publish is a job inside `ci.yml`, gated by `needs` on every check above. It used
to be a separate `docker.yml` that triggered on push in parallel with CI and
depended on nothing, so a red build still shipped `:latest`.

Full pipeline details: **[ci-cd.md](ci-cd.md)**

## Release Process

1. On `dev`: bump `version` in `Cargo.toml`, promote `[Unreleased]` in `CHANGELOG.md`
2. Commit: `chore: release vX.Y.Z` → push → open PR `dev` → `main`
3. After CI green + review → merge (merge commit, not squash)
4. CI reads the version from `CHANGELOG.md`, creates the git tag, builds and pushes the Docker image automatically

## Questions?

Open an issue or discussion on GitHub.