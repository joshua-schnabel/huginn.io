# ADR-0006 — The TLS probe skips certificate verification

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

The `tls` probe exists to answer one question: how many days until this
certificate expires. The whole value of that number is that it is available
*before* someone is paged, and still available *after* the certificate has
expired — `tls_cert_expiry_days` goes negative and the probe reports DOWN.

A verifying TLS client cannot answer it. Verification fails on an expired
certificate, so the handshake aborts and the certificate is never read. The
probe would report DOWN with no metric attached, in precisely the situation the
probe was built for — and it would report the same DOWN for an expired
certificate, a self-signed certificate, a hostname mismatch and an unreachable
host, which are four different problems.

## Decision

The TLS probe's HTTP client is built with `danger_accept_invalid_certs(true)`,
and only that client. Verification stays on everywhere else — HTTPS probes use a
separate, verifying client from the registry.

The probe reads the certificate out of the connection's TLS info and computes
the days remaining. `up` is then decided by two things: the handshake completed,
**and** the certificate has at least `tls_expiry_fail_days` days left (default 0,
so an expired certificate is DOWN).

Redirects are disabled on this client, so the certificate measured is the one at
the address configured.

## Consequences

- Expired and self-signed certificates are readable, which is the requirement.
- The probe reports the *number*, so an operator can distinguish "expires in
  three days" from "expired last week" from "host unreachable" — a plain DOWN
  cannot.
- This client trusts nothing about the peer and sends nothing to it at all: the
  handshake completes, the certificate is read, the socket is closed. There are
  no credentials on the connection and no application data in either direction,
  so accepting an invalid certificate carries no confidentiality risk in this
  narrow, read-only context.
- Semgrep flags the dangerous-verifier construction. The suppression is a
  `// nosemgrep:` comment sitting on that exact line with the reasoning above,
  which is also why `security.yml` strips suppressed findings from the SARIF
  upload rather than leaving them open in the Security tab forever.

### Amendment, 2026-08-09 — how the certificate is obtained

The decision above is unchanged; what changed is the transport it applies to.

The probe originally read the certificate out of an HTTPS **response**, using a
`reqwest` client with `danger_accept_invalid_certs`. That worked, and it meant
the endpoint had to speak HTTP over TLS — so IMAPS, SMTPS and LDAPS were out of
scope, even though their certificates expire exactly like any other and are
rather more likely to do so unnoticed. It was recorded as R3 in `risks.md`.

The probe now performs the TLS handshake directly (`tokio-rustls`) and takes the
peer certificate from the session. No application-protocol request is made,
which is precisely what makes any TLS port probeable: a server presents its
certificate during the handshake, before either side says anything, so a
protocol where the *server* speaks first is no obstacle.

Two consequences worth stating:

- **The verifier is now huginn's own code**, a `ServerCertVerifier` that returns
  `assertion()`, rather than a flag on someone else's client. That is more
  explicit about what is being skipped, and it is confined to one connector.
- **STARTTLS is still out of scope**, and now that is the only gap. A port that
  begins in plaintext and upgrades on command is not a TLS port until the
  command is sent, and sending it would mean teaching this probe each
  application protocol's upgrade handshake. Probe the implicit-TLS port instead.

R3 is removed from [`risks.md`](../risks.md) rather than marked done, which is
that file's own rule.

## Alternatives considered

**Verify, and treat a verification failure as "expired".** Rejected: it cannot
tell an expired certificate from a hostname mismatch or an untrusted CA, and it
produces no number.

**A second, verifying probe type alongside this one.** Not rejected on merit —
"does this certificate chain validate" is a genuinely different and useful
question. It is simply not implemented; it belongs in
[`roadmap.md`](../roadmap.md), not in this probe.
