# ADR-0005 — A distroless, nonroot runtime image

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

huginn is a single static-ish Rust binary that opens outbound sockets and,
optionally, two loopback listeners. It needs no package manager, no shell, no
`coreutils` and no init system at runtime.

Every one of those, if present, is attack surface and CVE surface — and CVE
surface is not abstract here: Trivy blocks the pipeline on fixable
CRITICAL/HIGH findings, so a runtime base with a large package set means either
a frequently red pipeline or a gate that gets relaxed.

## Decision

Runtime base is `gcr.io/distroless/cc-*`, running as the image's nonroot user;
the exact tag is pinned in the Dockerfile and moves with Debian. The builder
stage is a normal `rust:*-slim`; nothing from it reaches the final image except
the binary.

`cc` rather than `static`: the binary links against glibc. rustls means no
system TLS library is needed ([ADR-0003](0003-rustls-only.md)), which is what
makes `cc` sufficient.

The container is additionally run with `read_only: true`, `cap_drop: ALL` and
`no-new-privileges`, and the shipped compose file publishes both listeners on
`127.0.0.1` — the debug UI is unauthenticated, and InfluxDB holds every
measurement.

## Consequences

- The image carries roughly ten packages. Most CVE feeds have nothing to say
  about it, and the blocking Trivy gate is realistic to keep at
  fixable-CRITICAL/HIGH.
- There is no shell in the image. `docker exec` for debugging does not work, and
  that is the intended trade — the debug UI, `/metrics` and the logs are the
  supported ways to see what huginn is doing.
- The config must be mounted read-only and the token must arrive as a file
  ([ADR-0002](0002-secrets-from-files-only.md)); with a read-only root
  filesystem there is nowhere to write one anyway.
- A dependency that needs a system library at runtime cannot be adopted without
  revisiting this.

## Alternatives considered

**`debian:12-slim`.** Rejected here: it costs roughly 88 packages instead of ten
and a shell in the image, and huginn needs none of it. (muninn.io does make the
opposite call, because reading a host's package state needs real `apt` and
`dpkg` — the trade is measured in its own `docs/hardening.md`. That the two
sibling projects choose differently is the point: the requirement decides.)

**`scratch` with a fully static musl build.** Rejected: musl's allocator and DNS
resolver behave differently enough under load to be worth avoiding for a
latency-measuring tool, and the gain over distroless `cc` is a handful of
megabytes.
