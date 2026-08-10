# ── Stage 1: Build ────────────────────────────────────────────────────────
# Keep in sync with rust-version in the workspace Cargo.toml.
# 1.88 is the floor set by hickory-resolver 0.26, which is required for
# RUSTSEC-2026-0119 — 0.24 pins a vulnerable hickory-proto.
FROM rust:1.97-slim AS builder

WORKDIR /build

# Cache dependencies before copying source
COPY Cargo.toml Cargo.lock ./
COPY crates/huginn-core/Cargo.toml   crates/huginn-core/Cargo.toml
COPY crates/huginn-probes/Cargo.toml crates/huginn-probes/Cargo.toml
COPY crates/huginn-influx/Cargo.toml crates/huginn-influx/Cargo.toml
COPY crates/huginn-web/Cargo.toml    crates/huginn-web/Cargo.toml
COPY huginn/Cargo.toml           huginn/Cargo.toml

# Dummy source files to cache the dependency layer
RUN mkdir -p crates/huginn-core/src   && echo "pub fn _f(){}" > crates/huginn-core/src/lib.rs \
 && mkdir -p crates/huginn-probes/src && echo "pub fn _f(){}" > crates/huginn-probes/src/lib.rs \
 && mkdir -p crates/huginn-influx/src && echo "pub fn _f(){}" > crates/huginn-influx/src/lib.rs \
 && mkdir -p crates/huginn-web/src    && echo "pub fn _f(){}" > crates/huginn-web/src/lib.rs \
 && mkdir -p huginn/src           && echo "fn main(){}"   > huginn/src/main.rs \
 && cargo build --release --locked \
 && rm -rf crates/*/src huginn/src

# Real build — touch source files so cargo detects them as newer than the
# dummy artifacts from the dependency-caching step above.
COPY . .
RUN find . -name "*.rs" -exec touch {} \; && cargo build --release --locked

# ── Stage 2: Runtime (distroless — no shell, no package manager) ──────────
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /build/target/release/huginn /usr/local/bin/huginn
COPY config/config.example.yaml /etc/huginn/config.yaml

# Run as non-root
USER nonroot:nonroot

# 9116 = debug UI, 9464 = Prometheus /metrics (both optional, off by default).
# The liveness listener (health.port, 9115 by default) is deliberately NOT
# exposed: it is bound to 127.0.0.1 and is only ever reached from inside this
# container's network namespace, which is where HEALTHCHECK runs.
EXPOSE 9116 9464

# The binary checks itself. Distroless has no shell and no curl, so there is
# nothing else here that could make an HTTP request — which is why `healthcheck`
# is a subcommand rather than a CMD wrapping some tool.
#
# Exec form, so no shell is required to parse it. It reads the same config as
# the daemon (HUGINN_CONFIG, or the ENTRYPOINT's path by default), so the two
# cannot disagree about which port to use.
#
# start-period covers config load and the bind; the check itself only ever waits
# on loopback, so the timeout is short on purpose — a slow answer here means the
# runtime has stopped scheduling, which is the thing worth reporting.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/huginn", "--config", "/etc/huginn/config.yaml", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/huginn", "--config", "/etc/huginn/config.yaml"]
