# Risks and open questions

Live document. Risks are removed when they are resolved, not when they stop being
mentioned — R1 went that way when the listener limits landed. Numbers are not
reused, so a gap means something was fixed, not lost.

The 2026-08-02 audit's findings are closed and recorded in
[`security-audit.md`](security-audit.md); what is below is what remains open by
decision, plus the operational risks the audit did not cover.

## R2 — The debug UI is unauthenticated

**Severity: medium · Status: accepted trade, documented**

`/`, `/events` and `/metrics/latest` serve the complete probe inventory — names,
targets, error strings — to anyone who can reach the port. That is an
infrastructure map.

`metrics.api_key_file` protects **only** the Prometheus listener. Enabling it
while the UI is exposed protects nothing, because the UI serves the same data
without a key.

**Mitigation.** Off by default; binds `127.0.0.1` by default
([ADR-0007](adr/0007-debug-ui-has-no-cli-flag.md)); published on `127.0.0.1` by
the shipped compose file. Both listeners send a strict CSP with no
`unsafe-inline`, plus `nosniff`, `DENY`, `no-referrer` and `no-store`.

**Residual.** Every one of those is a deployment answer to an application
question. A UI exposed on purpose is exposed to everyone.

**Revisit if** anyone runs the UI outside loopback. Then it needs auth, not
documentation.

## R3 — The TLS probe only covers HTTPS ports

**Severity: low · Status: known limit, documented**

The `tls` probe reads the certificate out of an HTTPS response's TLS info, so the
endpoint has to speak HTTP over TLS. Raw TLS ports — IMAPS 993, SMTPS 465,
LDAPS — cannot be probed for expiry, even though they are exactly the kind of
certificate that expires unnoticed.

**Mitigation.** None. The limit is stated in the probe's documentation and in
[`configuration.md`](configuration.md).

**Fix.** A handshake-only client rather than a `reqwest` client. Bounded work,
nobody has needed it yet — [`roadmap.md`](roadmap.md).

## R4 — Unfixable base-image CVEs stay open in the Security tab

**Severity: low · Status: accepted, monitored**

Debian package CVEs in the distroless base, all LOW/MEDIUM, none with a patched
version published upstream. They appear as open code-scanning alerts. (17 of
them when this was last counted, on 2026-08-02 — the count moves with the base
image and is not worth chasing here; the Security tab has the current one.)

**Mitigation.** They are deliberately *not* filtered out — the open alert is the
audit trail. Only fixable CRITICAL/HIGH block the pipeline, so the gate stays
one people act on rather than one they switch off. `.trivyignore.yaml` exists
with its rules written down and no entries.

**Residual.** Unfixable today means blocking tomorrow: when Debian ships fixes,
the gate starts failing until the image is rebuilt. That is intended, and it
makes image currency an operational duty.

## R6 — A backend outage that outlives the process loses buffered results

**Severity: low · Status: accepted by design**

The retry queue is in memory. If huginn restarts while InfluxDB is down, whatever
was buffered is gone; and while it is down, `max_buffered_bytes` drops the oldest
batches.

**Mitigation.** The bound is configurable, drop-oldest keeps the newest data, and
the shutdown drain gives the writer a bounded window to flush before exit.

**Residual.** Bounded, known data loss during a long outage.

**Not planned to change.** Persisting the queue turns a stateless container into
one with a volume, to cover a case where the orchestrator is already restarting
things — [ADR-0004](adr/0004-bounded-retry-queue.md).

## O1 — Nothing has been released yet

**Open question, not a risk to a deployment**

There is no `v*` tag, so the whole release path — tag push, GitHub Release, SBOM,
housekeeping PR — has never run end to end here. Two bugs in it were found by
reading rather than by running (`ci-cd.md`), and both are fixed; a third would be
found the same way or not at all until the first release.

`release.yml`'s `workflow_dispatch` entry point exists so a partial release can
be completed by hand rather than re-cut.

## Related

- [`security-audit.md`](security-audit.md) — the closed findings and the method
- [`hardening.md`](hardening.md) — the mitigations referenced above
- [`roadmap.md`](roadmap.md) — what is planned about all this
