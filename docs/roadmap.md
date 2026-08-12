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

**Act on the second security audit.** The 2026-08-12 pass is done and found no
CRITICAL or HIGH; what it did find is five Low findings, open and each with a
reproduction, in [`security-audit.md`](security-audit.md#pass-2). In its own
priority order:

- **F-07** — control characters in a probe name reach the console, the Prometheus
  label values and the InfluxDB tags raw. The remote path has been escaped since
  F-01; the configuration path never was.
- **F-08** — the integration suite runs the container without the hardening the
  shipped compose file applies, so nothing keeps that hardening working. It does
  work today; that was verified separately, which is the only reason this is a
  missing gate rather than a broken setting.
- **F-10** — SHA-pinning of actions is held by hand. `sha_pinning_required` is
  the repository setting that would make it structural.
- **F-11** — the `publish` job's comment claims no third-party code runs between
  its credentialed checkout and the tag push. Four actions do.
- **F-09** — needs a decision rather than a fix: the connection cap converts
  memory exhaustion into denial of service on the flooded listener, which is
  the right trade and is written down nowhere.

The pass also carries three hardening suggestions that are not findings — see its
recommendations.

**Cut a release.** Everything above the topmost `## [x.y.z]` heading in
[`CHANGELOG.md`](../CHANGELOG.md) is unreleased, and it is additive and
corrective throughout. It is also the only way to close **O1** in
[`risks.md`](risks.md): three releases have been cut, all of them under the shape
that built the image twice, and the fix for that has never run under a real
release.

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
