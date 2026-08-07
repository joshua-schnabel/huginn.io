# Roadmap

What is left, and what is deliberately not being done. What already shipped is in
[`CHANGELOG.md`](../CHANGELOG.md); why it was built that way is in
[`adr/`](adr/).

## Where things stand

huginn works. Eight probe types, InfluxDB with a bounded retry queue, an optional
debug UI, an optional Prometheus endpoint, a distroless nonroot image built for
`linux/amd64` and `linux/arm64`, and a pipeline that builds, scans,
integration-tests and publishes one artefact.

**Nothing has been released yet.** There is no `v*` tag and no GitHub Release;
`CHANGELOG.md` carries a large `## [Unreleased]` section — the Prometheus
endpoint, the TLS probe, the retry queue and the 2026-08-02 security audit are
all in it. Cutting the first release is the next thing that happens, either
through Actions → **Release (dispatch)** or the manual `dev → main` flow;
[`releasing.md`](releasing.md) has both.

The release path itself was repaired before that could work: the tag is now
pushed with `RELEASE_PAT` so `release.yml` actually fires, and the version bump
writes `Cargo.toml` and `Cargo.lock` together so `--locked` jobs do not fail on
the release PR. See [`ci-cd.md`](ci-cd.md).

## Next

**Decide F-03: request limits on the HTTP listeners.** Neither listener caps
concurrent connections or request duration. Either add a `tower-http` timeout and
concurrency-limit layer — a new dependency, so it needs approval — or write down
that the container's `mem_limit` and `pids_limit` are the accepted mitigation.
Open either way; it should not stay undecided.
[R1](risks.md), [`security-audit.md`](security-audit.md#f-03).

**Authentication for the debug UI, or a decision not to have it.** `metrics.api_key_file`
protects only the Prometheus listener; the UI serves the same probe inventory
unauthenticated. Today the answer is "off by default, loopback by default,
published on `127.0.0.1` by compose", which is a deployment answer to an
application question. [R2](risks.md),
[ADR-0007](adr/0007-debug-ui-has-no-cli-flag.md).

**Warn when a secret file is group- or world-readable.** The documentation
prescribes mode `0600` and nothing checks it. A stat at startup and a warning is
a small change with a real payoff, and it fits the fail-closed handling secrets
already get ([ADR-0002](adr/0002-secrets-from-files-only.md)).

**A `HEALTHCHECK` in the Dockerfile.** It needs a `healthcheck` subcommand on the
binary, because distroless has no shell and no `curl` — so it is a small feature
rather than a one-liner. muninn.io has one and it is worth matching.

**Raw TLS ports for the certificate probe.** The `tls` probe reads the
certificate from an HTTPS response, so IMAPS, SMTPS and other non-HTTP TLS ports
are out of scope. Doing it properly means a handshake-only client rather than a
`reqwest` client. [R3](risks.md),
[ADR-0006](adr/0006-tls-probe-skips-verification.md).

**Re-run the security audit** after any change to the probe result path, the HTTP
listeners, or the container definition. The last pass is dated 2026-08-02 and its
scope and method are written down, so a repeat is a repeat rather than a fresh
invention. [`security-audit.md`](security-audit.md).

## Not planned

- **Persisting the retry queue to disk.** It would turn a stateless container
  into one with a volume, to cover a case where the orchestrator is already
  restarting things. [ADR-0004](adr/0004-bounded-retry-queue.md).
- **Configuration reload.** Change the YAML, restart the process.
- **Alerting or notification.** huginn measures and writes; deciding what is
  worth waking someone for belongs in InfluxDB or Prometheus, where the rules
  and the silencing already exist.
- **A second, verifying TLS probe** — a genuinely different question, but not one
  anyone has asked for yet. [ADR-0006](adr/0006-tls-probe-skips-verification.md).

## Related

- [`risks.md`](risks.md) — the open risks these items come from
- [`releasing.md`](releasing.md) — how the first release gets cut
- [`versioning.md`](versioning.md) — what a version number will promise
