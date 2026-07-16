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
          / E2E      \
         /─────────────\ End-to-End Tests
        /               \• Real binary as a subprocess
       /   Integ.  ~24   \• Slow, but highest in-repo confidence
      /───────────────────\
     /                     \ Integration Tests
    /      Unit   ~110      \• Multiple components together
   /─────────────────────────\• Real sockets / mock HTTP servers
  /                           \Unit Tests
 /───────────────────────────── \• Single function / module
                                 • Fast, isolated, deterministic
```

| Level | Count | Location | Speed |
|---|---:|---|---|
| Unit | ~110 | `#[cfg(test)]` inside source modules | < 1 s total |
| Integration + E2E | ~24 | `huginn/tests/*.rs`, `crates/*/tests/*.rs` | < 20 s total |
| System Integration | 1 | `scripts/integration-test.sh` + Docker Compose | ~2 min (CI only) |

Counts drift. `cargo test --workspace` is the authority.

### Test the artefact, not just the code

`huginn/tests/binary_lifecycle_test.rs` runs the compiled binary as a subprocess
via `CARGO_BIN_EXE_huginn`. This exists because of a specific failure: `run()`
returned immediately without a keep-alive, so `main()` exited and the Tokio
runtime cancelled every probe before it fired — the daemon monitored nothing.

**No in-process test could catch it.** They all spawn `run()` into the *test's*
runtime, which outlives it; production has no such runtime. The tests passed on
a completely broken binary for months.

When you test something whose behaviour depends on the process lifecycle, test
the process.

---

## Coverage Requirement

**CI enforces ≥ 80 % *line* coverage across the workspace, in aggregate**:

```bash
cargo llvm-cov --all --lcov --output-path lcov.info --fail-under-lines 80
```

Know what that does and does not buy you:

- It is an **aggregate**, not a per-file floor. A well-covered crate can mask an
  entirely untested module and the gate stays green.
- It counts **lines**, not regions. Branches inside a covered line are not
  distinguished.
- `cargo-llvm-cov`'s `--fail-under-*` flags are global; a genuine per-file gate
  would need a separate tool. Until then, treat 80 % as a floor against
  collapse, not as evidence any given file is tested.

To see where the gaps actually are:

```bash
cargo install cargo-llvm-cov          # once
cargo llvm-cov --workspace --open     # per-file, per-region HTML report
```

**Coverage is not the goal.** Dead code that is thoroughly tested still counts
toward the number — `run_subscriber` was ~100 % covered and unreachable in
production, and its three tests were inflating this gate while asserting
nothing about the shipped binary. A covered line is not a checked behaviour.

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
├── binary_lifecycle_test.rs    – the real binary as a subprocess: stays up, serves, probes
├── cli_output_test.rs          – ProbeResult serialisation
├── common.rs                   – shared helpers (free_port, start_server)
├── config_integration_test.rs  – config loading + ENV overrides
├── debug_ui_test.rs            – full HTTP server + reqwest client
├── multi_probe_e2e_test.rs     – EventHub → WebState → HTTP
└── sse_test.rs                 – SSE stream delivery
```

A library crate may have its own `tests/` directory when what it verifies is not
about the binary:

```
crates/huginn-core/tests/
└── shipped_configs_test.rs     – config/*.yaml must pass our own validate()
```

Note that the rule "anything needing tokio/sockets goes in `tests/`" is not what
the code actually does: `writer.rs` and `http.rs` run wiremock and real sockets
in `#[cfg(test)]`. The real rule is: **library crates test their own code inline;
`huginn/tests/` is for cross-crate and whole-binary behaviour.**

Use `wiremock` to mock InfluxDB or HTTP probe targets — never hit real external services in tests.

### Tests that touch the environment must be serialised

The environment is process-global and cargo runs tests on parallel threads, so
one test's `remove_var` can race another's `set_var`. `config.rs` gates all such
tests behind an `ENV_LOCK` mutex via the `with_env` helper. Use it.

### Don't sleep — poll

A fixed `sleep` before an assertion is a flake waiting for a loaded CI runner.
`run_with_ui_enabled_responds_to_health_check` slept 150 ms and made one
unretried request; it failed under concurrent compile load. Poll with a deadline
instead (`wait_until` in `binary_lifecycle_test.rs`, or `tokio::time::timeout`
around a retry loop).

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
