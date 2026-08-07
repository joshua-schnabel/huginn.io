# ADR-0003 — TLS is rustls only, and OpenSSL is banned outright

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

huginn speaks TLS in three places: HTTPS probes, the TLS certificate-expiry
probe, and the InfluxDB writer. In Rust there are two realistic backends —
rustls, and `native-tls`, which delegates to OpenSSL or the platform stack.

OpenSSL brings a C library into a project whose whole point is a small,
scannable image, and it brings the CVE cadence that comes with it. It also
brings a build-time dependency on system headers, which is precisely the thing
that makes a distroless runtime image awkward.

The decision is easy. Keeping it made is the hard part: a single transitive
dependency that enables a `native-tls` feature by default pulls OpenSSL back in,
and nothing about the build fails when it does.

## Decision

rustls everywhere, and the policy is enforced by `deny.toml` rather than by
intent. Four crates are banned outright:

| Banned | Why |
|---|---|
| `openssl` | TLS is rustls only |
| `openssl-sys` | pulls in the OpenSSL C library |
| `native-tls` | delegates to the platform/OpenSSL stack |
| `tokio-native-tls` | `native-tls` wrapper; use `tokio-rustls` |

`cargo deny check` runs in CI on every push and PR, so the ban is a gate, not a
preference. `reqwest` is configured with `use_rustls_tls()` explicitly rather
than relying on default features.

## Consequences

- The runtime image needs no system TLS library, which is part of what makes
  distroless viable ([ADR-0005](0005-distroless-nonroot.md)).
- Trivy findings against OpenSSL cannot occur, because the code is not there.
- A dependency that only offers `native-tls` cannot be adopted without revisiting
  this ADR. That has not happened yet, and it is the intended cost.
- The ban catches the transitive case, which is the one that actually happens. A
  direct `openssl` dependency would be noticed in review; a feature flag two
  levels down would not.

**This was not always enforced.** The policy existed in the documentation while
`deny.toml` merely allowed the OpenSSL licence, so nothing would have stopped a
transitive pull-in. The bans, and the removal of that licence entry, closed it.

## Alternatives considered

**Rely on `default-features = false` at each dependency.** Rejected: it is a
rule that must be remembered at every `cargo add`, and it says nothing about
transitive crates.

**Allow `native-tls` on non-container builds.** Rejected: the container is the
artefact. A configuration that is only tested in a shape nobody ships is a
configuration that is not tested.
