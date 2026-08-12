# Hardening

How huginn.io is secured: secrets handling, container hardening, and the scanning
pipeline. To **report a vulnerability**, see [`SECURITY.md`](SECURITY.md). For
the findings of the last full review — including the risks that are open by
decision — see [`security-audit.md`](security-audit.md).

## Secrets

huginn.io follows a strict **no-secrets-in-ENV** policy.

### ✅ Correct: Token in a file
```yaml
influx:
  token_file: "/run/secrets/influx_token"
```
The token is read from the file at startup. The file should be:
- Owned by `root` or the service user
- Mode `0600` (readable only by owner)
- Mounted as a Docker secret (ephemeral tmpfs, never persisted to disk)

### ❌ Wrong: Token in ENV
```bash
# DO NOT do this
INFLUX_TOKEN=mytoken huginn   # token visible in ps, /proc/environ, logs
```

## Docker secrets
```yaml
# docker-compose.yml
services:
  huginn:
    secrets:
      - influx_token
    environment:
      INFLUX_TOKEN_FILE: /run/secrets/influx_token  # ← path, not value

secrets:
  influx_token:
    file: ./secrets/influx_token.txt  # ← never commit this file
```

Add to `.gitignore`:
```
secrets/
*.token
.env
```

## The container

| Measure | Details |
|---|---|
| **Distroless base** | the `gcr.io/distroless/cc-*` image pinned in the [`Dockerfile`](../Dockerfile) — no shell, no apt, minimal surface |
| **Non-root user** | Runs as `nonroot:nonroot` |
| **No capabilities** | `cap_drop: ALL` — huginn binds ports above 1024 and never changes uid, so it needs none |
| **No privilege escalation** | `security_opt: no-new-privileges:true` |
| **Read-only rootfs** | `read_only: true` — huginn writes nothing to disk: config is `:ro`, the token is a tmpfs secret, results go out over HTTP |
| **Read-only config** | Config file mounted `:ro` in compose |
| **Resource limits** | `mem_limit: 256m`, `pids_limit: 128` — bounds the blast radius of a connection flood (see below) |
| **Loopback publishing** | Compose publishes `127.0.0.1:9116` / `127.0.0.1:8086`; widening it is a deliberate edit |
| **rustls** | TLS via pure-Rust rustls — no OpenSSL. Enforced, not just intended: `deny.toml` bans `openssl`, `openssl-sys`, `native-tls` and `tokio-native-tls` |

### Request limits on the HTTP listeners

Both listeners cap concurrent connections at **256** and give a peer **10
seconds** to send its request head. A connection that has sent nothing complete
by then is dropped.

This closes [F-03](security-audit.md#f-03). The measurement that opened it: 4 000
idle half-open connections raised RSS from 29 MiB to 113 MiB (~21 KiB each) while
both listeners kept serving normally, and nothing capped the count — the bound
was the container's memory limit, not the application.

**Neither limit is a `tower` layer, and that is the point.** A layer wraps the
service, and the service is not reached until hyper has parsed a request; a
request head that never completes never arrives, so `TimeoutLayer` would have
run zero times against this attack. The limits sit below the service instead:
the cap is a semaphore permit taken *before* `accept`, so at capacity peers wait
in the kernel backlog rather than each costing a task, and the deadline is
hyper's own `header_read_timeout`.

It bounds the **head**, not the connection — the SSE stream on `/events` stays
open as long as the browser wants it. `crates/huginn-web/src/serve.rs` carries
the reasoning and the tests, including one that fails if the timeout is removed.

Keep the listeners on loopback or a trusted network all the same, which is the
default on both counts.

## Metrics endpoint auth

The Prometheus `/metrics` listener is unauthenticated by default (off, loopback
— same posture as the debug UI). For scraping across an untrusted network, set
`metrics.api_key_file`: requests then need `Authorization: Bearer <key>`. The
key follows the file-only secrets policy above (Docker secret, mode `0600`,
never in YAML/ENV), a broken or empty key file **stops startup** instead of
falling back to unauthenticated, and the key comparison is constant-time.

## Untrusted data from monitored hosts

Every probe talks to an endpoint that may be hostile or compromised, and one
field carries what it says back: `ProbeResult.error`. The SMTP and IMAP probes
copy the remote banner/greeting into it verbatim (up to 512 bytes), and that
string then reaches the operator's console, InfluxDB and every HTTP consumer.

`ProbeResult::failure` therefore **escapes control characters** — `\n`, `\r`,
`\t` by name, the rest as `\xHH` — for the whole Cc range (U+0000–U+001F,
U+007F–U+009F). Without it a monitored host could emit ANSI/OSC sequences that
recolour the console, move the cursor, set the terminal title, or use CR to
overwrite the line just written, forging or hiding log output. The payload also
persists: it is stored in InfluxDB and fires again wherever that value is later
printed. Escaping sits at the single point where every probe builds a failure,
so every sink is covered at once.

The other sinks each keep their own escaping as a second layer: InfluxDB line
protocol escapes tags and quoted fields, the Prometheus exposition escapes label
values, and the debug UI HTML-escapes every cell before it reaches the DOM.

## Debug UI response headers

Both listeners send a strict `Content-Security-Policy`
(`default-src 'none'` plus same-origin script/style/connect), `nosniff`,
`X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` and
`Cache-Control: no-store`. The CSP deliberately omits `unsafe-inline`, which is
what makes an injected inline event handler inert — defence in depth behind the
escaping the UI already does, not a replacement for it.

The debug UI itself is **unauthenticated by design**: `/`, `/events` and
`/metrics/latest` expose the full probe inventory to anyone who can reach the
port. That is why it binds loopback by default, why compose publishes it on
`127.0.0.1`, and why `metrics.api_key_file` protects only the Prometheus
listener — enabling that key does **not** protect the UI, which serves the same
data.

## Deliberate exception: the TLS probe skips certificate verification

The `tls` probe's dedicated HTTP client is built with
`danger_accept_invalid_certs(true)` (plus a `nosemgrep` suppression on that
line in `crates/huginn-probes/src/tls.rs`). This is intentional and narrowly
scoped:

- The probe's job is to **read** the peer certificate and report its expiry —
  including certificates that are already expired or self-signed, which a
  verifying client would refuse to complete a handshake with.
- The connection carries no secrets and trusts nothing from the peer: the only
  thing taken from the response is the certificate itself.
- Every other TLS connection huginn makes (HTTP/HTTPS probes, InfluxDB writes)
  uses normal rustls verification.

The `nosemgrep` comment is the authoritative record of this acceptance:
`security.yml` strips suppressed findings from the SARIF upload (GitHub ignores
the SARIF `suppressions` property), so this reviewed exception does not linger
as a permanently-open alert in the Security tab. Only explicitly suppressed
matches are dropped — the rule stays active for any new occurrence.

Consequence to be aware of: the TLS probe does **not** detect an invalid chain
or a hostname mismatch — it only measures expiry of whatever certificate the
endpoint presents.

## Supply chain

`cargo-deny` replaces `cargo-audit` and adds license and registry checks:

```bash
cargo deny check
```

Configuration in `deny.toml`:

- **Advisories** — the RustSec advisory database
- **Licences** — an allow-list of SPDX identifiers (MIT, Apache-2.0, BSD, ISC, …)
- **Bans** — `openssl`, `openssl-sys`, `native-tls` and `tokio-native-tls` are
  banned outright, which is what makes the rustls-only policy a gate rather than
  an intention ([ADR-0003](adr/0003-rustls-only.md)); duplicate versions warn
- **Sources** — `crates.io` only; git sources are denied

Runs automatically in CI (`supply-chain` job). Note that `deny.toml` must stay
*parseable*, not merely present: it once failed to load against a newer
cargo-deny, so the gate reported nothing while checking nothing.

## Source scanning

Three tools, all in `security.yml`, none of which needs a build:

- **Semgrep** — `p/rust` (unsafe patterns, taint flows) and `p/secrets`
  (hardcoded credentials). A full pass uploads to the Security tab and never
  blocks; a second pass blocks on ERROR severity.
- **ShellCheck** — `scripts/*.sh` at severity `warning`. Semgrep has no registry
  ruleset for shell, and these scripts drive the integration suite and the
  release version stamp.
- **actionlint** — the workflows: unknown `uses:` inputs, bad `needs`, and shell
  errors inside `run:` blocks.

A finding accepted with an in-code `// nosemgrep: <rule>` comment is stripped
from the SARIF upload, because GitHub ignores SARIF's `suppressions` property and
the finding would otherwise stay open forever. The comment, with its reasoning,
is the authoritative acceptance record — see the TLS probe exception above.

## Image scanning

```bash
# Scan locally with Trivy
docker build -t huginn .
trivy image --severity HIGH,CRITICAL huginn
```

Also runs automatically in CI before any image push.

Only **fixable** CRITICAL/HIGH findings block the pipeline. Unfixable
base-image CVEs (no patched Debian package exists yet) are **deliberately kept
visible** as open code-scanning alerts: they are accepted, monitored risk, and
filtering them out of the report would erase that audit trail. Once upstream
ships a fix, CRITICAL/HIGH findings start blocking CI; lower severities become
actionable in the Security tab.

## Related

- [`SECURITY.md`](SECURITY.md) — reporting a vulnerability
- [`security-audit.md`](security-audit.md) — every audit pass, newest first
- [`risks.md`](risks.md) — what stays open, and why
- [`ci-cd.md`](ci-cd.md) — the gates that enforce all of this
