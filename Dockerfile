# ── Stage 1: Build ────────────────────────────────────────────────────────
# Keep in sync with rust-version in the workspace Cargo.toml.
# 1.88 is the floor set by hickory-resolver 0.26, which is required for
# RUSTSEC-2026-0119 — 0.24 pins a vulnerable hickory-proto.
#
# Pinned by digest, with the tag kept in front of it. Every GitHub Action in
# this repository is pinned to a 40-character SHA because a tag is movable and a
# compromised upstream would otherwise reach CI with no Dependabot PR; a base
# image tag is movable in exactly the same way, and it is the one that ends up
# inside the artefact people run. The `tag@digest` form keeps the version
# readable — and Dependabot updates both halves together, so this does not
# freeze the builder in place.
FROM rust:1.96-slim@sha256:31ee7fc65186be7e0e0ccb3f2ca305f14e4739e7642a1ae65753aa5d7b874523 AS builder

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
# Digest-pinned for the same reason as the builder, and it matters more here:
# these bytes ship. `cc-debian12` has no version tag to speak of, so before this
# the runtime layer was whatever the tag pointed at on the day of the build, and
# two builds of one commit could differ. R4 in docs/risks.md makes keeping this
# current an operational duty; Dependabot moves the digest, and the Trivy gate
# is what says when it must.
FROM gcr.io/distroless/cc-debian12@sha256:e8e7ee4b8b106d4c5fde9e422a321b2b8a2d5cca546c97adcce927f3e1d36e36

COPY --from=builder /build/target/release/huginn /usr/local/bin/huginn
COPY config/config.example.yaml /etc/huginn/config.yaml

# Run as non-root
USER nonroot:nonroot

# 9116 = debug UI, 9464 = Prometheus /metrics (both optional, off by default)
EXPOSE 9116 9464

ENTRYPOINT ["/usr/local/bin/huginn", "--config", "/etc/huginn/config.yaml"]
