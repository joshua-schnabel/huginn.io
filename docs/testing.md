# Testing Guide

This document describes the testing philosophy, structure, and requirements for hugin.dev contributors.

---

## The Test Pyramid

hugin.dev follows the classic test pyramid: many fast unit tests at the base, fewer integration tests in the middle, and a small number of end-to-end tests at the top.

```
             ▲
            /E\
           / 2 \       End-to-End Tests
          /─────\      • Full running stack
         /       \     • Slow, but high confidence
        / Integr. \
       /     22    \   Integration Tests
      /─────────────\  • Multiple components together
     /               \ • Real sockets / mock HTTP servers
    /   Unit  ~55     \
   /───────────────────\  Unit Tests
  /                     \ • Single function / module
 /─────────────────────── \ • Fast, isolated, deterministic
```

| Level | Count | Location | Speed |
|---|---:|---|---|
| Unit | ~55 | `#[cfg(test)]` inside source modules | < 50 ms total |
| Integration | ~22 | `hugin-dev/tests/*.rs` | < 5 s total |
| E2E | ~1 | `hugin-dev/tests/sse_test.rs` | < 3 s |

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

### What NOT to test
- Trivial getters / setters
- Rust standard-library behaviour (e.g. `Vec::push`)
- Log statements in isolation

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
Use plain English that reads as a sentence:

```
succeeds_on_220_banner
fails_when_port_closed
fails_on_empty_banner
event_loop_inserts_result_on_probe_completed
subscriber_handles_lagged_events
```

Avoid `test_`, `should_`, or numbered names like `test1`.

### Helper pattern for network tests
Bind a real socket on port `0` (OS assigns a free port), spawn a minimal server, pass the address to the probe under test:

```rust
async fn fake_smtp_server(banner: &'static str) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let _ = socket.write_all(banner.as_bytes()).await;
        }
    });
    addr
}
```

---

## Integration Tests

### What to test
- Interactions between two or more crates (e.g. `scheduler` + `EventHub` + `WebState`)
- HTTP route handlers via a real bound port (axum test server)
- Config loading from actual YAML files on disk

### What NOT to test
- The behaviour of a single module in isolation (that is a unit test)
- External services — use mock servers ([`wiremock`](https://github.com/LukeMathWalker/wiremock-rs))

### Where they live
Integration tests live in **`hugin-dev/tests/`** as separate `.rs` files:

```
hugin-dev/tests/
├── cli_output_test.rs        – ProbeResult serialisation
├── config_integration_test.rs – config loading + ENV overrides
├── debug_ui_test.rs          – full HTTP server + reqwest client
└── sse_test.rs               – SSE stream delivery
```

Each file is compiled as its own test binary by Cargo.

### Mock HTTP servers
Use `wiremock` to simulate InfluxDB or HTTP probe targets:

```rust
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

let server = MockServer::start().await;
Mock::given(method("POST"))
    .respond_with(ResponseTemplate::new(204))
    .expect(1)           // assert exactly 1 request was made
    .mount(&server)
    .await;

// … exercise code under test …

server.verify().await;  // fails the test if expectation was not met
```

---

## End-to-End Tests

### What to test
The complete user-visible path: binary entry point (or equivalent wiring) → real network I/O → observable output.

Currently: `sse_test.rs::sse_endpoint_delivers_probe_event_as_data_message`
- Starts a real `run_server()` on a random port
- Opens a streaming HTTP connection to `/events`
- Publishes a `ProbeEvent` on the `EventHub`
- Asserts that a `data:` line appears in the SSE stream

### What NOT to test in E2E
- Internal implementation details
- Error branches already covered by unit/integration tests

### Keep E2E tests few and stable
E2E tests are the most expensive to maintain. Only add one when a new user-visible feature cannot be adequately covered by the layers below.

---

## TDD Workflow

hugin.dev uses **Test-Driven Development**. New features and bug fixes follow the Red → Green → Refactor cycle:

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
cargo test -p hugin-probes

# Run a specific test by name (substring match)
cargo test -p hugin-probes fails_on_empty_banner

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
| New user-visible push feature | E2E | `hugin-dev/tests/` |
| Bug fix | Unit (reproduce the bug first) | same file as the fix |
