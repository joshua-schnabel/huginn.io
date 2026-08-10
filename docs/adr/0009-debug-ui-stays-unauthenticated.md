# ADR-0009 — The debug UI stays unauthenticated

**Status:** accepted · **Date:** 2026-08-09

## Context

The debug UI serves `/`, `/events` and `/metrics/latest`: every probe name,
every target, every error string. That is an infrastructure map, and anyone who
can reach the port gets it. It has no authentication.

The Prometheus listener next to it *does* have optional authentication —
`metrics.api_key_file`, fail-closed, constant-time comparison. The asymmetry is
the uncomfortable part: an operator who sets `metrics.api_key_file` and exposes
the UI has protected nothing, because the UI serves the same inventory without a
key. That asymmetry was carried as **R2** in [`risks.md`](../risks.md) and as an
open question on the roadmap, phrased as "authentication for the debug UI, or a
decision not to have it".

This ADR is that decision. It has been deferred through three releases, and an
open question that never closes is indistinguishable from an oversight — which
is the actual risk here, more than the missing auth.

## Decision

**The debug UI does not get authentication, now or in 1.0.**

What protects it instead, unchanged:

- **Off by default.** `ui.enabled` defaults to `false`.
- **Loopback by default**, and in a container that default is *wrong on
  purpose*: a published port reaches the container's bridge IP, never its
  loopback, so exposing the UI takes two deliberate settings rather than one
  ([ADR-0007](0007-debug-ui-has-no-cli-flag.md)).
- **Published on `127.0.0.1` by the shipped compose file**, so following the
  quick start does not expose it either.
- **Strict security response headers** on both listeners — a CSP with no
  `unsafe-inline`, `nosniff`, `DENY`, `no-referrer`, `no-store` (audit F-02).
- **A connection cap and a header-read timeout**, so an unauthenticated peer
  cannot grow the process without bound (audit F-03).

And the documentation says plainly, in the README, `configuration.md` and
`hardening.md`, that the UI is unauthenticated and that `metrics.api_key_file`
does not cover it.

## Consequences

- **A UI exposed on purpose is exposed to everyone who can route to it.** That
  is the residual risk, it is not mitigated by anything above, and it is
  accepted. R2 is removed from `risks.md` — that file's rule is that a risk
  leaves when it is resolved, and "decided" is a resolution even though the
  behaviour is unchanged.
- Anyone who needs the UI on a network puts it behind something that already
  does authentication properly: a reverse proxy, an SSH tunnel, a VPN. Those are
  better at it than huginn would be, and they are already deployed.
- The Prometheus endpoint keeps its optional key, because its use case is the
  opposite: a scraper on another host is the *normal* deployment, not the
  exceptional one.
- If someone does run the UI outside loopback and wants it protected in huginn
  itself, this ADR is what they should argue with. It is a decision, not a
  permanent fact.

## Alternatives considered

**`ui.api_key_file`, mirroring `metrics.api_key_file`.** The obvious symmetry,
and genuinely cheap — the file reading, the fail-closed handling and the
constant-time comparison already exist and would be reused rather than written.
Rejected on what it would actually buy: a bearer token typed into a browser is
not a session, so it means either a query parameter (which lands in logs and
`Referer` headers) or a proxy holding the header — and if there is a proxy, the
proxy can do the authentication. It would add a credential to huginn's
threat model and a second secret file to every deployment, to protect a debug
tool that is off by default, in exchange for security that is weaker than the
tunnel most operators would use anyway.

**HTTP Basic auth over plain HTTP.** Rejected: the UI has no TLS, so the
credential crosses the network in base64 on every request. That is worse than no
authentication, because it looks like some.

**Bind the UI to loopback and refuse to start otherwise.** Rejected as too
strict for a debug tool, and it would break the container case that
[ADR-0007](0007-debug-ui-has-no-cli-flag.md) already documents: `0.0.0.0` inside
a container is normal and correct when the *host* side publishes on
`127.0.0.1`.

**Remove the debug UI.** Not seriously considered, but worth recording: it is
the feature people use while getting their config right, and taking it away
would push them towards exposing something worse.

## Related

- [ADR-0007](0007-debug-ui-has-no-cli-flag.md) — why it is config-gated and
  loopback-bound
- [ADR-0008](0008-liveness-listener-on-by-default.md) — the other listener
  decision, which went the other way and explains why
- [`hardening.md`](../hardening.md) — the posture this sits inside
- [`security-audit.md`](../security-audit.md) — where it is listed as accepted
  risk 1
