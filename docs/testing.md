# Testing Guide

This document describes the testing philosophy, structure, and requirements for huginn.io contributors.

---

## The Test Pyramid

huginn.io follows a four-level test pyramid: many fast unit tests at the base, fewer component-integration tests, a small number of end-to-end tests, and one Docker-level system integration test at the top.

```
               ▲
              /S\
             / 4 \       System Integration Test
            /─────\      • Real Docker stack (InfluxDB + huginn)
           /       \     • CI only, ~2 min, highest confidence
          / E2E  ~9 \
         /───────────\   End-to-End Tests
        /             \  • Full running stack in-process
       /   Integ. ~27  \ • Slow, but high confidence
      /─────────────────\
     /                   \ Integration Tests
    /    Unit  ~73        \• Multiple components together
   /───────────────────────\• Real sockets / mock HTTP servers
  /                         \Unit Tests
 /─────────────────────────── \• Single function / module
                               • Fast, isolated, deterministic
```

| Level | Count | Location | Speed |
|---|---:|---|---|
| Unit | ~73 | `#[cfg(test)]` inside source modules | < 100 ms total |
| Integration | ~27 | `huginn/tests/*.rs` | < 10 s total |
| E2E | ~9 | `huginn/tests/*.rs` | < 15 s total |
| System Integration | 1 | `scripts/integration-test.sh` + Docker Compose | ~2 min (CI only) |

---

## Coverage Requirement

**Every source file must reach ≥ 80 % region coverage.**

Measure with [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```bash
# Install (once)
cargo install cargo-llvm-cov

# Per-file region coverage report
cargo llvm-cov --workspace --open
```

CI will fail the PR if any file drops below 80 %.

---

## Unit Tests

### What to test
- Every public function's happy path
- All `match` / `if` branches that contain real logic
- Error paths (connection refused, empty response, unexpected status, etc.)
- Boundary values (0 bytes, empty strings, capacity limits)

### Where they live
Unit tests live in a `#[cfg(test)]` block at the **bottom of the same file**:

```rust
// src/smtp.rs

pub async fn probe(cfg: &ProbeConfig) -> ProbeResult { … }

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn succeeds_on_220_banner() { … }

    #[tokio::test]
    async fn fails_on_empty_banner() { … }
}
```

### Naming convention
Plain English that reads as a sentence: `succeeds_on_220_banner`, `fails_when_port_closed`, `event_loop_inserts_result_on_probe_completed`.  
Avoid `test_`, `should_`, or numbered names.

---

## Integration Tests

### What to test
- Interactions between two or more crates (e.g. `scheduler` + `EventHub` + `WebState`)
- HTTP route handlers via a real bound port (axum test server)
- Config loading from actual YAML files on disk

### Where they live
Integration tests live in **`huginn/tests/`** as separate `.rs` files:

```
huginn/tests/
├── cli_output_test.rs          – ProbeResult serialisation
├── config_integration_test.rs  – config loading + ENV overrides
├── debug_ui_test.rs            – full HTTP server + reqwest client
└── sse_test.rs                 – SSE stream delivery
```

Use `wiremock` to mock InfluxDB or HTTP probe targets — never hit real external services in tests.

---

## End-to-End Tests

The complete user-visible path: binary wiring → real network I/O → observable output.

Example: `sse_test.rs` starts a real `run_server()`, opens a streaming HTTP connection to `/events`, publishes a `ProbeEvent`, and asserts a `data:` line appears in the SSE stream.

Add an E2E test only when a new user-visible feature cannot be adequately covered by the layers below.

---

## System Integration Tests

Tests the complete production stack in Docker:
- Image builds without errors
- `huginn` connects to InfluxDB and writes data
- `/health` returns `OK`, `/metrics/latest` returns probe results

```
docker-compose.integration.yml   – InfluxDB + huginn
config/config.integration.yaml   – 2-second probes
scripts/integration-test.sh      – curl assertions
```

**Run locally:**

```bash
echo -n "integration-test-token-huginn-ci" > /tmp/influx_token.txt
docker compose -f docker-compose.integration.yml up -d --build
bash scripts/integration-test.sh
docker compose -f docker-compose.integration.yml down -v
```

One system integration test covering the core data flow is enough.

---

## TDD Workflow

huginn.io uses **Test-Driven Development**. New features and bug fixes follow the Red → Green → Refactor cycle:

```
1. RED    – Write a failing test that describes the desired behaviour.
            Commit it (it must not compile or must fail).

2. GREEN  – Write the minimum production code to make the test pass.
            Do not over-engineer. Do not add untested code.

3. REFACTOR – Clean up duplication, naming, and structure.
              All tests must still pass after refactoring.
```

**Rule**: Production code that has no test is not merged. If you find untested code in a PR review, request a test before approving.

---

## Running Tests

```bash
# Run all tests in the workspace
cargo test --workspace

# Run tests for a single crate
cargo test -p huginn-probes

# Run a specific test by name (substring match)
cargo test -p huginn-probes fails_on_empty_banner

# Watch mode (requires cargo-watch)
cargo watch -x "test --workspace"

# Coverage report (requires cargo-llvm-cov)
cargo llvm-cov --workspace --open
```

---

## Quick Reference

| Situation | Test type | Location |
|---|---|---|
| New probe function | Unit | same file, `#[cfg(test)]` |
| New EventHub behaviour | Unit | `event.rs #[cfg(test)]` |
| New HTTP route | Unit + Integration | `server.rs` + `debug_ui_test.rs` |
| New config option | Integration | `config_integration_test.rs` |
| New user-visible push feature | E2E | `huginn/tests/` |
| Bug fix | Unit (reproduce the bug first) | same file as the fix |
| New external service dependency | System Integration | `scripts/integration-test.sh` |
