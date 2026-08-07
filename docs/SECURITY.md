# Security policy

## Reporting a vulnerability

Please report security issues privately via
[GitHub's private vulnerability reporting](https://github.com/joshua-schnabel/huginn.io/security/advisories/new)
rather than opening a public issue.

Include what you need to make the case reproducible: affected version or commit,
configuration, and the observed behaviour. You can expect an acknowledgement
within a few days.

This file lives at a path GitHub recognises, which is what makes the
"Report a vulnerability" button appear. The security *practices* — secret
handling, container hardening, the scanning pipeline — are documented in
[`hardening.md`](hardening.md).

## Supported versions

Only the **latest release** receives security fixes. There are no maintenance
branches for older versions — upgrade to the newest release before reporting,
if possible. (What upgrading may entail is documented in
[`versioning.md`](versioning.md).)

## Scope

In scope:

- Leaking the InfluxDB token (config parsing, logs, error messages, the debug UI)
- Anything reachable from the debug UI on `:9116` — it has no authentication and
  is meant for a trusted network only; see below
- Container escape or privilege escalation from the distroless/nonroot image
- Dependency vulnerabilities not caught by `cargo deny check`

Out of scope:

- **The debug UI being unauthenticated.** This is known and deliberate. It is
  off by default (`ui.enabled: false`), binds `127.0.0.1` by default
  (`ui.bind`), and exposes probe results only. Reaching a wider network takes
  an explicit `ui.bind: "0.0.0.0"` — do not expose it to an untrusted network.
- Probe targets you configure yourself — huginn will connect wherever you point it.

## Related

- [`hardening.md`](hardening.md) — the posture behind this policy
- [`security-audit.md`](security-audit.md) — the last audit, in full
- [`versioning.md`](versioning.md) — which versions are supported
