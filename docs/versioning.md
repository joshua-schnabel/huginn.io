# Versioning and stability

huginn.io follows [Semantic Versioning 2.0.0](https://semver.org). This page
defines what that promise actually covers — a version number is only meaningful
if you know which surfaces it protects.

## The stable surface

| Surface | What is covered |
|---|---|
| **Config schema** | Every YAML key and `HUGINN_*`/`INFLUX_*` ENV override in [`configuration.md`](configuration.md), including defaults and validation rules. A config that loads today loads on every later 1.x. |
| **CLI** | The `huginn` binary's flags (`--config`, `--output`) and their semantics. |
| **InfluxDB schema** | The `probe_result` measurement: tag names, field names/types, timestamp precision (`ms`) as documented in [`influxdb.md`](influxdb.md). New *optional* fields (e.g. a new probe's metric) may appear in a minor release — InfluxDB is schemaless, so additions break nothing. |
| **Container contract** | Image config path `/etc/huginn/config.yaml`, token path `/run/secrets/influx_token`, nonroot runtime, port `9116`. |
| **Probe semantics** | What UP/DOWN means per probe type, as documented in `configuration.md`. A change that flips existing results (like a stricter default) is breaking. |
| **Prometheus metrics** | The `/metrics` endpoint's metric names, label names (`probe`, `type`, `target`) and units as documented in `configuration.md`. New metric families may appear in a minor release; renaming or removing one is breaking. |

## Explicitly unstable

- **The debug web UI** — its HTML/JS/CSS, and the exact JSON shape of
  `/metrics/latest` and `/events`. It is a debug tool, not an API. (`/health`
  returning `200 OK` when alive *is* stable — it's made for probes and
  orchestrators.)
- **The Rust crate APIs** (`huginn-core`, `huginn-probes`, `huginn-influx`,
  `huginn-web`) — the workspace crates are internal structure, not a published
  library.
- **Log output** — messages, fields and formatting of the pretty/JSON logs.

## MSRV

The minimum supported Rust version is `rust-version` in `Cargo.toml` — read it
there rather than from a number repeated here, which is how the two drift. It
may be raised in a **minor** release, never in a patch release.

The Dockerfile's builder sits at or above that floor and is what actually
enforces it, because CI runs floating stable. Dependabot moves the builder tag
forward on its own; that is an update, not an MSRV change. Raising the floor is
a deliberate act and gets a changelog entry.

## Supported versions

Only the latest release receives fixes — see [`SECURITY.md`](SECURITY.md).

## Upgrading

Between released `0.x` versions the changes that need attention are listed in
[`CHANGELOG.md`](../CHANGELOG.md), release by release. This section covers only
the two behaviour changes that pre-date `0.2.0` and are therefore easy to miss
when coming from the very first version.

### From 0.1.0

Two behaviour changes need attention; everything else is additive:

1. **The debug UI binds `127.0.0.1` instead of `0.0.0.0`.** If you enable the
   UI in a container, you must now set `ui.bind: "0.0.0.0"` (or
   `HUGINN_UI_BIND=0.0.0.0`) — a published port never reaches the container's
   loopback. The shipped `docker-compose.yml` already does this.
2. **HTTP/HTTPS probes no longer follow redirects.** A 301/302 is reported with
   its own status code, so `expected_status: 200` against a redirecting URL is
   now DOWN. Point the probe at the redirect target, or set `expected_status`
   to the redirect code if the redirect itself is what you monitor.

The full change list lives in the [CHANGELOG](../CHANGELOG.md).

## Related

- [`CHANGELOG.md`](../CHANGELOG.md) — what changed in each release
- [`releasing.md`](releasing.md) — how a version number becomes a release
- [`roadmap.md`](roadmap.md) — what might change next
