# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **The daemon exited immediately at startup, having run no probes.** `run()` spawned the probe loops and returned; `main()` then exited and the Tokio runtime cancelled every task before the first tick. The fix (keep-alive on the shutdown channel) existed on `dev` but was lost when `feature/refactoring` branched from a parallel CI fix.
- **Tests could not observe that bug.** They all spawned `run()` into the test's own runtime, which outlives it — production has no such runtime. Added `huginn/tests/binary_lifecycle_test.rs`, which runs the real binary as a subprocess, and a negative shutdown test asserting `run()` does *not* return without a signal.
- **DockerHub publish ran in parallel with CI and depended on nothing** — a commit with failing tests, clippy or cargo-deny still shipped `:latest`. Publish is now a job in `ci.yml` gated by `needs` on every check, with `contents: write` so the release tag push no longer 403s.
- **`cargo deny check` was not running at all.** `deny.toml` used `severity-threshold`/`unlicensed`/`copyleft`, removed in cargo-deny ≥ 0.14, so the config failed to parse. Once repaired it surfaced four real advisories: `rustls-webpki` → 0.103.13 (RUSTSEC-2026-0098/-0099/-0104) and `hickory-resolver` 0.24 → 0.26 (RUSTSEC-2026-0119).
- **CI ran only on PRs targeting `main`/`dev`**, so feature branches had no gate — the condition that let the two branches diverge. It now runs on every PR.
- **Trivy skipped the release PR.** It was gated on `base_ref == 'dev'`, so `dev → main` was never scanned and CVEs surfaced only after landing on `main`. It now runs on every PR.
- **A newline in a probe error corrupted an entire InfluxDB batch** — line protocol is newline-delimited and `escape_field_str` did not escape `\n`. Also: `escape_tag` did not escape backslashes, and `urlencode` (formerly `urlenccode`) encoded code points rather than UTF-8 bytes, breaking non-ASCII org/bucket names.
- **The InfluxDB HTTP client had no timeout**, so a blackholing server would hang the batch subscriber — including its shutdown flush — indefinitely.
- **`--output pretty` could not override `format: json`** from the config file: the check was an OR, and `--output` always had a default, so "not given" and "explicitly pretty" were indistinguishable.
- **Invalid ENV values were swallowed** (`HUGINN_UI_PORT=abc`, `HUGINN_LOG_FORMAT=xml`, `HUGINN_UI_ENABLED=yes`). They now warn and keep the previous value; warnings are emitted after tracing initialises, since config is loaded before it exists.
- **Config errors that only surfaced at runtime** are now rejected at load: `event_hub_capacity: 0` (panicked in `broadcast::channel`), `batch_size: 0` (made every result its own POST), duplicate probe names (collided in the UI map and the InfluxDB series), and malformed per-type targets.
- Fixed a real flake: `run_with_ui_enabled_responds_to_health_check` slept a fixed 150 ms and made one unretried request.

### Removed
- `run_subscriber`, `run_subscriber_batched` and `InfluxWriter::write` — the old single-consumer writer paths. Replaced by the `run_batcher` + `run_writer` split (see below). Their meaningful behaviours (clean exit on hub close, surviving a lagged receiver) are now tested against the new tasks.

### Added
- **InfluxDB resilience** — the writer is split into a `run_batcher` (groups results, never awaits I/O) and a `run_writer` (drains a bounded `RetryQueue`). Failed writes are retried with exponential backoff instead of discarded; `WriteError` classifies transport/5xx/429/408 as retryable and 4xx as permanent (dropped). Retry is unbounded in attempts, bounded in memory (`max_buffered_bytes`, drop-oldest). New `influx` config keys: `max_buffered_bytes`, `retry_initial_backoff_ms`, `retry_max_backoff_ms`, `shutdown_drain_timeout_ms`.
- **`Probe` trait + `ProbeRegistry`** — per-protocol probes implement a common trait; the registry owns shared state (the HTTP client) so probe loops no longer thread resources they don't use.
- **`ProbeResult.metrics`** (`BTreeMap<String, f64>`) — a home for per-probe-type numeric readings (e.g. TLS expiry days, packet loss), emitted as additional line-protocol fields. No probe populates it yet.
- **Config validation** — rejects duplicate probe names, `event_hub_capacity: 0`, `batch_size: 0`, and per-type malformed targets (dns needs `ip:port`, tcp/smtp/imap/udp need a port, http/https need an absolute URL) at load time.
- **DNS probe** (`type: dns`) — resolves hostnames via configurable nameserver using `hickory-resolver`; optional `dns_expected_ip` validation
- **InfluxDB batch writes** — configurable `batch_size` and `batch_timeout_ms`; reduces HTTP traffic from 1 request per probe to batched line-protocol writes
- **Configurable EventHub capacity** — `event_hub_capacity` in app config (default 256)
- **System integration test** — `docker-compose.integration.yml` spins up InfluxDB + huginn and runs curl-based assertions against the live stack
- **E2E tests** — multi-probe parallel execution, graceful shutdown, DNS probe E2E scenarios
- **`huginn-web` crate** — axum web server extracted into its own crate with SSE push updates, separate HTML/CSS/JS assets
- **EventHub architecture** — central `broadcast::Sender` in `huginn-core`; probes publish events, InfluxDB writer and web server subscribe independently
- **CI/CD redesign** — `ci.yml` (quality gate + gated DockerHub publish) and `security.yml` (Semgrep SAST + Trivy CVE)
- **SAST tooling** — Semgrep (`p/rust` + `p/secrets`) two-pass: SARIF upload + blocking on ERROR severity
- **Supply-chain security** — `deny.toml` for `cargo-deny`; replaces `cargo-audit` with advisories + license allow-list + registry restriction
- **Branch setup** — `main` (stable) and `dev` (integration) branches; direct push blocked via branch protection
- **DockerHub tags** — `:dev` + `:X.Y.Z-dev` on dev push; `:latest` + `:X.Y.Z` on main push
- **`docs/ci-cd.md`** — pipeline documentation and branch protection guide
- **`docs/testing.md`** — four-level test pyramid, TDD workflow, coverage requirements

### Changed
- Project renamed from `hugin.dec` to `huginn.io`
- `cargo-audit` replaced by `cargo-deny` in all CI pipelines
- Docker image registry: GHCR → DockerHub
- `hickory-resolver` 0.24 → 0.26 (fixes RUSTSEC-2026-0119); raises the MSRV to Rust 1.88 (Dockerfile builder and `rust-version` bumped to match)
- Config precedence is now honoured in both directions: `--output`/`HUGINN_LOG_FORMAT` overrides `log.format` from the config file (previously an OR that could not override back to `pretty`)
- Invalid ENV overrides now warn and keep the previous value instead of being silently ignored

## [0.1.0] - 2025-03-25

### Added
- Cargo workspace with 3 library crates (`huginn-core`, `huginn-probes`, `huginn-influx`) and binary `huginn`
- **6 probe types**: TCP, HTTP, HTTPS, SMTP (banner check), IMAP (greeting check), UDP (DNS payload)
- **InfluxDB 2.x writer** using native line protocol via `reqwest` + `rustls`; token read from file, never from ENV
- **YAML configuration** (`config/config.example.yaml`) with full ENV override support (`HUGINN_*`, `INFLUX_TOKEN_FILE`)
- **Pretty colored CLI output** (default) and JSON mode via `--output json` or `HUGINN_LOG_FORMAT=json`
- **Axum debug web UI** (optional, `--ui` / `HUGINN_UI_ENABLED=true`): `/`, `/health`, `/metrics/latest`
- **Graceful shutdown** via CTRL+C with broadcast channel across all probe loops
- **Docker multi-stage build**: `rust:slim` builder → `distroless/cc-debian12` runtime, runs as `nonroot`
- **Docker Compose** with Docker Secrets for InfluxDB token (never in ENV)
- **GitHub Actions CI** (`ci.yml`): fmt, clippy, test matrix (stable+beta), cargo-audit
- **GitHub Actions Docker** (`docker.yml`): Trivy scan, push `:dev` on dev-merge, push `:latest`+`:vX.Y.Z` on main-merge (version from CHANGELOG)
- **55 tests** (unit + integration, TDD-style)
- Documentation: `README.md`, `docs/getting-started.md`, `docs/configuration.md`, `docs/influxdb.md`, `docs/security.md`, `docs/troubleshooting.md`
- `CONTRIBUTING.md` with branching workflow, PR process, Conventional Commits, release process

[Unreleased]: https://github.com/OWNER/huginn/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OWNER/huginn/releases/tag/v0.1.0
