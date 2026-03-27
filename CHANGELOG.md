# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **DNS probe** (`type: dns`) — resolves hostnames via configurable nameserver using `hickory-resolver`; optional `dns_expected_ip` validation
- **InfluxDB batch writes** — configurable `batch_size` and `batch_timeout_ms`; reduces HTTP traffic from 1 request per probe to batched line-protocol writes
- **Configurable EventHub capacity** — `event_hub_capacity` in app config (default 256)
- **System integration test** — `docker-compose.integration.yml` spins up InfluxDB + hugin-dev and runs curl-based assertions against the live stack
- **E2E tests** — multi-probe parallel execution, graceful shutdown, DNS probe E2E scenarios
- **`hugin-web` crate** — axum web server extracted into its own crate with SSE push updates, separate HTML/CSS/JS assets
- **EventHub architecture** — central `broadcast::Sender` in `hugin-core`; probes publish events, InfluxDB writer and web server subscribe independently
- **CI/CD redesign** — split into `ci.yml` (quality gate), `security.yml` (Trivy CVE, PR→main), `sast.yml` (Semgrep SAST, all PRs), `docker.yml` (DockerHub publish)
- **SAST tooling** — Semgrep (`p/rust` + `p/secrets`) two-pass: SARIF upload + blocking on ERROR severity
- **Supply-chain security** — `deny.toml` for `cargo-deny`; replaces `cargo-audit` with advisories + license allow-list + registry restriction
- **Branch setup** — `main` (stable) and `dev` (integration) branches; direct push blocked via branch protection
- **DockerHub tags** — `:dev` + `:X.Y.Z-dev` on dev push; `:latest` + `:X.Y.Z` on main push
- **`docs/ci-cd.md`** — pipeline documentation and branch protection guide
- **`docs/testing.md`** — four-level test pyramid, TDD workflow, coverage requirements

### Changed
- Project renamed from `hugin.dec` to `hugin.dev`
- `cargo-audit` replaced by `cargo-deny` in all CI pipelines
- Docker image registry: GHCR → DockerHub

## [0.1.0] - 2025-03-25

### Added
- Cargo workspace with 3 library crates (`hugin-core`, `hugin-probes`, `hugin-influx`) and binary `hugin-dev`
- **6 probe types**: TCP, HTTP, HTTPS, SMTP (banner check), IMAP (greeting check), UDP (DNS payload)
- **InfluxDB 2.x writer** using native line protocol via `reqwest` + `rustls`; token read from file, never from ENV
- **YAML configuration** (`config/config.example.yaml`) with full ENV override support (`HUGIN_*`, `INFLUX_TOKEN_FILE`)
- **Pretty colored CLI output** (default) and JSON mode via `--output json` or `HUGIN_LOG_FORMAT=json`
- **Axum debug web UI** (optional, `--ui` / `HUGIN_UI_ENABLED=true`): `/`, `/health`, `/metrics/latest`
- **Graceful shutdown** via CTRL+C with broadcast channel across all probe loops
- **Docker multi-stage build**: `rust:slim` builder → `distroless/cc-debian12` runtime, runs as `nonroot`
- **Docker Compose** with Docker Secrets for InfluxDB token (never in ENV)
- **GitHub Actions CI** (`ci.yml`): fmt, clippy, test matrix (stable+beta), cargo-audit
- **GitHub Actions Docker** (`docker.yml`): Trivy scan, push `:dev` on dev-merge, push `:latest`+`:vX.Y.Z` on main-merge (version from CHANGELOG)
- **55 tests** (unit + integration, TDD-style)
- Documentation: `README.md`, `docs/getting-started.md`, `docs/configuration.md`, `docs/influxdb.md`, `docs/security.md`, `docs/troubleshooting.md`
- `CONTRIBUTING.md` with branching workflow, PR process, Conventional Commits, release process

[Unreleased]: https://github.com/OWNER/hugin-dev/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OWNER/hugin-dev/releases/tag/v0.1.0
