# Contributing

Thanks for your interest! Here's the short path from idea to merged PR.

> **AI agents/tools:** read [`AGENTS.md`](../AGENTS.md) first — it's the canonical context (architecture, conventions, workflow, and the hard rules) for working in this repo.

## Quick start

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

## Commits ([Conventional Commits](https://www.conventionalcommits.org/))

```
feat(probes): add ICMP ping probe type
fix(scheduler): prevent probe loop exiting on lagged broadcast
chore(deps): update reqwest to 0.12
```

Types: `feat` · `fix` · `chore` · `docs` · `test` · `refactor` · `perf` · `style`

## Where tests live

| Location | What goes here |
|---|---|
| `#[cfg(test)]` module in the same file | Unit tests — test a single function or type in isolation |
| `huginn/tests/*.rs` | Integration tests — spin up real servers, mock HTTP endpoints, test the binary end-to-end |

As a rule of thumb: if your test needs `tokio::spawn`, a TCP port, or a `WireMock` server, it belongs in `huginn/tests/`. Everything else is a unit test that lives next to the code.

## Local development

**Prerequisites:** Rust stable — the floor is `rust-version` in `Cargo.toml`, see [`versioning.md`](versioning.md) — plus Docker + Compose, `cargo-deny`, optionally `cargo-llvm-cov`.

```bash
cp config/config.example.yaml config/config.yaml

cargo run -- --config config/config.yaml          # pretty output
cargo run -- --config config/config.yaml --output json
HUGINN_UI_ENABLED=true cargo run -- --config config/config.yaml   # debug web UI (no --ui flag; enable via ENV or ui.enabled: true)
```

Full testing guide (TDD workflow, coverage requirements, naming): **[testing.md](testing.md)**

## Gates

| Workflow | Trigger | What it checks |
|---|---|---|
| `ci.yml` | every PR · push → `dev`, `main`, `v*` tags | fmt · clippy · tests (stable + beta canary) · cargo-deny · coverage ≥ 80 % (workspace lines) · image build · Trivy · system integration · **then** publish |
| `security.yml` | every PR · every push | ShellCheck · actionlint · Semgrep SAST |

Trivy lives in `ci.yml`, not in `security.yml`: it scans the exact image tarball
that gets published, and when it sat in `security.yml` it built its own image, so
the scanned image was never the one that shipped.

Publish is a job inside `ci.yml`, gated by `needs` on every check above. It used
to be a separate `docker.yml` that triggered on push in parallel with CI and
depended on nothing, so a red build still shipped `:latest`.

Full pipeline details: **[ci-cd.md](ci-cd.md)**

## Releasing

**You pick a version number. That is the whole manual part.** Never edit
`Cargo.toml`'s version by hand — the post-release housekeeping PR sets it, along
with `Cargo.lock`, and doing it yourself is how the two drift.

Either use Actions → **Release (dispatch)** and pick `patch`/`minor`/`major`, or:

1. On `dev`, rename `## [Unreleased]` in `CHANGELOG.md` to `## [X.Y.Z] - <date>`.
2. Open the release PR `dev → main`. The version gate validates `X.Y.Z` before
   the merge is allowed.
3. After CI is green, merge — a **merge commit**, not a squash.
4. The pipeline publishes the image, creates the tag, opens the GitHub Release,
   and opens the housekeeping PR back into `dev`.

The runbook, with the verification commands, is
[`releasing.md`](releasing.md).

## Questions

Open an issue or discussion on GitHub.

## Related

- [`AGENTS.md`](../AGENTS.md) — the rules, the gates and the doc map
- [`testing.md`](testing.md) — what a test has to look like here
- [`ci-cd.md`](ci-cd.md) — what CI will run on your branch
- [`releasing.md`](releasing.md) — how a release is cut
