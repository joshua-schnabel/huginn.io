# Risks and open questions

Live document. Risks are removed when they are resolved, not when they stop being
mentioned — R1 went that way when the listener limits landed. Numbers are not
reused, so a gap means something was fixed, not lost.

Audit findings live in [`security-audit.md`](security-audit.md), closed and open
alike; what is below is what remains open by decision, plus the operational risks
no audit covered. The 2026-08-12 pass left F-07 … F-11 open — they are tracked
there, as findings with reproductions, and move here only if a decision turns one
into an accepted risk.

## R4 — Unfixable base-image CVEs stay open in the Security tab

**Severity: low · Status: accepted, monitored**

Debian package CVEs in the distroless base, all LOW/MEDIUM, none with a patched
version published upstream. They appear as open code-scanning alerts. (19 of
them when this was last counted, on 2026-08-12 — 6 MEDIUM, 13 LOW, none fixable;
it was 17 on 2026-08-02. The count moves with the base image and is not worth
chasing here; the Security tab has the current one.)

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

## R7 — A connection flood denies the listener it floods

**Severity: low · Status: accepted by design**

The connection cap that closed [F-03](security-audit.md#f-03) bounds memory by
making excess peers wait for a permit, and permits free only when the
header-read deadline expires. So a peer that can reach an enabled listener can
hold its 256 permits with slow request heads and make legitimate requests to
*that listener* wait — measured on 2026-08-12 under a 4 000-connection
half-open flood, three of five timed out.

**Mitigation.** The permits are per listener, which is what keeps this narrow:
during the same flood the other two listeners answered in 0.3–4 ms throughout,
the container stayed healthy and `huginn healthcheck` kept exiting 0 — so a
flood of a published debug port cannot fail the container's HEALTHCHECK and make
an orchestrator restart a working monitor. Probing, batching and the InfluxDB
writer are untouched: measurement continues throughout. The deadline was cut
from ten seconds to three, which shortens the denial in proportion.

**Residual.** An optional, off-by-default, loopback-by-default debug surface can
be made unresponsive by whoever can already reach it. The debug UI is
unauthenticated by decision anyway
([ADR-0009](adr/0009-debug-ui-stays-unauthenticated.md)), so an attacker in that
position can already read everything it serves.

**Not planned to change.** The alternative to waiting is refusing, and refusing
requires per-peer accounting on a socket that is meant to be small. Bounding
memory at the cost of latency here is the right way round — the failure that
matters is the monitor dying, and it no longer can. F-09 of the 2026-08-12 pass.

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
