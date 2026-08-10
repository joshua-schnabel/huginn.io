# Risks and open questions

Live document. Risks are removed when they are resolved, not when they stop being
mentioned — R1 went that way when the listener limits landed. Numbers are not
reused, so a gap means something was fixed, not lost.

The 2026-08-02 audit's findings are closed and recorded in
[`security-audit.md`](security-audit.md); what is below is what remains open by
decision, plus the operational risks the audit did not cover.

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

## O1 — The release path has run, but not yet under one build

**Open question, not a risk to a deployment**

Three releases have been cut, so the path is no longer untested — but every one
of them ran it in the shape that built the image twice, and the fix for that
(`ci.yml` no longer triggering on tags, plus the digest recorded in the tag and
re-checked before the Release is created) has not yet been exercised by a real
release.

`release.yml`'s `workflow_dispatch` entry point exists so a partial release can
be completed by hand rather than re-cut.

**Closes when** one release completes end to end with a single build, and the
digest in the GitHub Release matches the one the tag records.

## Related

- [`security-audit.md`](security-audit.md) — the closed findings and the method
- [`hardening.md`](hardening.md) — the mitigations referenced above
- [`roadmap.md`](roadmap.md) — what is planned about all this
