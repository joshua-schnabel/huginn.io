# ── Stage 1: Build ────────────────────────────────────────────────────────
FROM rust:1.81-slim AS builder

WORKDIR /build

# Cache dependencies before copying source
COPY Cargo.toml Cargo.lock ./
COPY crates/hugin-core/Cargo.toml   crates/hugin-core/Cargo.toml
COPY crates/hugin-probes/Cargo.toml crates/hugin-probes/Cargo.toml
COPY crates/hugin-influx/Cargo.toml crates/hugin-influx/Cargo.toml
COPY hugin-dec/Cargo.toml           hugin-dec/Cargo.toml

# Dummy source files to cache the dependency layer
RUN mkdir -p crates/hugin-core/src   && echo "pub fn _f(){}" > crates/hugin-core/src/lib.rs \
 && mkdir -p crates/hugin-probes/src && echo "pub fn _f(){}" > crates/hugin-probes/src/lib.rs \
 && mkdir -p crates/hugin-influx/src && echo "pub fn _f(){}" > crates/hugin-influx/src/lib.rs \
 && mkdir -p hugin-dec/src           && echo "fn main(){}"   > hugin-dec/src/main.rs \
 && cargo build --release --locked \
 && rm -rf crates/*/src hugin-dec/src

# Real build
COPY . .
RUN cargo build --release --locked

# ── Stage 2: Runtime (distroless — no shell, no package manager) ──────────
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /build/target/release/hugin-dec /usr/local/bin/hugin-dec
COPY config/config.example.yaml /etc/hugin/config.yaml

# Run as non-root
USER nonroot:nonroot

EXPOSE 9116

ENTRYPOINT ["/usr/local/bin/hugin-dec", "--config", "/etc/hugin/config.yaml"]
