# Copilot Instructions

## Project Overview

**huginn** is a lightweight uptime and latency monitor written in Rust. It executes configurable probes (TCP, HTTP/HTTPS, SMTP, IMAP, UDP, DNS) at regular intervals, measures response time and up/down status, and writes results to InfluxDB via batched line-protocol HTTP writes. An optional debug web UI serves live results via SSE.

## Build, Test, and Lint Commands

```bash
# Build
cargo build --release --locked   # production
cargo build                      # debug

# Test
cargo test --workspace           # all tests
cargo test -p huginn-probes      # single crate
cargo test -p huginn-probes fails_on_empty_banner  # single test (substring match)

# Lint & format
cargo fmt --all                  # format
cargo fmt --all -- --check       # check only (CI)
cargo clippy --all-targets --all-features -- -D warnings  # clippy (all warnings = errors)
cargo deny check                 # supply-chain: CVEs, licenses, banned crates

# Coverage (requires cargo-llvm-cov)
cargo llvm-cov --workspace --open
cargo llvm-cov --all --lcov --output-path lcov.info --fail-under-lines 80

# Run locally
cargo run -- --config config/config.yaml
cargo run -- --config config/config.yaml --output json
cargo run -- --config config/config.yaml --ui
```

### System Integration Tests (Docker required)
```bash
echo -n "integration-test-token-huginn-ci" > /tmp/influx_token.txt
docker compose -f docker-compose.integration.yml up -d --build
bash scripts/integration-test.sh
docker compose -f docker-compose.integration.yml down -v
```

Copy `config/config.example.yaml` → `config/config.yaml` before running locally.

## Workspace Architecture

This is a Cargo workspace. Each crate has a single, well-bounded responsibility:

| Crate | Role |
|---|---|
| `huginn/` | Binary entry point — CLI, config loading, logging init, graceful shutdown, orchestration |
| `crates/huginn-core/` | Shared types (`ProbeResult`), config structs, `HuginError`, `EventHub` |
| `crates/huginn-probes/` | Probe scheduler + per-protocol executors (tcp, http, smtp, imap, udp, dns) |
| `crates/huginn-influx/` | InfluxDB writer — line-protocol serialization, batching, HTTP POST |
| `crates/huginn-web/` | Optional Axum debug server — `/health`, `/metrics/latest`, `/events` (SSE) |

### Data Flow

```
main.rs
  └─ loads AppConfig (YAML + ENV overrides)
  └─ creates EventHub (tokio broadcast channel)
  └─ spawns tasks:
       ├─ Scheduler (huginn-probes) ──publishes──► EventHub
       ├─ Console output             ◄─subscribes─┘
       ├─ InfluxDB writer            ◄─subscribes─┘
       └─ Web UI (if enabled)        ◄─subscribes─┘
```

**EventHub is the sole pub/sub bus.** The scheduler is the only publisher; all other components are subscribers. Shutdown is signalled via a separate `broadcast::channel<()>` driven by `tokio::signal::ctrl_c()`.

## Key Conventions

### Error Handling
- Custom `HuginError` enum using **thiserror** lives in `huginn-core::error`
- Type alias: `type Result<T> = std::result::Result<T, HuginError>`
- Use `?` for propagation; return early on config/startup errors
- Probe failures are logged with `error!()` and returned as `ProbeResult::failure()`; they never panic

### Async
- Single **tokio** runtime (`#[tokio::main]`)
- Each long-running component is a `tokio::spawn()`-ed task
- `tokio::select!` is used in the scheduler and InfluxDB writer to multiplex shutdown signal with timer/channel
- Async tests use `#[tokio::test]`

### Logging
- **tracing** + **tracing-subscriber** with `EnvFilter`
- Pretty format by default; JSON via `--output json` flag
- `RUST_LOG` controls log level (default: `info`)
- Use structured fields: `info!(probe = %name, response_ms, "probe UP")`

### Configuration
- YAML file + ENV variable overrides; CLI flag > ENV > YAML > default
- `AppConfig::load(path)` deserializes YAML then applies ENV overrides
- **Secrets (InfluxDB token) must be in a file** — never in ENV. Path is set via `influx.token_file` in config. The file is read once at startup; missing file = immediate fatal error.
- Docker secrets are mounted at `/run/secrets/influx_token` (ephemeral tmpfs)

### InfluxDB Writes
- Raw HTTP line protocol — no SDK
- Format: `measurement,name=<n>,type=<t> response_ms=<v>,up=<bool>,status_code=<n> <timestamp_ms>`
- Writes are batched: flush when `batch_size` results accumulate **or** `batch_timeout_ms` elapses
- Non-200 responses are logged as warnings; the writer keeps running

### Testing
- **80% line coverage per file is enforced by CI** (`cargo llvm-cov --fail-under-lines 80`)
- Unit tests live in inline `#[cfg(test)]` modules in the same file as the code under test
- Test names describe behavior in plain English: `succeeds_on_220_banner`, `fails_on_timeout`
- Use `#[tokio::test]` for async tests
- Integration tests live in `huginn/tests/*.rs` and use **wiremock** to mock InfluxDB and HTTP targets — never hit real external services
- Helper functions (e.g., `success_result()`, `minimal_config()`) provide reusable test fixtures

### Shared State Pattern
- Shared immutable config: `Arc<AppConfig>`
- Shared mutable web state: `Arc<WebState>` (interior mutability via tokio `RwLock`)
- EventHub is cloned cheaply via `Arc`

## CI Pipeline (GitHub Actions)

| Workflow | Trigger | Steps |
|---|---|---|
| `ci.yml` | All PRs + pushes to dev/main | fmt check → clippy → tests (stable + beta matrix) → cargo deny → coverage (≥80%) → Docker integration test |
| `sast.yml` | All PRs + pushes | Semgrep (`p/rust` + `p/secrets`); ERROR-level findings block merge |
| `security.yml` | PR→main + push main | Trivy image scan; fixable CRITICAL/HIGH CVEs block merge |
| `docker.yml` | PRs + push dev/main | PRs: build only; push dev: publish `:dev`; push main: publish `:latest` + git tag |

Branch strategy: `feature/*` → PR → `dev` → PR → `main`

## Security Constraints

- **No OpenSSL** — TLS is handled exclusively by **rustls** (pure Rust)
- **Distroless runtime image** (`gcr.io/distroless/cc-debian12`), runs as `nonroot:nonroot`
- Token files must have mode `0600`; config mounted read-only in Docker
- `cargo deny` blocks known CVEs and unapproved licenses (MIT, Apache-2.0, BSD, ISC only)
- Never add `--privileged`, `--cap-add`, or secret values to ENV in Docker configs

## Commit Convention

Follow Conventional Commits: `feat:`, `fix:`, `chore:`, `docs:`, `test:`, `refactor:`, `perf:`, `style:`
