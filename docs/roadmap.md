# Roadmap

What is left, and what is deliberately not being done. What already shipped is in
[`CHANGELOG.md`](../CHANGELOG.md); why it was built that way is in
[`adr/`](adr/).

## Where things stand

huginn works. Eight probe types, InfluxDB with a bounded retry queue, an optional
debug UI, an optional Prometheus endpoint, a distroless nonroot image built for
`linux/amd64` and `linux/arm64`, and a pipeline that builds, scans,
integration-tests and publishes one artefact.

**Several releases have shipped.** The current version is the topmost
`## [x.y.z]` heading in [`CHANGELOG.md`](../CHANGELOG.md) — read it there rather
than from a number written into this sentence, which would be wrong the morning
after the next release. The image is published to Docker Hub and mirrored
byte-identically to ghcr.

The release path was repaired along the way: the tag is pushed with
`RELEASE_PAT` so `release.yml` actually fires, the version bump writes
`Cargo.toml` and `Cargo.lock` together so `--locked` jobs do not fail on the
release PR, and `ci.yml` no longer runs on tags — which is what stopped every
release building twice. See [`ci-cd.md`](ci-cd.md).

**A stable major version has shipped**, so the surfaces in
[`versioning.md`](versioning.md) are promises now rather than intentions: the
config schema, the CLI, the InfluxDB schema, the container contract, probe
semantics and the Prometheus metric names cannot change incompatibly without a
major version. What is left below is additive or corrective, and none of it
touches those surfaces.

## Next

**Self-monitoring for the write path.** Retry-queue evictions, permanently
rejected batches and writer health are invisible from outside the process: the
probe gauges cannot show that measurements were taken and then dropped, which is
exactly the failure mode huginn is built around. A new metric family is additive
under [`versioning.md`](versioning.md), so this can land in a minor release — and
its names join the stable surface when it does.

**Re-run the security audit.** Listener handling, secret-file behaviour, the
shutdown path and the TLS transport have all changed since the 2026-08-02 pass,
and that pass's own closing recommendation was to repeat it after exactly these
kinds of change. Scope and method are written down in
[`security-audit.md`](security-audit.md), so it is a repeat rather than a fresh
invention — which is what makes it the largest return per hour of anything here.

**The debug UI collapses distinct probe names into one row.** A row's DOM id is
derived by replacing every character outside `[A-Za-z0-9_-]` with `_`, so
`db.primary` and `db/primary` produce the same id and overwrite each other: two
probes are configured, one row is shown, and nothing says so. Name validation
requires only non-empty and unique, so both names are legal. It is confined to
the UI — `escape_label` and `escape_tag` escape rather than replace, so
Prometheus and InfluxDB keep the two apart — and the UI is explicitly unstable
([`versioning.md`](versioning.md)), which is why this sits below the two above
rather than in front of them. A collision-free row key fixes it, and that change
is the moment to also reconcile the initial `/metrics/latest` snapshot with the
`/events` subscription: the subscription is opened after the snapshot request,
and `/events` does not replay, so a result landing between the two is not shown
until the probe next runs.

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
- [`releasing.md`](releasing.md) — how a release gets cut
- [`versioning.md`](versioning.md) — what a version number promises
