# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **`smtp` and `imap` no longer report a healthy server as DOWN when its greeting is split across TCP segments.** Both probes took whatever a single `read()` returned and tested its prefix. TCP is a byte stream and may split anywhere, so a perfectly valid `220 mail.example.com ESMTP` arriving as `22` and then the rest failed the check — while the server was fine. It is timing-dependent, which makes it worse than a constant bug: it shows up as a monitor that occasionally invents outages, which is the least believable kind of alert. Both now read until the line is complete, the peer closes, or 512 bytes (the RFC 5321 line limit) have arrived, so a peer that never sends a newline cannot make the probe hold an unbounded buffer.
- **`timeout_secs` is the budget for a whole probe, not for each step in it.** `smtp` and `imap` applied it once to the connect and again to the greeting read, so the real worst case was twice the configured value — and a probe could hold its loop for that long after a shutdown signal. Both steps now share one deadline.

### Changed

- **The HTTP latency boundary is documented rather than merely implemented.** `response_ms` for `http`/`https` ends when the response headers arrive; the body is never read. Including it would make the measurement depend on the size of whatever the endpoint returns, so a page that grew by a megabyte would be indistinguishable from a server getting slower. This was already the behaviour; it is now stated, and `docs/versioning.md` treats probe semantics as a stable surface.

## [0.3.0] - 2026-08-08

### Fixed

- **A secret file readable beyond its owner is reported.** `influx.token_file` and `metrics.api_key_file` were checked for existence, readability and emptiness but never for their mode, while the documentation prescribes `0600` — R5, and the second recommendation of the 2026-08-02 audit. A warning rather than a refusal: a read-only bind mount can carry permissions the operator does not control, and a token that works should not stop a deployment. Raised again as M-01 of muninn.io's own review, where the gap is sharper because that image carries a shell; the fix landed in both, which is what keeping the two aligned is for.
- **Both HTTP listeners cap connections and time out a request head.** 256 concurrent connections per listener, and ten seconds for a peer to send its head — a connection that has sent nothing complete by then is dropped. This closes F-03 of the 2026-08-02 audit, where 4 000 idle half-open connections took the shipped image from 29 MiB to 113 MiB with nothing refusing them. Notably *not* a `tower` layer, which is what the audit recommended and would not have worked: a layer wraps the service, and a request head that never completes never reaches one. The cap is a semaphore permit taken before `accept`, so peers wait in the kernel backlog instead of each costing a task, and the deadline is hyper's own `header_read_timeout`. It bounds the head rather than the connection, so the SSE stream on `/events` is unaffected. `hyper-util` becomes a direct dependency and adds nothing to the supply chain — it was already in the tree under axum, and `Cargo.lock` grows by one line.
- **The release workflows sign the commits they create.** Both branch rulesets carry `required_signatures`, and `prepare-dev` and `release-dispatch` committed with the runner's default identity — unsigned. The failure mode gave nothing away: the pull request opened, every required check went green, and the merge button stayed blocked with nothing reported as failing. Both jobs now import a key from `GPG_PRIVATE_KEY` and fail with a message naming the secret when it is absent, because an unsigned commit here produces an unmergeable pull request either way.
- **The signing identity is read from the key rather than hard-coded.** GitHub marks a signature *Verified* only when the commit's author email matches a UID on the key **and** a verified email on the account that owns it, so keeping `github-actions[bot]` beside a real key would have produced a valid signature that GitHub still labelled unverified. The UID's shape is asserted too: `Name <email>` is a convention, not a guarantee, and splitting a bare-string UID yields an "address" that is not one.
- **`prepare-dev` merges `main` back into `dev`, and no longer invents a changelog heading.** The release notes are written on the release branch and reach `main` through the release merge; `dev` never saw them, so the old logic found no `## [X.Y.Z]` section and synthesised an empty one — every release silently lost its notes on `dev`. The merge brings the real section, and with it the commits `dev` had never received, which is why it read "N commits behind main" with the gap growing every release. The housekeeping pull request is merged with a merge commit rather than a squash, or the merge would collapse to a single parent and undo itself. A conflict aborts with instructions instead of being guessed at.

## [0.2.1] - 2026-08-07

Fixes the tail of the release path. huginn's own code is untouched — the image
`0.2.1` publishes is `0.2.0` rebuilt from the same sources.

### Fixed

- **The release test report was never built.** `scripts/test-report.sh` failed with "no `test result:` lines found in input" against a log that contained fourteen of them, so `v0.2.0` shipped without its report. `CARGO_TERM_COLOR: always` is set workflow-wide, cargo colours its status lines even when piped to a file, and the escape sequences sit *before* the word — so `^\s*Running` never matched, no suite was ever opened, and every `test result:` line was skipped as belonging to nothing. The parser strips ANSI on the way in now. Stripping there rather than asking the caller for `CARGO_TERM_COLOR=never`: the input is a captured log, and a parser that only works on logs captured a particular way breaks on the next caller. muninn.io hit this at its own first release and fixed it the same way; huginn never received the fix.
- **`## [Unreleased]` was left closed after `v0.2.0`.** `release.yml`'s `prepare-dev` was skipped because the SBOM upload failed against a release GitHub had marked immutable, so the changelog was never reopened — and `release-dispatch.yml` refuses to cut a release without that section. Reopened by hand, with the compare link repointed at the released tag.

### Added

- **Security updates are moved onto `dev` automatically.** `target-branch: dev` covers *version* updates only; security updates ignore it and always open against the default branch, and no setting changes that. Left alone, such a PR merges a lockfile into `main` that `dev` has never seen, and the next release merge reverts it silently. `dependabot-auto-merge.yml` now retargets them and asks Dependabot to rebuild the branch against `dev` — the rebuild matters, because the original diff was resolved against `main`'s manifests and can be a *downgrade*: #51 proposed `rustls-webpki` 0.103.10 while `dev` already carried 0.103.13. A just-retargeted PR is never auto-merged in the same run.

### Changed

- **`dependabot.yml` no longer overstates what `target-branch` covers.** It claimed the setting stops bumps landing on `main`; it stops version updates only.

## [0.2.0] - 2026-08-07

### Added

- **`docs/architecture.md`, `docs/roadmap.md`, `docs/risks.md` and seven ADRs** — the structural documents huginn had none of. The ADRs record decisions already in the code: the event bus, secrets-from-files, rustls-only, the bounded retry queue, distroless/nonroot, the TLS probe's deliberate lack of certificate verification, and the debug UI having no CLI flag. Each names the alternative that was rejected.
- **`CLAUDE.md` and a committed `.claude/settings.json`** — the settings file is deliberately read-only: gates, inspection and `gh`/`git` queries, nothing that writes, pushes or merges.
- **A coverage percentage in the Actions job summary**, computed from `lcov.info`'s `LF`/`LH` records, plus a coverage-gate badge in the README.
- **Prometheus metrics endpoint** — a `/metrics` listener in Prometheus text format, gated independently of the debug UI (`metrics.enabled`/`bind`/`port`, default off on `127.0.0.1:9464`, `HUGINN_METRICS_*` ENV overrides). Exposes per-probe gauges `huginn_probe_success`, `huginn_probe_duration_seconds`, `huginn_probe_http_status_code`, `huginn_probe_last_run_timestamp_seconds`, plus every probe-specific reading as `huginn_probe_<key>` (e.g. `huginn_probe_tls_cert_expiry_days`). Optional `Authorization: Bearer` protection via `metrics.api_key_file` — file-only like every secret, fail-closed (a missing/empty key file stops startup), constant-time key comparison. Hand-rolled exposition format — no new dependency. Metric and label names are part of the stable surface (`docs/versioning.md`).
- **Release enrichment** — the GitHub Release now includes a "Container images" section (pull commands for DockerHub + ghcr, multi-arch manifest digest) and a human-readable test report: the release workflow re-runs the full test suite with coverage on the tagged commit, appends a summary to the notes, and attaches `test-report.md` as a release asset (`scripts/test-report.sh`).
- **TLS certificate-expiry probe** (`type: tls`) — completes a TLS handshake with an HTTPS endpoint (`host:port`) and emits the days until the server certificate expires as the `tls_cert_expiry_days` metric (negative once expired, still attached to DOWN results). The probe reports DOWN once the certificate has expired, or earlier via the optional `tls_expiry_fail_days` threshold (a negative threshold is rejected at load). Certificate verification is deliberately skipped so expired and self-signed certificates remain readable — see `docs/hardening.md`. New runtime dependency: `x509-parser`.
- **Release automation** — `release.yml` fires on the `vX.Y.Z` tag that `publish` creates: it opens the GitHub Release (notes pulled from this file's matching section, `0.x`/`-rc` flagged as pre-release) and opens an auto-merging PR into `dev` that reopens a fresh `## [Unreleased]`, fixes the compare links, and bumps `Cargo.toml`. The bot never pushes to `main`/`dev`.
- **CI version gate** — a release PR (`dev → main`) is blocked unless the top `CHANGELOG.md` version is valid SemVer and strictly greater than the latest `v*` tag; also re-checked before anything ships.
- **GitHub Container Registry mirror** — every published image is mirrored from DockerHub to `ghcr.io` with `skopeo copy --all`, byte-identical (same digests) to the scanned/tested image; no second build.
- **InfluxDB resilience** — the writer is split into a `run_batcher` (groups results, never awaits I/O) and a `run_writer` (drains a bounded `RetryQueue`). Failed writes are retried with exponential backoff instead of discarded; `WriteError` classifies transport/5xx/429/408 as retryable and 4xx as permanent (dropped). Retry is unbounded in attempts, bounded in memory (`max_buffered_bytes`, drop-oldest). New `influx` config keys: `max_buffered_bytes`, `retry_initial_backoff_ms`, `retry_max_backoff_ms`, `shutdown_drain_timeout_ms`.
- **`Probe` trait + `ProbeRegistry`** — per-protocol probes implement a common trait; the registry owns shared state (the HTTP client) so probe loops no longer thread resources they don't use.
- **`ProbeResult.metrics`** (`BTreeMap<String, f64>`) — a home for per-probe-type numeric readings, emitted as additional line-protocol fields. The TLS probe populates it with `tls_cert_expiry_days`.
- **Config validation** — rejects duplicate probe names, `event_hub_capacity: 0`, `batch_size: 0`, and per-type malformed targets (dns needs `ip:port`, tcp/smtp/imap/udp need a port, http/https need an absolute URL) at load time.
- **DNS probe** (`type: dns`) — resolves hostnames via configurable nameserver using `hickory-resolver`; optional `dns_expected_ip` validation
- **InfluxDB batch writes** — configurable `batch_size` and `batch_timeout_ms`; reduces HTTP traffic from 1 request per probe to batched line-protocol writes
- **Configurable EventHub capacity** — `event_hub_capacity` in app config (default 256)
- **System integration test** — `docker-compose.integration.yml` spins up InfluxDB + huginn (plus a Caddy sidecar serving a self-signed certificate) and runs curl-based assertions against the live stack, covering the tcp, http, dns, udp and tls probe types end-to-end
- **E2E tests** — multi-probe parallel execution, graceful shutdown, DNS probe E2E scenarios
- **`huginn-web` crate** — axum web server extracted into its own crate with SSE push updates, separate HTML/CSS/JS assets
- **EventHub architecture** — central `broadcast::Sender` in `huginn-core`; probes publish events, InfluxDB writer and web server subscribe independently
- **CI/CD redesign** — `ci.yml` (quality gate + gated DockerHub publish) and `security.yml` (Semgrep SAST + Trivy CVE)
- **SAST tooling** — Semgrep (`p/rust` + `p/secrets`) two-pass: SARIF upload + blocking on ERROR severity
- **Supply-chain security** — `deny.toml` for `cargo-deny`; replaces `cargo-audit` with advisories + license allow-list + registry restriction
- **Branch setup** — `main` (stable) and `dev` (integration) branches; direct push blocked via branch protection
- **DockerHub tags** — `:dev` + `:X.Y.Z-dev` on dev push; `:latest` + `:X.Y.Z` on main push
- **`docs/ci-cd.md`** — pipeline documentation and branch protection guide
- **`docs/testing.md`** — four-level test pyramid, TDD workflow, coverage requirements
- **`docs/versioning.md`** — the SemVer stability promise (config schema, CLI/ENV, InfluxDB schema), what stays unstable, the MSRV policy, and upgrade notes for 0.1.0 users

### Changed

- **The documentation follows huginn's sibling [muninn.io](https://github.com/joshua-schnabel/muninn.io) in shape**: the same README anatomy, the same ten-section `AGENTS.md`, sentence-case headings and a `## Related` footer on every page. The two projects are kept aligned deliberately, so a change to either has an obvious counterpart in the other.
- **No version number is repeated in prose.** A version lives where it is the authority — `rust-version` in `Cargo.toml`, the pins in the `Dockerfile` — and documentation names the field instead. Historical incidents and dated measurements keep their numbers and say when. This is now a convention in `AGENTS.md` §7.
- **The Docker builder tag is updated by Dependabot** and only has to sit at or above `rust-version`; the MSRV floor is unchanged. Bumping the builder is an update, not an MSRV change.
- `.cargo/config.toml` is commented in English, and `cargo audit-all` runs `cargo deny check` — the alias used to run cargo-audit, which is not the gate CI enforces. The broken `t-fail-fast` alias is gone: `--fail-fast` is rejected by libtest on stable.
- **The HTTP/HTTPS probe no longer follows redirects.** reqwest follows up to 10 hops by default, which meant `expected_status: 200` silently passed for a 301→200 chain and the measured `response_ms` included the extra round-trips. An uptime check has to judge the URL it was given, so a redirect is now reported with its own status — a 301 against `expected_status: 200` is DOWN. If you were relying on the old behaviour, point the probe at the redirect target instead.
- Project renamed from `hugin.dec` to `huginn.io`
- `cargo-audit` replaced by `cargo-deny` in all CI pipelines
- Docker image registry: GHCR → DockerHub
- `hickory-resolver` 0.24 → 0.26 (fixes RUSTSEC-2026-0119); raises the MSRV to Rust 1.88 (Dockerfile builder and `rust-version` bumped to match)
- Config precedence is now honoured in both directions: `--output`/`HUGINN_LOG_FORMAT` overrides `log.format` from the config file (previously an OR that could not override back to `pretty`)
- Invalid ENV overrides now warn and keep the previous value instead of being silently ignored
- **Leaner release build** — `[profile.release]` now strips symbols with thin LTO and `codegen-units = 1`; tokio is compiled with only the features huginn uses; `.dockerignore` excludes more non-build inputs.
- **BREAKING — the debug UI now binds `127.0.0.1` instead of `0.0.0.0`.** The address is the new `ui.bind` key (`HUGINN_UI_BIND`), validated as an IP address at load. It has no authentication and publishes every probe target, so reaching a wider network is now an explicit act. **Containers must set `0.0.0.0`** — a published port reaches the container's bridge IP, never its loopback; `docker-compose.yml` and `config/config.integration.yaml` do this already. Only setups that enable the UI (`ui.enabled` defaults to `false`) are affected.

### Removed

- **Codecov.** The coverage percentage is something this pipeline already computes; shipping every line of the source tree to a third party to have it computed again bought a badge and cost a dependency with repository access.
- `run_subscriber`, `run_subscriber_batched` and `InfluxWriter::write` — the old single-consumer writer paths. Replaced by the `run_batcher` + `run_writer` split (see below). Their meaningful behaviours (clean exit on hub close, surviving a lagged receiver) are now tested against the new tasks.
- The never-produced `packet_loss_pct` / `icmp_rtt_min_ms` / `icmp_rtt_max_ms` metric-key constants — no ICMP probe exists, and 1.0 should not freeze dead API surface.

### Fixed

- **`release.yml` could never have fired.** `ci.yml`'s `publish` pushed the release tag with the built-in `GITHUB_TOKEN`, and GitHub does not start a workflow from an event that token created — the recursion guard. The image, the ghcr mirror and the tag would ship; the GitHub Release, the test report and the housekeeping PR would not, with every job green. The tag push now uses `RELEASE_PAT`, and `release.yml` gains a `workflow_dispatch` entry point so an existing tag can be released by hand. muninn.io shipped the same wiring and hit it at its v0.1.0.
- **A release version bump could not resolve.** Every internal crate carried a second copy of the workspace version — `huginn-core = { path = "…", version = "0.1.0" }` in four manifests — so stamping a release made the crates `x.y.z` while the requirements still read `^0.1.0`, and `cargo` failed with *"failed to select a version for the requirement"* before a test could run, on both release paths. Internal dependencies are path-only now, with `publish = false` on the workspace, and `scripts/set-workspace-version.sh` writes `Cargo.toml` **and** `Cargo.lock` together because every CI job runs `--locked`.
- **Dependabot targeted `main`.** With no `target-branch` its PRs went at the release branch, bypassing `dev` — and the next `dev → main` PR would have silently reverted every bump. All ecosystems now target `dev`.
- **`release-dispatch.yml` merged into `main` with `--squash`**, contradicting the branch model that `docs/CONTRIBUTING.md` and `AGENTS.md` both state. It asks for a merge commit now.
- **`cargo deny check` rejected the workspace's own structure.** Path dependencies without a version are wildcards by cargo-deny's reading; `allow-wildcard-paths` covers exactly this case.
- **The `scan` job had no checkout**, so the blocking Trivy pass exited with *"cannot find ignorefile"* rather than scanning.
- **Dependabot's weekly cargo run had been failing since 2026-07-19** and opened no grouped dependency PRs at all. `serde` 1.0.229 (published the day before) requires `serde_core =1.0.229`, an exact pin, and neither `serde_core` nor `serde_derive` is named in a `Cargo.toml` — so with Dependabot's default `direct`-only scope it was allowed to move `serde` alone, which cargo cannot do. The lockfile came back unchanged, the updater raised `Failed to update serde!`, and every other crate in the group went down with it. Resolved by bumping `serde` in the lockfile by hand; widening the scope to `dependency-type: all` was tried first and reverted, because with transitive crates in scope the updater wrote a lockfile with a dangling `syn` reference that `cargo --locked` rejects. Security updates were unaffected throughout.
- **The UDP probe could never reach an IPv6 target.** The local socket was always bound to the IPv4 wildcard `0.0.0.0:0`, which cannot connect to an IPv6 peer, so a target that `validate()` had accepted reported DOWN forever. The target is now resolved first (under the probe timeout, so a stalling resolver can no longer exceed `timeout_secs`) and the socket is bound in the matching address family. A bind failure now also reports the elapsed time instead of a hardcoded `0.0`.
- **The daemon exited immediately at startup, having run no probes.** `run()` spawned the probe loops and returned; `main()` then exited and the Tokio runtime cancelled every task before the first tick. The fix (keep-alive on the shutdown channel) existed on `dev` but was lost when `feature/refactoring` branched from a parallel CI fix.
- **Tests could not observe that bug.** They all spawned `run()` into the test's own runtime, which outlives it — production has no such runtime. Added `huginn/tests/binary_lifecycle_test.rs`, which runs the real binary as a subprocess, and a negative shutdown test asserting `run()` does *not* return without a signal.
- **DockerHub publish ran in parallel with CI and depended on nothing** — a commit with failing tests, clippy or cargo-deny still shipped `:latest`. Publish is now a job in `ci.yml` gated by `needs` on every check, with `contents: write` so the release tag push no longer 403s.
- **`cargo deny check` was not running at all.** `deny.toml` used `severity-threshold`/`unlicensed`/`copyleft`, removed in cargo-deny ≥ 0.14, so the config failed to parse. Once repaired it surfaced four real advisories: `rustls-webpki` → 0.103.13 (RUSTSEC-2026-0098/-0099/-0104) and `hickory-resolver` 0.24 → 0.26 (RUSTSEC-2026-0119).
- **CI ran only on PRs targeting `main`/`dev`**, so feature branches had no gate — the condition that let the two branches diverge. It now runs on every PR.
- **Trivy skipped the release PR.** It was gated on `base_ref == 'dev'`, so `dev → main` was never scanned and CVEs surfaced only after landing on `main`. It now runs on every PR.
- **A newline in a probe error corrupted an entire InfluxDB batch** — line protocol is newline-delimited and `escape_field_str` did not escape `\n`. Also: `escape_tag` did not escape backslashes, and `urlencode` (formerly `urlenccode`) encoded code points rather than UTF-8 bytes, breaking non-ASCII org/bucket names.
- **The InfluxDB HTTP client had no timeout**, so a blackholing server would hang the batch subscriber — including its shutdown flush — indefinitely.
- **`--output pretty` could not override `format: json`** from the config file: the check was an OR, and `--output` always had a default, so "not given" and "explicitly pretty" were indistinguishable.
- **Invalid ENV values were swallowed** (`HUGINN_UI_PORT=abc`, `HUGINN_LOG_FORMAT=xml`, `HUGINN_UI_ENABLED=yes`). They now warn and keep the previous value; warnings are emitted after tracing initialises, since config is loaded before it exists.
- **Config errors that only surfaced at runtime** are now rejected at load: `event_hub_capacity: 0` (panicked in `broadcast::channel`), `batch_size: 0` (made every result its own POST), duplicate probe names (collided in the UI map and the InfluxDB series), and malformed per-type targets.
- Fixed a real flake: `run_with_ui_enabled_responds_to_health_check` slept a fixed 150 ms and made one unretried request.

### Security

- **No job runs `cargo` with a write token.** `actions/checkout` persists its token into `.git/config`, and `cargo` compiles and executes `build.rs` and proc-macros from every dependency in the tree — so four CI jobs handed the whole dependency tree a credential. Every checkout now sets `persist-credentials: false` except the three that push (`publish`, `prepare-dev`, `release-dispatch`), none of which runs cargo.
- **Credentials no longer reach `argv`.** `/proc/<pid>/cmdline` is readable by every process on the runner. `skopeo --dest-creds`/`--src-creds` became `skopeo login --password-stdin` with a mode-0600 `REGISTRY_AUTH_FILE` removed by a trap; the Docker Hub login body moved from `curl -d "{…$TOKEN…}"` to `jq -n --arg` piped into `curl --data @-`; the session JWT moved from `-H "Authorization: …"` to `curl -K -` on stdin.
- **ShellCheck and actionlint gate every push and PR.** Semgrep has no registry ruleset for shell, and `scripts/*.sh` drive the integration suite, the release version stamp and the release test report. actionlint additionally catches what YAML validity cannot — an unknown `uses:` input, a bad `needs`, a shell error inside a `run:` block.
- **`security-events: write` is scoped to the one job that uploads SARIF.** ShellCheck and actionlint only read the tree; actionlint additionally mounts it into a container.
- **An SBOM is produced twice:** per-architecture from the scanned image tarball in `ci.yml` (90-day artefact), and from the published multi-arch tag in `release.yml`, attached to the GitHub Release. Generated from the artefact rather than a rebuild — an SBOM describing different bytes than the ones that shipped is worse than none, because it will be believed.
- **`.trivyignore.yaml`** is added, empty, with its rules written down before the first entry is wanted: only the blocking scan reads it, every entry carries `expired_at`, and every entry must argue why the vulnerable code is unreachable in huginn's deployment.
- **Probe errors are escaped before they leave the process.** The SMTP/IMAP probes copy a remote banner into `ProbeResult.error` verbatim, and that string reached the operator's console, InfluxDB and every HTTP consumer with control bytes intact — a monitored host could emit ANSI/OSC sequences to recolour the terminal, set its title, or use CR to overwrite (forge or hide) the line just written, and the payload persisted in InfluxDB. `ProbeResult::failure` now escapes the whole Unicode Cc range (`\n`, `\r`, `\t` by name, the rest as `\xHH`).
- **An empty InfluxDB token file now stops startup.** It used to be accepted as an empty token: the process started, InfluxDB answered 401, and the writer classified that 4xx as permanent and discarded every batch — a monitor that looked healthy while losing all of its data. Matches the fail-closed rule `metrics.api_key_file` already followed.
- **Security response headers on both HTTP listeners** — a strict `Content-Security-Policy` (no `unsafe-inline`), `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store`. No new dependency.
- **Hardened `docker-compose.yml`** — `read_only: true`, `cap_drop: ALL`, `no-new-privileges`, `mem_limit: 256m`, `pids_limit: 128`, and both published ports moved to `127.0.0.1` (the debug UI is unauthenticated and InfluxDB holds every measurement).
- **`deny.toml` now enforces the rustls-only policy** — `openssl`, `openssl-sys`, `native-tls` and `tokio-native-tls` are banned outright, git sources are denied rather than warned about, and the unused `OpenSSL` license allow-entry is gone.
- **`docs/security-audit.md`** — the full audit report behind these changes: findings with reproduction and impact, what was checked and found not exploitable, and the risks that stay open by decision.
- **Closed a shell-injection path from `CHANGELOG.md` into the `publish` job.** The version was extracted with `sed` and then interpolated as `${{ }}` straight into `run:` blocks, in the one job holding `contents: write`, `packages: write` and the DockerHub credentials — a crafted `## [...]` heading merged to `dev` reached a shell. Extraction and SemVer validation now live in `scripts/changelog-version.sh`, shared with the version gate, and every consumer takes the value through `env:`. The gate alone did not cover this: it is a deliberate no-op outside a release context, while `publish` runs on every push.
- **Every GitHub Action is pinned to a full commit SHA**, and the Semgrep container to a digest. Tags and branches are movable, so a compromised upstream reached CI without a Dependabot PR — including the actions that consume the registry credentials. `dtolnay/rust-toolchain` moved from the `@stable`/`@master` branches to the `v1` SHA with an explicit `toolchain:` input; the toolchain channel itself still floats.
- **A `v*` tag can no longer publish from a commit that is not on `main`.** Tags are not covered by the branch ruleset and the version gate is a no-op on a tag push, so a hand-pushed tag would have published an image and cut a release. `ci.yml` `publish` and `release.yml` now verify the tagged commit is an ancestor of `main`.
- **Dependabot now waits 3 days before proposing a new version** (`cooldown`), so a freshly published malicious release is not auto-merged within the hour. Security updates are exempt by design and are never delayed.

## [0.1.0] - 2026-03-25

### Added
- Cargo workspace with 3 library crates (`huginn-core`, `huginn-probes`, `huginn-influx`) and binary `huginn`
- **6 probe types**: TCP, HTTP, HTTPS, SMTP (banner check), IMAP (greeting check), UDP (DNS payload)
- **InfluxDB 2.x writer** using native line protocol via `reqwest` + `rustls`; token read from file, never from ENV
- **YAML configuration** (`config/config.example.yaml`) with full ENV override support (`HUGINN_*`, `INFLUX_TOKEN_FILE`)
- **Pretty colored CLI output** (default) and JSON mode via `--output json` or `HUGINN_LOG_FORMAT=json`
- **Axum debug web UI** (optional, `--ui` / `HUGINN_UI_ENABLED=true`): `/`, `/health`, `/metrics/latest`
- **Graceful shutdown** via CTRL+C with broadcast channel across all probe loops
- **Docker multi-stage build**: `rust:slim` builder → `distroless/cc-debian12` runtime, runs as `nonroot`
- **Docker Compose** with Docker Secrets for InfluxDB token (never in ENV)
- **GitHub Actions CI** (`ci.yml`): fmt, clippy, test matrix (stable+beta), cargo-audit
- **GitHub Actions Docker** (`docker.yml`): Trivy scan, push `:dev` on dev-merge, push `:latest`+`:vX.Y.Z` on main-merge (version from CHANGELOG)
- **55 tests** (unit + integration, TDD-style)
- Documentation: `README.md`, `docs/getting-started.md`, `docs/configuration.md`, `docs/influxdb.md`, `docs/security.md`, `docs/troubleshooting.md`
- `CONTRIBUTING.md` with branching workflow, PR process, Conventional Commits, release process

[Unreleased]: https://github.com/joshua-schnabel/huginn.io/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/joshua-schnabel/huginn.io/releases/tag/v0.3.0
[0.2.1]: https://github.com/joshua-schnabel/huginn.io/releases/tag/v0.2.1
[0.2.0]: https://github.com/joshua-schnabel/huginn.io/releases/tag/v0.2.0
[0.1.0]: https://github.com/joshua-schnabel/huginn.io/commits/main
<!-- 0.1.0 predates the release pipeline and was never tagged; from the first
     tagged release on, the automation maintains real compare links here. -->
