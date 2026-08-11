# AGENTS.md

Canonical context for AI coding agents working on **huginn.io**. Single source of
truth for how to work here; every tool (Claude Code, Cursor, Aider, Gemini CLI, …)
should read it first. Human-facing depth lives in the linked docs — this file
summarises and points, it does not duplicate.

---

## 1. What this project is

huginn.io is a lightweight **uptime and latency monitor** written in Rust. It
runs configurable probes (TCP · HTTP/HTTPS · SMTP · IMAP · UDP · DNS · TLS
certificate expiry) on a schedule, measures up/down and response time, and
writes results to **InfluxDB** as batched line protocol. An optional Axum debug
UI streams live results over SSE, and a separately gated Prometheus endpoint
exposes the same data as gauges. It ships as a **distroless, nonroot** multi-arch
container image, from a Cargo workspace of five crates.

**Status: released, with a stable major version out.** The image is on Docker Hub
and mirrored to ghcr. Read the current version from `CHANGELOG.md`'s topmost
`## [x.y.z]` heading rather than from any sentence here — a version written into
prose is wrong the morning after a release (§7), as the list of tags that used to
stand in this paragraph duly was. [`docs/roadmap.md`](docs/roadmap.md) carries
what is still open and is the one place it is tracked — do not restate it here,
it goes stale. Start with [`docs/architecture.md`](docs/architecture.md).

Sibling project: [muninn.io](https://github.com/joshua-schnabel/muninn.io), same
maintainer. It was built on huginn's conventions, and the two are kept aligned
deliberately — same README shape, same doc map, same pipeline, same rules here.
A change to any of those in one project should have an obvious counterpart in
the other.

---

## 2. Working with the maintainer

- **Language:** reply to the maintainer (**Joshua**) in **German**. Keep
  everything committed — code, comments, commit messages, docs, this file — in
  **English**.
- **Autonomy (solo project, Joshua is sole maintainer):** act pragmatically
  within the guardrails in §3. Execute the task, report concisely, ask only at
  genuine forks. Land work as a **pull request**, never directly on a protected
  branch.
- **Verify before changing.** Confirm versions, API shapes and facts against the
  actual source or official docs before editing. Guessing has caused real
  breakage here — §6 is the list. When unsure, check.
- **Security is a first-class priority.** huginn handles credentials and ships a
  container, so weigh every change through a security lens (secrets never in ENV
  or logs, least privilege, no new attack surface) and call out anything with a
  security dimension. §9 is not optional polish.
- **Don't duplicate.** Reuse existing helpers; when documenting, link the
  canonical page rather than copying it.

---

## 3. Hard rules — never without explicit approval

Stops, not preferences. Ask first, every time:

1. **Never push to `main` or `dev`.** Both are protected; all changes go through
   a PR from a `feature|fix|chore|docs|test/<name>` branch.
2. **Never merge or approve PRs.** Opening them is fine.
3. **Never change repository settings, secrets or rulesets.** No Actions secrets,
   branch protection or repo configuration.
4. **Never add a dependency, and never rewrite history or force-push**, without
   asking. New crates change the supply-chain surface; history rewrites are
   destructive.
5. **Never hand-push a `v*` tag.** The pipeline creates them after every gate
   passed; a hand-pushed tag would cut a release around them.

Everything else — editing code and docs, opening PRs, running the gates — is fair
game.

---

## 4. Architecture & where things live

Cargo workspace; one bounded responsibility per crate:

| Crate | Role |
|---|---|
| `huginn/` | Binary: CLI, config load, logging init, the **scheduler**, shutdown, orchestration (`main.rs`, `scheduler.rs`) |
| `crates/huginn-core/` | Shared types (`ProbeResult`), config model, `HuginError`, the `EventHub` |
| `crates/huginn-probes/` | The `Probe` trait, `ProbeRegistry`, one executor per protocol |
| `crates/huginn-influx/` | Line-protocol writer: batching, bounded retry queue, HTTP POST |
| `crates/huginn-web/` | Axum debug server (`/`, `/health`, `/metrics/latest`, `/events`) plus the separately gated Prometheus listener |

Dependencies point one way: `huginn` → everything; the other three →
`huginn-core`. No cycles, and `huginn-core` knows nothing about HTTP, InfluxDB
or any probe protocol.

**Data flow — the `EventHub` is the sole pub/sub bus** (a tokio `broadcast`
channel):

```
main.rs → load AppConfig (YAML + ENV) → create EventHub → spawn tasks:
   Scheduler ──publishes──► EventHub ──subscribes──► console output
                                              └─────► InfluxDB batcher → queue → writer
                                              └─────► WebState → debug UI + Prometheus
```

The **scheduler is the only publisher**; everything else subscribes. Shutdown is
a separate `broadcast::channel<()>` fired by SIGINT and, on Unix, SIGTERM.
Full picture: [`docs/architecture.md`](docs/architecture.md).

Other locations: `config/` (example + integration YAML), `scripts/` (the
integration suite, the release version stamp, the release test report),
`docs/adr/`, `deny.toml`, `.cargo/config.toml`, `.trivyignore.yaml`.

---

## 5. Commands (the gates)

```bash
cargo fmt --all -- --check                                   # format
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo deny check                                             # advisories, licences, bans, sources
cargo llvm-cov --all --lcov --output-path lcov.info --fail-under-lines 80
cargo build --release --locked
```

Aliases for all of these are in `.cargo/config.toml` (`cargo fmt-check`,
`cargo lint`, `cargo t-all`, `cargo audit-all`, `cargo cov-ci`).

Run locally: `cargo dev` (example config) or `cargo dev-json`. The web UI has
**no CLI flag** — `HUGINN_UI_ENABLED=true cargo dev`, and see §6.

System integration test:
`docker compose -f docker-compose.integration.yml up -d --build`, then
`bash scripts/integration-test.sh`.

CI runs all of the above on every PR, plus the image build, Trivy, Semgrep,
shellcheck, actionlint and the integration suite — see
[`docs/ci-cd.md`](docs/ci-cd.md). Run them locally anyway: the image jobs take
tens of minutes, and a red pipeline is a slower way to learn that `cargo fmt`
was not run.

---

## 6. Facts that shape the code

Each cost real investigation, and each contradicts a plausible assumption. Do not
undo them.

**The Docker build is the real MSRV gate.** `rust-version` in `Cargo.toml`
documents the floor, but under edition 2021 and resolver 2 it does *not* steer
resolution — CI runs floating stable and stays green while the image build
fails. Not hypothetical: `hickory-resolver` 0.26, required for
RUSTSEC-2026-0119, needs 1.88 and broke the image against the then-current 1.85
builder while every local check passed. The builder must stay at or above the
floor; Dependabot moves it forward on its own, and that is not an MSRV change.

**Catching only SIGINT silently disabled the shutdown drain.** `docker stop` and
systemd send **SIGTERM**. Without a handler for it the process died before the
InfluxDB writer could drain, so every buffered-but-unwritten result was lost on
each restart and deploy — and nothing said so.

**A broadcast receiver must be created before its task is spawned.** A tokio
`broadcast::Receiver` only sees messages sent after it was created. Subscribing
inside the spawned task loses the first probe tick whenever the executor does not
poll that task promptly — an intermittent, environment-dependent gap. Both the
console and the batcher take their receiver on the main task and move it in.
[ADR-0001](docs/adr/0001-eventhub-single-publisher.md)

**An empty secret file must be fatal, not empty.** An empty InfluxDB token file
used to be accepted as an empty token: the process started, InfluxDB answered
401, the writer classified that 4xx as *permanent* and discarded every batch. A
monitor that looks healthy while losing all of its data is the worst outcome
available. Every secret file is now fail-closed — missing, unreadable or empty
stops startup. [ADR-0002](docs/adr/0002-secrets-from-files-only.md)

**GitHub does not start a workflow from an event `GITHUB_TOKEN` created.** The
recursion guard. `ci.yml`'s `publish` pushes the release tag, and with the
built-in token `release.yml` (`on: push: tags`) never fires — the image ships and
the Release does not. The tag push uses `RELEASE_PAT` where available, and
`release.yml` has a `workflow_dispatch` entry point for when it is not.
muninn.io hit this at its v0.1.0.

**One version, in one place.** Internal crates are path dependencies with **no**
`version =` requirement, and the workspace is `publish = false`. A version
requirement on a path dependency is a second copy of the workspace version:
stamping a release made the crates x.y.z while the requirements still read
^0.1.0, and resolution failed before a test could run. `scripts/set-workspace-version.sh`
writes `Cargo.toml` and `Cargo.lock` together, because `--locked` is everywhere.

**`gawk` mis-compiles a dynamic regex containing `\[` and `\]`.** The version's
dots open a character class. Every changelog-section extraction in the workflows
therefore uses `substr()` comparison rather than a regex. Do not "simplify" it
back.

---

## 7. Conventions

### Coding style

- **Match the surrounding code** — naming, module layout, comment density,
  idioms. Consistency beats preference.
- `snake_case` items and modules, `CamelCase` types; descriptive names
  (`response_ms`, not `r`). Test names read as English sentences
  (`fails_on_timeout`), not `test_*` or `should_*`.
- **No `unwrap()` / `expect()` / `panic!` in non-test code.** Return a `Result`
  and propagate with `?`. Panics are for tests and genuinely unreachable
  invariants, with a comment saying why.
- Small, single-purpose functions; tight crate surfaces (`pub` only what other
  crates need).
- Doc-comment (`///`) public items. Comments explain the **why**, not the what.
- **Never repeat a version number in prose.** A version belongs where it is the
  authority — `rust-version` in `Cargo.toml`, the pins in the `Dockerfile`, a
  `fixed_version` in `.trivyignore.yaml` — and everywhere else you name the
  field and let the reader look. A number copied into a sentence is wrong the
  morning after Dependabot bumps it, and nothing fails when it goes stale.
  Two exceptions, because they are records rather than claims about now:
  a **historical incident** ("broke against the then-current 1.85 builder") and
  a **dated measurement** ("17 CVEs when counted on 2026-08-02"). Both keep
  their numbers, and both say when.
- `cargo fmt` defaults (no `rustfmt.toml`); **fix** clippy rather than
  `#[allow]`-ing it, and if an allow is genuinely needed, give a one-line reason.
- Avoid `unsafe`. Adding a dependency needs approval (§3).

### Project idioms

- **Errors:** `HuginError` (`thiserror`) in `huginn-core::error`, with
  `type Result<T>`; `anyhow` only at the binary boundary. **Probe failures never
  panic** — they become `ProbeResult::failure()` and are published like any other
  result.
- **Config & secrets:** YAML + `HUGINN_*` ENV; precedence CLI > ENV > YAML >
  default, *in both directions* — a CLI flag must be able to override the file
  back to the default, which an `||` cannot express. Secrets are file paths only,
  fail-closed. [ADR-0002](docs/adr/0002-secrets-from-files-only.md)
- **Async:** single tokio runtime (`#[tokio::main]`); each long-running component
  is a `tokio::spawn`ed task; `tokio::select!` multiplexes shutdown with timers;
  `#[tokio::test]` for async tests.
- **Logging:** `tracing` + `tracing-subscriber` (`EnvFilter`, `RUST_LOG`); pretty
  by default, JSON via `--output json`. Structured fields
  (`info!(probe = %name, response_ms, "probe UP")`), not interpolated prose.
- **InfluxDB writes:** raw line protocol, no SDK. `run_batcher` groups and never
  awaits I/O; `run_writer` drains a bounded `RetryQueue`. Retryable
  (transport/5xx/429/408) → exponential backoff; other 4xx → drop the batch.
  [ADR-0004](docs/adr/0004-bounded-retry-queue.md)
- **TLS is rustls only.** `openssl`, `openssl-sys`, `native-tls` and
  `tokio-native-tls` are banned in `deny.toml`.
  [ADR-0003](docs/adr/0003-rustls-only.md)
- **MSRV** is `rust-version` in `Cargo.toml`; edition 2021, resolver 2 — and see
  §6 on which gate actually enforces it.
- **Testing:** unit tests inline in `#[cfg(test)]`; cross-crate and whole-binary
  behaviour in `huginn/tests/`. Never hit a real external service — use
  `wiremock`. **Don't sleep — poll.** Tests touching the environment must be
  serialised. ≥ 80 % workspace line coverage.
  [`docs/testing.md`](docs/testing.md)

---

## 8. Git & PR workflow

- Branch off `dev` with a valid prefix: `feature/` · `fix/` · `chore/` · `docs/`
  · `test/`. Pushing such a branch makes `auto-pr.yml` open a draft PR into
  `dev`; a branch that does not match the prefix is auto-deleted.
- Flow: `feature/* → dev` (**squash**) → `main` (**merge commit**). No direct
  pushes (§3).
- **Conventional Commits:** `feat · fix · chore · docs · test · refactor · perf ·
  style`. End commit bodies with the `Co-Authored-By` trailer.
- Commit messages explain **why**, and state what was verified. "Verified: X
  passes" beats "should work".
- Releasing is a runbook, not a habit — [`docs/releasing.md`](docs/releasing.md).

---

## 9. Security posture — high priority

Any change touching secrets, the container, network exposure, dependencies or
workflow permissions must be reasoned about explicitly and flagged in the PR.

- **Secrets are file paths only.** Never a YAML value, never an environment
  variable, never logged. Missing, unreadable or empty stops startup.
- **Distroless + nonroot**, read-only root filesystem, all capabilities dropped,
  `no-new-privileges`. Never `--privileged` or `--cap-add`.
- **Both listeners are off by default and bind loopback**, and the debug UI is
  unauthenticated — `metrics.api_key_file` protects only the Prometheus
  listener, which is a decided trade rather than an open gap —
  [ADR-0009](docs/adr/0009-debug-ui-stays-unauthenticated.md).
- **Untrusted data from monitored hosts is escaped** before it leaves the
  process: a remote SMTP/IMAP banner reached the console, InfluxDB and every HTTP
  consumer with control bytes intact. `ProbeResult::failure` escapes the whole
  Unicode `Cc` range.
- **Least-privilege workflow `permissions:`** — grant the minimum a job needs,
  and scope `security-events: write` to the one job that uploads SARIF.
- **Gates:** cargo-deny (advisories, licences, the rustls-only ban), Semgrep
  (`p/rust` + `p/secrets`, ERROR blocks), Trivy (fixable CRITICAL/HIGH block),
  shellcheck, actionlint. A Trivy suppression needs an expiry and a reachability
  argument — the rules are in `.trivyignore.yaml`.

---

## 10. Doc map

| Topic | Read |
|---|---|
| What is still open | [`docs/roadmap.md`](docs/roadmap.md) |
| Architecture, event bus, startup, shutdown | [`docs/architecture.md`](docs/architecture.md) |
| Config reference (YAML + ENV) | [`docs/configuration.md`](docs/configuration.md) |
| InfluxDB setup and data model | [`docs/influxdb.md`](docs/influxdb.md) |
| Symptom → cause → fix | [`docs/troubleshooting.md`](docs/troubleshooting.md) |
| Testing pyramid, coverage, no-sleep rule | [`docs/testing.md`](docs/testing.md) |
| Container hardening | [`docs/hardening.md`](docs/hardening.md) |
| The 2026-08-02 security audit | [`docs/security-audit.md`](docs/security-audit.md) |
| Vulnerability reporting | [`docs/SECURITY.md`](docs/SECURITY.md) |
| Contributor workflow | [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) |
| SemVer policy, stable surface | [`docs/versioning.md`](docs/versioning.md) |
| CI/CD and repository setup | [`docs/ci-cd.md`](docs/ci-cd.md) |
| Every workflow explained | [`docs/workflows.md`](docs/workflows.md) |
| Release runbook | [`docs/releasing.md`](docs/releasing.md) |
| Open risks | [`docs/risks.md`](docs/risks.md) |
| Architecture decisions | [`docs/adr/`](docs/adr/) |
| Supply-chain policy | [`deny.toml`](deny.toml) |
