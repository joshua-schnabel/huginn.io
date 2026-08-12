# Versioning and stability

huginn.io follows [Semantic Versioning 2.0.0](https://semver.org). This page
defines what that promise actually covers — a version number is only meaningful
if you know which surfaces it protects.

## The stable surface

| Surface | What is covered |
|---|---|
| **Config schema** | Every YAML key and `HUGINN_*`/`INFLUX_*` ENV override in [`configuration.md`](configuration.md), including defaults and validation rules. A config that loads today loads on every later 1.x. |
| **CLI** | The `huginn` binary's flags (`--config`, `--output`), the `healthcheck` subcommand and its exit status, and the fact that running the binary with **no** subcommand starts the monitor. |
| **InfluxDB schema** | The `probe_result` measurement: tag names, field names/types, timestamp precision (`ms`) as documented in [`influxdb.md`](influxdb.md). New *optional* fields (e.g. a new probe's metric) may appear in a minor release — InfluxDB is schemaless, so additions break nothing. |
| **Container contract** | Image config path `/etc/huginn/config.yaml`, token path `/run/secrets/influx_token`, nonroot runtime, ports `9116` (debug UI) and `9464` (Prometheus), and the image's `HEALTHCHECK` reporting liveness without extra configuration. |
| **Probe semantics** | What UP/DOWN means per probe type, as documented in `configuration.md`. A change that flips existing results (like a stricter default) is breaking. |
| **Prometheus metrics** | The `/metrics` endpoint's metric names, label names (`probe`, `type`, `target`) and units as documented in `configuration.md`. New metric families may appear in a minor release; renaming or removing one is breaking. |

## Explicitly unstable

- **The debug web UI** — its HTML/JS/CSS, and the exact JSON shape of
  `/metrics/latest` and `/events`. It is a debug tool, not an API. (`/health`
  returning `200 OK` when alive *is* stable, on the UI listener and on the
  liveness listener alike — it's made for probes and orchestrators.)
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

Every release's changes are listed in [`CHANGELOG.md`](../CHANGELOG.md), release
by release. This section covers only the upgrades that can stop a deployment that
was working the day before, rather than merely change what it reports — the ones
easy to miss precisely because nothing in the old configuration looks wrong.
Newest first.

### From 0.3.0

The stable major version made configuration loading **strict** and added a
listener that is **on by default**. Either can turn a process that started
yesterday into one that refuses to start — deliberately, and always with the
offending key named in the error.

1. **Unknown and inapplicable keys are now fatal.** A misspelled key used to be
   swallowed in silence, which is the worst outcome available: you believe you
   set something, the default applies, and nothing disagrees. `batch_sizes` for
   `batch_size` now stops startup, as does a mistyped section like `metric:` for
   `metrics:` — which would otherwise quietly disable the endpoint it was meant
   to configure. The same applies to a key that is valid but does not belong to
   the probe type it sits on. **Before upgrading, start the new version against
   your existing config once**; if it starts, there is nothing to fix.

2. **The liveness listener is new and enabled by default**, on `127.0.0.1:9115`,
   and it has no `bind` key. Three things follow, all of which surface as a
   failure to start rather than as a silent change:
   - if something else already holds that port, huginn does not start;
   - several instances on one host *outside* containers need a distinct
     `health.port` each (inside containers this never comes up — each has its own
     network namespace);
   - a `ui` or `metrics` listener configured on that same loopback port is now
     rejected while the config loads.

   Set `health.enabled: false` to opt out; the container's `HEALTHCHECK` then has
   nothing to ask, so leave it on where the image is used. See the
   [`health` section](configuration.md#health-section).

3. **Values that used to fail late now fail at startup.** Each was previously
   discoverable only once a probe or a write had already gone wrong, often
   minutes in and usually blamed on the wrong thing:
   - `influx.url` must carry a scheme — without one the first write failed,
     looking like an unreachable server;
   - `expected_status` must be a real HTTP status and `dns_expected_ip` must
     parse as an IP address — neither could ever match otherwise, so the probe
     reported DOWN on every tick while the target was healthy;
   - `influx.retry_initial_backoff_ms` and `influx.retry_max_backoff_ms` must be
     non-zero, and the ceiling may not sit below the first delay;
   - `tls_expiry_fail_days` must be **finite** as well as non-negative. YAML
     accepts `.nan`, and because `NaN < 0.0` is false a NaN threshold passed the
     old sign check and then made every expiry comparison false too — reporting
     UP however close the certificate was to expiring.

4. **An enabled listener that cannot bind stops startup.** The debug UI and the
   Prometheus endpoint used to bind inside their own tasks, so a taken port
   produced one logged error while the daemon carried on without the service you
   had explicitly asked for. A config that "worked" while quietly missing its
   metrics endpoint will now tell you.

5. **Listener collisions are compared as parsed addresses.** A wildcard covers
   every address in its family, so `ui.bind: 0.0.0.0` against a loopback
   `metrics` or `health` listener on the same port is a collision and is rejected
   at load — where it names both listeners, instead of failing at bind time with
   a bare "address in use".

6. **Duplicate probe names are rejected.** They keyed one entry in the UI map and
   shared an InfluxDB series, so the second probe overwrote the first.

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
