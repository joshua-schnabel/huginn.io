# Security Policy

## Reporting a Vulnerability

Please report security issues privately via
[GitHub's private vulnerability reporting](https://github.com/joshua-schnabel/huginn/security/advisories/new)
rather than opening a public issue.

Include what you need to make the case reproducible: affected version or commit,
configuration, and the observed behaviour. You can expect an acknowledgement
within a few days.

This file lives at a path GitHub recognises, which is what makes the
"Report a vulnerability" button appear. The security *practices* — secret
handling, container hardening, the scanning pipeline — are documented in
[`docs/security.md`](../docs/security.md).

## Supported Versions

huginn.io is pre-1.0. Only the latest release on `main` receives fixes.

## Scope

In scope:

- Leaking the InfluxDB token (config parsing, logs, error messages, the debug UI)
- Anything reachable from the debug UI on `:9116` — it has no authentication and
  is meant for a trusted network only; see below
- Container escape or privilege escalation from the distroless/nonroot image
- Dependency vulnerabilities not caught by `cargo deny check`

Out of scope:

- **The debug UI being unauthenticated.** This is known and deliberate. It is
  off by default (`ui.enabled: false`) and exposes probe results only. Do not
  expose it to an untrusted network.
- Probe targets you configure yourself — huginn will connect wherever you point it.
