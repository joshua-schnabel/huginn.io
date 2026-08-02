# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **Dependabot's weekly cargo run had been failing since 2026-07-19** and opened no grouped dependency PRs at all. `serde` 1.0.229 (published the day before) requires `serde_core =1.0.229`, an exact pin, and neither `serde_core` nor `serde_derive` is named in a `Cargo.toml` — so with Dependabot's default `direct`-only scope it was allowed to move `serde` alone, which cargo cannot do. The lockfile came back unchanged, the updater raised `Failed to update serde!`, and every other crate in the group went down with it. Resolved by bumping `serde` in the lockfile by hand; widening the scope to `dependency-type: all` was tried first and reverted, because with transitive crates in scope the updater wrote a lockfile with a dangling `syn` reference that `cargo --locked` rejects. Security updates were unaffected throughout.
- **The UDP probe could never reach an IPv6 target.** The local socket was always bound to the IPv4 wildcard `0.0.0.0:0`, which cannot connect to an IPv6 peer, so a target that `validate()` had accepted reported DOWN forever. The target is now resolved first (under the probe timeout, so a stalling resolver can no longer exceed `timeout_secs`) and the socket is bound in the matching address family. A bind failure now also reports the elapsed time instead of a hardcoded `0.0`.
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
- **Release automation** — `release.yml` fires on the `vX.Y.Z` tag that `publish` creates: it opens the GitHub Release (notes pulled from this file's matching section, `0.x`/`-rc` flagged as pre-release) and opens an auto-merging PR into `dev` that reopens a fresh `## [Unreleased]`, fixes the compare links, and bumps `Cargo.toml`. The bot never pushes to `main`/`dev`.
- **CI version gate** — a release PR (`dev → main`) is blocked unless the top `CHANGELOG.md` version is valid SemVer and strictly greater than the latest `v*` tag; also re-checked before anything ships.
- **GitHub Container Registry mirror** — every published image is mirrored from DockerHub to `ghcr.io` with `skopeo copy --all`, byte-identical (same digests) to the scanned/tested image; no second build.
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
- **The HTTP/HTTPS probe no longer follows redirects.** reqwest follows up to 10 hops by default, which meant `expected_status: 200` silently passed for a 301→200 chain and the measured `response_ms` included the extra round-trips. An uptime check has to judge the URL it was given, so a redirect is now reported with its own status — a 301 against `expected_status: 200` is DOWN. If you were relying on the old behaviour, point the probe at the redirect target instead.
- Project renamed from `hugin.dec` to `huginn.io`
- `cargo-audit` replaced by `cargo-deny` in all CI pipelines
- Docker image registry: GHCR → DockerHub
- `hickory-resolver` 0.24 → 0.26 (fixes RUSTSEC-2026-0119); raises the MSRV to Rust 1.88 (Dockerfile builder and `rust-version` bumped to match)
- Config precedence is now honoured in both directions: `--output`/`HUGINN_LOG_FORMAT` overrides `log.format` from the config file (previously an OR that could not override back to `pretty`)
- Invalid ENV overrides now warn and keep the previous value instead of being silently ignored
- **BREAKING — the debug UI now binds `127.0.0.1` instead of `0.0.0.0`.** The address is the new `ui.bind` key (`HUGINN_UI_BIND`), validated as an IP address at load. It has no authentication and publishes every probe target, so reaching a wider network is now an explicit act. **Containers must set `0.0.0.0`** — a published port reaches the container's bridge IP, never its loopback; `docker-compose.yml` and `config/config.integration.yaml` do this already. Only setups that enable the UI (`ui.enabled` defaults to `false`) are affected.

### Security
- **Closed a shell-injection path from `CHANGELOG.md` into the `publish` job.** The version was extracted with `sed` and then interpolated as `${{ }}` straight into `run:` blocks, in the one job holding `contents: write`, `packages: write` and the DockerHub credentials — a crafted `## [...]` heading merged to `dev` reached a shell. Extraction and SemVer validation now live in `scripts/changelog-version.sh`, shared with the version gate, and every consumer takes the value through `env:`. The gate alone did not cover this: it is a deliberate no-op outside a release context, while `publish` runs on every push.
- **Every GitHub Action is pinned to a full commit SHA**, and the Semgrep container to a digest. Tags and branches are movable, so a compromised upstream reached CI without a Dependabot PR — including the actions that consume the registry credentials. `dtolnay/rust-toolchain` moved from the `@stable`/`@master` branches to the `v1` SHA with an explicit `toolchain:` input; the toolchain channel itself still floats.
- **A `v*` tag can no longer publish from a commit that is not on `main`.** Tags are not covered by the branch ruleset and the version gate is a no-op on a tag push, so a hand-pushed tag would have published an image and cut a release. `ci.yml` `publish` and `release.yml` now verify the tagged commit is an ancestor of `main`.
- **Dependabot now waits 3 days before proposing a new version** (`cooldown`), so a freshly published malicious release is not auto-merged within the hour. Security updates are exempt by design and are never delayed.

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

[Unreleased]: https://github.com/joshua-schnabel/huginn.io/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/joshua-schnabel/huginn.io/releases/tag/v0.1.0
