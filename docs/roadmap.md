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

**Cut a release.** Everything above the topmost `## [x.y.z]` heading in
[`CHANGELOG.md`](../CHANGELOG.md) is unreleased, and it is additive and
corrective throughout — and it carries the dependency updates that close the
advisories still open against `main`. It is also the first release under the
ref-scoped environments, so the first that proves `release` holds what the tag
push and the signed housekeeping commit need.

**Decide the libssl3 suppression before it lapses on 2026-11-30.**
`.trivyignore.yaml` suppresses CVE-2026-84782 in the distroless base, with the
reachability argument at the entry: huginn links neither libssl nor libcrypto
and speaks no DTLS. On that date the suppression expires and Trivy blocks the
pipeline. Either a rebuilt base image has landed by then — Dependabot's docker
group proposes it, and the entry is deleted — or the runtime moves to a base
without OpenSSL, which removes this whole class of alert rather than this one.

**Close the tag gap in `release`.** A tag push runs `release.yml` from the
tagged commit, so anyone able to push a `v*` tag reaches the `release`
environment with a workflow of their own — the "tag must be on main" check lives
in the file being replaced. Closing it needs a tag ruleset that lets only one
identity create `v*` tags, and that identity cannot be `RELEASE_PAT`, which acts
as the maintainer: the tag push has to move to an app first.
[`ci-cd.md`](ci-cd.md#where-the-secrets-live) has the reasoning. Best done after
a release has proved the environments as they are.

**Bump the two scanner images by hand.** The actionlint and Semgrep images in
`security.yml` are pinned by digest, and Dependabot does not update `container:`
or `docker run` references (`dependabot.yml` says why), so nothing proposes a
new version. Neither pin has moved since 2026-08-07. Semgrep's rules are fetched
at scan time, so an old image still scans with current rules; actionlint's
checks are in the image.

**What the second audit left that is not a finding.** Its five findings are all
fixed — [`security-audit.md`](security-audit.md#pass-2) has the status of each.
What remains are its hardening suggestions, none of them a known weakness:

- **Validate secret files as legal HTTP header values at load**, so a token with
  a control byte is refused at startup rather than after the queue has filled.
- **Narrow `allowed_actions`** from `all`, now that `sha_pinning_required` holds
  the pinning.

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
