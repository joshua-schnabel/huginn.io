# ── Stage 1: Build ────────────────────────────────────────────────────────
FROM rust:1.81-slim AS builder

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

# Real build
COPY . .
RUN cargo build --release --locked

# ── Stage 2: Runtime (distroless — no shell, no package manager) ──────────
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /build/target/release/huginn /usr/local/bin/huginn
COPY config/config.example.yaml /etc/huginn/config.yaml

# Run as non-root
USER nonroot:nonroot

EXPOSE 9116

ENTRYPOINT ["/usr/local/bin/huginn", "--config", "/etc/huginn/config.yaml"]
