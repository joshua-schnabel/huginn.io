# AGENTS.md

Canonical context for AI coding agents working on **huginn.io**. This is the
single source of truth for how to work here; every tool (Claude Code, Cursor,
Aider, Gemini CLI, …) should read it first. Human-facing depth lives in the docs
linked at the bottom — this file summarises and points, it does not duplicate.

---

## 1. What this project is

huginn.io is a lightweight **uptime & latency monitor** written in Rust. It runs
configurable probes (TCP · HTTP/HTTPS · SMTP · IMAP · UDP · DNS) on a schedule,
measures up/down + response time, and writes results to **InfluxDB** via batched
line-protocol HTTP. An optional Axum **debug UI** streams live results over SSE.
It ships as a **distroless, nonroot** multi-arch Docker image. It's a Cargo
workspace of five crates (see §4).

---

## 2. Working with the maintainer

- **Language:** reply to the maintainer (**Joshua**) in **German**. Keep
  everything you commit — code, comments, commit messages, docs, and this file —
  in **English**.
- **Autonomy (solo project, Joshua is sole maintainer):** act pragmatically and
  autonomously *within the guardrails in §3*. Execute the task, keep Joshua in
  the loop with concise progress, and ask only at genuine forks. Land work as a
  **pull request**, never as a direct change to a protected branch.
- **Verify before changing:** confirm versions, API shapes, and facts against the
  actual source / official docs before editing. Guessing has caused real
  breakage here — when unsure, check, don't assume.
- **Security is a first-class priority.** huginn handles credentials and ships a
  container, so weigh **every** change through a security lens (secrets never in
  ENV or logs, least privilege, no new attack surface) and explicitly call out
  anything with a security dimension. When in doubt, choose the safer option and
  flag it. See §9 — it is not optional polish.
- **Don't duplicate:** prefer reusing existing helpers/patterns; when documenting,
  link to the canonical doc rather than copying it.

---

## 3. Hard rules — never do these without explicit approval

These are stops, not preferences. Ask first, every time:

1. **Never push to `main` or `dev`.** Both are protected; all changes go through a
   PR. Work on a `feature|fix|chore|docs|test/<name>` branch.
2. **Never merge or approve PRs.** Opening PRs is fine; merging/approving is
   Joshua's decision.
3. **Never change secrets or repository/ruleset settings.** No creating or editing
   Actions secrets, branch protection, rulesets, or repo config.
4. **Never add a new dependency, and never rewrite git history / force-push**
   without asking. New crates change the supply-chain surface; history rewrites
   are destructive.

Everything else (editing code/docs, opening PRs, running the test/lint gates) is
fair game — do it.

---

## 4. Architecture & where things live

Cargo workspace; each crate has one bounded responsibility:

| Crate | Role |
|---|---|
| `huginn/` | Binary entry point — CLI, config load, logging init, the probe **scheduler**, graceful shutdown, orchestration (`main.rs`, `scheduler.rs`) |
| `crates/huginn-core/` | Shared types (`ProbeResult`), config structs, `HuginError`, the `EventHub` (`config.rs`, `error.rs`, `event.rs`, `types.rs`) |
| `crates/huginn-probes/` | `Probe` trait + `ProbeRegistry` + per-protocol executors (`tcp/http/smtp/imap/udp/dns.rs`, `registry.rs`) |
| `crates/huginn-influx/` | InfluxDB line-protocol writer — batching, bounded retry queue, HTTP POST (`writer.rs`, `queue.rs`) |
| `crates/huginn-web/` | Optional Axum debug server — `/health`, `/metrics/latest`, `/events` SSE (`server.rs`, `sse.rs`, `state.rs`) |

**Data flow — `EventHub` is the sole pub/sub bus** (a tokio `broadcast` channel):

```
main.rs → load AppConfig (YAML + ENV) → create EventHub → spawn tasks:
   Scheduler ──publishes──► EventHub ──subscribes──► Console output
                                              └─────► InfluxDB writer
                                              └─────► Web UI (if enabled)
```

The **scheduler is the only publisher**; everything else subscribes. Shutdown is
a separate `broadcast::channel<()>` driven by `tokio::signal::ctrl_c()`.

Other locations: `config/` (example + integration YAML), `scripts/integration-test.sh`,
`Dockerfile` + `docker-compose*.yml`, `deny.toml`, `.github/workflows/`.

---

## 5. Commands (the gates)

Run these before committing; CI enforces the same:

```bash
cargo fmt --all -- --check                                   # format (CI check)
cargo clippy --all-targets --all-features -- -D warnings     # lint: warnings = errors
cargo test --all --locked                                    # all tests
cargo deny check                                             # supply-chain: CVEs, licenses, banned crates
cargo llvm-cov --all --lcov --output-path lcov.info --fail-under-lines 80   # ≥80% workspace-line coverage
cargo build --release --locked                               # production binary
```

Run locally: `cargo run -- --config config/config.yaml [--output json]`. The web
UI has **no CLI flag** — enable via `HUGINN_UI_ENABLED=true` or `ui.enabled: true`.
It binds `127.0.0.1` by default (`ui.bind` / `HUGINN_UI_BIND`); **in a container
it needs `0.0.0.0`**, or the published port reaches nothing.
System integration test (Docker): `docker compose -f docker-compose.integration.yml up -d --build`
then `bash scripts/integration-test.sh`. Copy `config/config.example.yaml` →
`config/config.yaml` first.

---

## 6. Conventions

### Coding style
- **Match the surrounding code** — its naming, module layout, comment density and
  idioms. Consistency beats personal preference.
- `snake_case` for items/modules, `CamelCase` for types; descriptive names
  (`response_ms`, not `r`). Test names describe behaviour in plain English
  (`fails_on_timeout`).
- **No `unwrap()` / `expect()` / `panic!` in non-test code** — return a `Result`
  and propagate with `?`. Panics are for tests and genuinely-unreachable
  invariants only (with a comment saying why).
- Keep functions small and single-purpose; keep each crate's public surface tight
  (`pub` only what other crates need).
- Doc-comment (`///`) public items; write comments about the *why*, not the *what*.
- `cargo fmt` defaults apply (there is no `rustfmt.toml`), and
  `clippy -D warnings` must be clean — **fix** clippy rather than `#[allow]`-ing it
  away; if an allow is truly needed, add a one-line reason.
- Prefer the standard library or already-present crates; **adding a dependency
  needs approval** (§3). Avoid `unsafe`.

### Project idioms
- **Errors:** custom `HuginError` (`thiserror`) in `huginn-core::error`, with
  `type Result<T> = std::result::Result<T, HuginError>`; `anyhow` only at the
  binary boundary. Probe failures return `ProbeResult::failure()` and are logged
  with `error!()` — **they never panic**.
- **Async:** single tokio runtime (`#[tokio::main]`); each long-running component
  is a `tokio::spawn`-ed task; `tokio::select!` multiplexes the shutdown signal
  with timers/channels; async tests use `#[tokio::test]`.
- **Logging:** `tracing` + `tracing-subscriber` (`EnvFilter`, `RUST_LOG`); pretty
  by default, JSON via `--output json`; structured fields
  (`info!(probe = %name, response_ms, "probe UP")`).
- **Config & secrets:** YAML + `HUGINN_*` ENV; precedence CLI > ENV > YAML >
  default. **The InfluxDB token must be read from a file — never from ENV**
  (`influx.token_file`; Docker secret at `/run/secrets/influx_token`, tmpfs, mode
  `0600`). Missing token file = immediate fatal error.
- **InfluxDB writes:** raw line protocol, no SDK. `run_batcher` groups results
  (flush on `batch_size` **or** `batch_timeout_ms`) and never awaits I/O;
  `run_writer` drains a bounded `RetryQueue`. Retryable (transport/5xx/429/408)
  → exponential backoff; 4xx → drop the batch; memory-bounded by
  `max_buffered_bytes` (drop-oldest).
- **MSRV = Rust 1.88**, edition 2021 (`[workspace.package]`). CI runs floating
  stable, so **the Docker build is the real MSRV gate** — a dep that raises the
  MSRV passes CI but breaks the image. Keep the Dockerfile builder in step.
- **Testing:** unit tests in inline `#[cfg(test)]` next to the code; integration
  tests in `huginn/tests/*.rs` (anything needing `tokio::spawn`, a TCP port, or
  `wiremock` — never hit real external services). **Don't sleep — poll** to avoid
  flakes (see `docs/testing.md`). ≥80% aggregate workspace-line coverage.
- **No OpenSSL:** TLS is **rustls** only.

---

## 7. Git & PR workflow

- Branch off `dev` with a valid prefix: `feature/` · `fix/` · `chore/` · `docs/`
  · `test/`. Pushing such a branch makes `auto-pr.yml` open a draft PR into `dev`
  automatically; a branch that doesn't match the prefix is auto-deleted.
- Flow: `feature/* → dev` (**squash** merge) → `main` (**merge commit**). No
  direct pushes to `dev`/`main` (see §3).
- **Conventional Commits:** `feat · fix · chore · docs · test · refactor · perf ·
  style`. End commit bodies with the required `Co-Authored-By` trailer.
- Full contributor guide: [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md).

---

## 8. CI/CD & releasing

Everything runs through **`ci.yml`** on every PR + push. The image is **built
once per arch** into a tarball; `scan` (Trivy), `integration` (compose), `push`
(skopeo by digest) and `publish` all consume *that same artifact*, so the bytes
scanned, tested and published are byte-identical. `publish` assembles the
multi-arch DockerHub manifest, **mirrors it to `ghcr.io`**, and tags `vX.Y.Z`.
`security.yml` is **Semgrep-only**; `cargo-deny` gates the supply chain. Every
workflow is described one-by-one in [`docs/workflows.md`](docs/workflows.md).

**Releasing** (details in [`docs/releasing.md`](docs/releasing.md) and
[`docs/ci-cd.md`](docs/ci-cd.md)):
- **One-click (recommended):** Actions → **Release (dispatch)** → pick
  `patch`/`minor`/`major`. Owner-only. It computes the version, stamps
  `CHANGELOG.md` + `Cargo.toml`, and opens an auto-merging PR into `main`.
- **Manual:** rename `## [Unreleased]` → `## [X.Y.Z] - <date>` on `dev`, open a
  PR `dev → main`; the **version gate** blocks a merge unless the version is
  valid SemVer and greater than the last tag.
- After the `main` merge: image published (DockerHub + ghcr) + tag created, then
  `release.yml` creates the GitHub Release and opens the dev housekeeping PR.
- **Never hand-push `v*` tags** — the pipeline creates them after every gate.

---

## 9. Security posture — high priority

Security is a first-class concern here (see §2), not an afterthought. Any change
that touches secrets, the container, network exposure, dependencies, or workflow
permissions must be reasoned about explicitly and flagged in the PR.

- Secrets **file-only, never in ENV**; never commit secret values or add them to
  Docker/compose ENV, and never log them. Don't add `--privileged`, `--cap-add`.
- Distroless + nonroot runtime; config mounted read-only; TLS is **rustls** only.
- Least-privilege workflow `permissions:` — grant the minimum a job needs.
- Gates: **cargo-deny** (CVEs + license allow-list in `deny.toml`), **Semgrep**
  (`p/rust` + `p/secrets`, ERROR blocks), **Trivy** (image CVEs, fixable
  CRITICAL/HIGH block). More: [`docs/hardening.md`](docs/hardening.md)
  (practices) and [`docs/SECURITY.md`](docs/SECURITY.md) (reporting policy).

---

## 10. Doc map

| Topic | Read |
|---|---|
| Contributor workflow, branching, commits | [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) |
| CI/CD pipeline, repo configuration | [`docs/ci-cd.md`](docs/ci-cd.md) |
| Every workflow explained (humans + AI) | [`docs/workflows.md`](docs/workflows.md) |
| Release runbook (one-click + manual) | [`docs/releasing.md`](docs/releasing.md) |
| Testing pyramid, TDD, coverage, no-sleep rule | [`docs/testing.md`](docs/testing.md) |
| Security practices (hardening) | [`docs/hardening.md`](docs/hardening.md) |
| Reporting a vulnerability (policy) | [`docs/SECURITY.md`](docs/SECURITY.md) |
| Config reference (YAML + ENV) | [`docs/configuration.md`](docs/configuration.md) |
| InfluxDB setup & data model | [`docs/influxdb.md`](docs/influxdb.md) |
| Troubleshooting | [`docs/troubleshooting.md`](docs/troubleshooting.md) |
| Supply-chain policy | [`deny.toml`](deny.toml) |
