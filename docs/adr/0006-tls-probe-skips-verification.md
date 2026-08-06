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
- This client trusts nothing about the peer and sends nothing to it beyond a
  handshake and a bare request. There are no credentials on the connection and
  no response body is used, so accepting an invalid certificate carries no
  confidentiality risk in this narrow, read-only context.
- Semgrep flags `reqwest-accept-invalid` on the builder line. The suppression is
  a `// nosemgrep:` comment sitting on that exact line with the reasoning above,
  which is also why `security.yml` strips suppressed findings from the SARIF
  upload rather than leaving them open in the Security tab forever.
- The probe reads the certificate from an HTTPS response, so raw non-HTTP TLS
  ports (IMAPS, SMTPS) are out of scope. Recorded in
  [`risks.md`](../risks.md).

## Alternatives considered

**Verify, and treat a verification failure as "expired".** Rejected: it cannot
tell an expired certificate from a hostname mismatch or an untrusted CA, and it
produces no number.

**A second, verifying probe type alongside this one.** Not rejected on merit —
"does this certificate chain validate" is a genuinely different and useful
question. It is simply not implemented; it belongs in
[`roadmap.md`](../roadmap.md), not in this probe.
