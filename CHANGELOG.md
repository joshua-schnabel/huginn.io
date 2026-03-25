# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2025-03-25

### Added
- Cargo workspace with 3 library crates (`hugin-core`, `hugin-probes`, `hugin-influx`) and binary `hugin-dec`
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

[Unreleased]: https://github.com/OWNER/hugin-dec/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OWNER/hugin-dec/releases/tag/v0.1.0
