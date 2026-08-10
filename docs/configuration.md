# Configuration reference

## YAML structure

```yaml
influx:          # InfluxDB connection (required)
ui:              # Debug web UI (optional, off by default)
metrics:         # Prometheus /metrics endpoint (optional, off by default)
health:          # Liveness endpoint (optional, ON by default)
log:             # Logging settings (optional)
probes:          # List of probes (required)
```

## `influx` section

| Key | Type | Required | Default | Description |
|---|---|---|---|---|
| `url` | string | ✅ | — | InfluxDB base URL |
| `org` | string | ✅ | — | InfluxDB organisation |
| `bucket` | string | ✅ | — | Target bucket |
| `token_file` | string | — | `/run/secrets/influx_token` | Path to file containing the token. The default matches the Docker-secret mount path; the file must exist **and be non-empty** at startup, otherwise huginn refuses to start |
| `batch_size` | int | — | `10` | Write when this many points are buffered. Must be > 0 |
| `batch_timeout_ms` | int | — | `1000` | Write after this many ms even if batch is not full |
| `max_buffered_bytes` | int | — | `8388608` | Memory ceiling for batches waiting while InfluxDB is unreachable (8 MiB ≈ 35k–55k results) |
| `retry_initial_backoff_ms` | int | — | `500` | First retry delay after a failed write; doubles per attempt |
| `retry_max_backoff_ms` | int | — | `30000` | Ceiling for the retry backoff |
| `shutdown_drain_timeout_ms` | int | — | `5000` | How long to keep draining buffered batches after a shutdown signal |

> ⚠️ **Never** set the token directly in YAML or as an ENV variable.
> Use `token_file` pointing to a Docker secret or a protected file.

A missing **or empty** token file is a fatal startup error. Empty is treated the
same as missing on purpose: with an empty token huginn would start, send
`Authorization: Token ` with no value, and InfluxDB would answer 401 — a 4xx,
which the writer classifies as permanent and discards. That looks like a healthy
monitor that is throwing away every measurement it takes, so it fails at startup
instead.

### What happens when InfluxDB is down

Results are batched and queued, not written directly. If a write fails, the batch
stays queued and is retried with exponential backoff — it is **not** discarded.

- **Retries are unbounded in attempts, bounded in memory.** Capping attempts
  would mean every batch dies a few seconds into an outage, so the buffer would
  never fill and would protect nothing. The real limit is `max_buffered_bytes`:
  when the queue is full, the **oldest** batch is dropped, keeping the most
  recent window. Evictions are logged.
- **Only transient failures are retried.** Network errors, 5xx, 429 and 408 are
  retried (`Retry-After` is honoured). 400/401/403/404/413/422 mean InfluxDB will
  never accept that batch — a bad token, a wrong bucket, malformed data — so it
  is discarded immediately and logged at error level. Retrying it would block
  every good batch behind it forever.
- **Shutdown drains, but not indefinitely.** After a shutdown signal the writer
  gets `shutdown_drain_timeout_ms` to flush what is queued. If InfluxDB is still
  unreachable when that expires, the buffered results are discarded and the count
  is logged — otherwise unbounded retry would mean a process that never exits.

If you see `single InfluxDB batch exceeds max_buffered_bytes`, lower
`influx.batch_size` or raise `max_buffered_bytes`.

## `ui` section

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Enable debug web UI |
| `bind` | string | `127.0.0.1` | Listening address (IP, not a hostname) |
| `port` | int | `9116` | Listening port |

The UI has **no authentication** and serves every probe target and error string
to anyone who can reach it, so it binds loopback only. Widening that is a
deliberate act:

- **In a container you must set `0.0.0.0`.** A published port (`-p 9116:9116`)
  reaches the container's bridge IP, never its loopback — with the default the
  port is open on the host but nothing answers. The shipped
  `docker-compose.yml` sets `HUGINN_UI_BIND=0.0.0.0` for exactly this reason.
- To publish it but keep it off the network, bind the *host* side instead:
  `ports: ["127.0.0.1:9116:9116"]`.

`bind` must be an IP address (`0.0.0.0`, `127.0.0.1`, `::1`, `::`); a hostname
is rejected at load.

## `metrics` section

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Enable the Prometheus `/metrics` listener |
| `bind` | string | `127.0.0.1` | Listening address (IP, not a hostname) |
| `port` | int | `9464` | Listening port (9464 is the conventional exporter port) |
| `api_key_file` | string | — | Optional path to a file containing an API key. When set, scrapes must send `Authorization: Bearer <key>`. Path only — the key value never goes into YAML or ENV (same policy as `influx.token_file`). A configured-but-missing or empty file stops startup rather than serving unauthenticated |

Serves `GET /metrics` in the Prometheus text format, independently of the debug
UI — either can be enabled without the other (but not both on the same
`bind:port`; that is rejected at load). Exposed gauges, one sample per probe
with labels `probe`, `type`, `target`:

- `huginn_probe_success` — 1/0 for the last run
- `huginn_probe_duration_seconds` — last run duration
- `huginn_probe_http_status_code` — HTTP probes only
- `huginn_probe_last_run_timestamp_seconds`
- `huginn_probe_<key>` for every probe-specific reading, e.g.
  `huginn_probe_tls_cert_expiry_days`

Without `api_key_file` the endpoint is unauthenticated like the UI: bind
loopback unless the network is trusted, and use `0.0.0.0` inside a container.
With a key, Prometheus scrapes it via:

```yaml
scrape_configs:
  - job_name: huginn
    authorization:
      type: Bearer
      credentials_file: /etc/prometheus/huginn_api_key
    static_configs:
      - targets: ["huginn-host:9464"]
```

## `health` section

The liveness listener — **the only one that is on by default.**

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Serve `GET /health` on loopback |
| `port` | int | `9115` | Listening port on `127.0.0.1` |

There is **no `bind` key**, and that is the point. The listener is fixed to
`127.0.0.1` and cannot be widened, which is what makes an on-by-default listener
defensible: Docker runs `HEALTHCHECK` *inside* the container, so loopback is
where the check needs it, while a published port reaches the container's bridge
IP and never this socket. It serves the string `OK` and nothing else — no probe
names, no targets, no errors — so unlike the debug UI it discloses nothing about
what is monitored. [ADR-0008](adr/0008-liveness-listener-on-by-default.md)

It exists so the image can carry a `HEALTHCHECK`. Distroless has no shell and no
`curl`, so the check is the binary itself:

```bash
huginn --config /etc/huginn/config.yaml healthcheck   # exit 0 = alive
```

**Liveness, not readiness.** A 200 means the process is running and its runtime
is still scheduling work. It says nothing about whether probes are succeeding or
InfluxDB is reachable, deliberately: an orchestrator that restarted huginn
because a monitored host went down would remove the monitor exactly when it is
needed. Probe health is what `/metrics` and the probe results are for.

Two things follow from the port being fixed rather than chosen:

- **Several huginns on one host collide.** In containers this never comes up —
  each has its own network namespace. Outside them, or under
  `network_mode: host`, give each instance its own `health.port`.
- **A `ui` or `metrics` listener on the same loopback port is rejected at
  load**, rather than one of the two silently losing the bind at runtime.

With `enabled: false` there is no listener, and `huginn healthcheck` says so
instead of reporting a connection error that reads like a dead process. Drop the
`HEALTHCHECK` from your deployment if you turn it off.

## `log` section

| Key | Type | Default | Description |
|---|---|---|---|
| `format` | `pretty` \| `json` | `pretty` | Console output format |
| `level` | string | `info` | Log level (`trace`, `debug`, `info`, `warn`, `error`) |

## `probes` section

Each probe entry:

| Key | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | string | ✅ | — | Unique probe name (used as InfluxDB tag) |
| `type` | enum | ✅ | — | `tcp`, `http`, `https`, `smtp`, `imap`, `udp`, `dns`, `tls` |
| `target` | string | ✅ | — | `host:port` or URL (DNS: nameserver `IP:port`) |
| `interval_secs` | int | — | `30` | Probe interval in seconds |
| `timeout_secs` | int | — | `5` | Connection/read timeout |
| `expected_status` | int | — | `200` | HTTP/HTTPS only: expected status code |
| `dns_query` | string | — | `example.com` | DNS only: hostname to resolve |
| `dns_expected_ip` | string | — | — | DNS only: if set, probe fails when resolved IP doesn't match |
| `tls_expiry_fail_days` | float | — | `0` | TLS only: report DOWN once the certificate expires in fewer than this many days. `0` (the default) fails only once it has expired. Must be ≥ 0 |

### Validation

The config is checked at startup and huginn refuses to run rather than fail
later in a way that looks like an outage.

**Unknown keys are an error.** A misspelled key used to be ignored in silence,
which is the worst outcome available: you believe you set something, the default
applies, and nothing anywhere disagrees. `batch_sizes` instead of `batch_size`
now fails at load and names the key; so does a mistyped *section*, where
`metric:` for `metrics:` would otherwise have quietly disabled the endpoint it
was meant to configure.

The rest:

- `name` must be **unique** and non-empty. Names key the web UI's map and the
  InfluxDB tag series, so duplicates silently overwrite each other's history.
- `target` must be non-empty and match the probe type: `dns` needs a nameserver
  address with a port (`8.8.8.8:53`, `[2001:4860:4860::8888]:53`);
  `tcp`/`smtp`/`imap`/`udp`/`tls` need a port (`tls`: typically `:443`);
  `http`/`https` need an absolute URL. The URL scheme does **not** have to match
  the probe type — `type: http` with an `https://` target is fine.
- `interval_secs`, `timeout_secs`, `batch_size`, `batch_timeout_ms`,
  `max_buffered_bytes`, `event_hub_capacity`, `retry_initial_backoff_ms` and
  `retry_max_backoff_ms` must be greater than 0, and the retry ceiling must not
  be below the first delay — a maximum under the initial value is not a smaller
  maximum, it is a setting that never applies.
- `influx.url` must be an absolute URL. Without a scheme it parses as a string
  and fails only when the first batch is written, looking like an unreachable
  server.
- `tls_expiry_fail_days` must be ≥ 0 **and finite**. YAML accepts `.nan`, and
  `NaN < 0` is false — so a NaN threshold passed the sign check and then made
  every expiry comparison false too, reporting UP however close the certificate
  was to expiring.
- `expected_status` must be a real HTTP status (100–599), and `dns_expected_ip`
  must parse as an IP address. Neither could ever match otherwise, so the probe
  would report DOWN on every tick while the target was healthy.
- `ui.bind` and `metrics.bind` must be IP addresses, and an enabled listener's
  port must not be `0` — port 0 asks the OS for an arbitrary free port, giving a
  service nothing can be configured to reach.
- `ui` and `metrics` must not both be enabled on the same `bind:port`. The
  comparison is on the **parsed** addresses, so `::1` and `0:0:0:0:0:0:0:1` are
  recognised as one socket.
- An enabled listener that **cannot bind** stops startup. Both are bound before
  the scheduler starts rather than inside their own tasks, where a taken port
  produced one logged line while the daemon ran on without the service. For
  `metrics` that matters twice over: Prometheus reports a scrape target that
  never answers as the *monitored* host being down.

### DNS probe example

```yaml
- name: my-dns
  type: dns
  target: "8.8.8.8:53"        # nameserver IP:port
  dns_query: "example.com"
  dns_expected_ip: "93.184.216.34"   # optional IP validation
  interval_secs: 60
```

### TLS probe example

```yaml
- name: my-cert
  type: tls
  target: "example.com:443"   # any TLS port: 443, 993, 465, 636, …
  interval_secs: 3600         # certificates change slowly — probe rarely
  tls_expiry_fail_days: 14    # DOWN once fewer than 14 days remain
```

The probe completes a TLS handshake — and nothing more; no application-protocol
request is made — then reports the days until the server certificate expires as
the `tls_cert_expiry_days` metric (negative once
expired; kept on DOWN results too, so alerts can see how far gone it is).
Certificate **verification is intentionally skipped** — the point is to read
the certificate, self-signed and expired ones included, not to trust it. See
[`hardening.md`](hardening.md) for the security reasoning.

### What `timeout_secs` and `response_ms` mean

- **`timeout_secs` is the budget for the whole probe**, not for each step inside
  it. An `smtp` or `imap` probe spends it on the connect *and* the greeting
  together; it used to apply once to each, so the real worst case was twice the
  configured value.
- **`response_ms` for `http`/`https` ends at the response headers.** The body is
  never read. Including it would make the number depend on the size of whatever
  the endpoint returns, so a page that grew by a megabyte would be
  indistinguishable from a server getting slower.
- **`smtp` and `imap` read a complete greeting line** (bounded to 512 bytes)
  rather than whatever one `read()` happens to return. TCP is a byte stream and
  may split anywhere, so a valid `220 …` arriving as `22` + the rest used to be
  reported DOWN — occasionally, depending on timing.

### Known protocol limits

- **`smtp` / `imap` expect a plaintext greeting** — probing implicit-TLS ports
  (SMTPS `:465`, IMAPS `:993`) reports DOWN. Probe the plaintext/STARTTLS ports
  (`:25`, `:143`) instead.
- **`dns` resolves A/AAAA over UDP only** — no MX/TXT/etc. record types, no
  TCP/DoT/DoH transport.
- **`udp` sends a DNS-shaped payload** and counts any reply as UP — it is a
  reachability check, best suited to DNS-like services.
- **`tls` works on any TLS port.** The certificate comes from the handshake
  itself, so IMAPS (`:993`), SMTPS (`:465`), LDAPS (`:636`) and HTTPS are all
  probeable. It does *not* speak STARTTLS: a port that begins in plaintext and
  upgrades on command is not a TLS port until the command is sent, so probe the
  implicit-TLS port instead.

## App-level settings

| Key | Type | Default | Description |
|---|---|---|---|
| `event_hub_capacity` | int | `256` | Broadcast channel capacity for internal probe events |

## Environment overrides

A defined subset can be overridden without editing the YAML file — not every
key. Probe entries, `event_hub_capacity`, the whole `health` section and the
`influx` batching and retry keys have no ENV form; the table below is the whole
list:

| Variable | Overrides |
|---|---|
| `HUGINN_CONFIG` | Config file path (default `/etc/huginn/config.yaml`; this is the ENV form of `--config`) |
| `HUGINN_LOG_FORMAT` | `log.format` |
| `HUGINN_LOG_LEVEL` | `log.level` |
| `HUGINN_UI_BIND` | `ui.bind` |
| `HUGINN_UI_ENABLED` | `ui.enabled` |
| `HUGINN_UI_PORT` | `ui.port` |
| `HUGINN_METRICS_BIND` | `metrics.bind` |
| `HUGINN_METRICS_ENABLED` | `metrics.enabled` |
| `HUGINN_METRICS_PORT` | `metrics.port` |
| `HUGINN_METRICS_API_KEY_FILE` | `metrics.api_key_file` (a path — never the key itself) |
| `INFLUX_URL` | `influx.url` |
| `INFLUX_ORG` | `influx.org` |
| `INFLUX_BUCKET` | `influx.bucket` |
| `INFLUX_TOKEN_FILE` | `influx.token_file` |

Priority: **CLI > ENV > YAML > built-in defaults**

**Exception — `RUST_LOG`:** if set, it feeds the tracing filter directly and
wins over *everything*, including `--output`, `HUGINN_LOG_LEVEL` and
`log.level`. It also accepts per-module directives
(`RUST_LOG=huginn_influx=debug,info`), which the plain level keys cannot
express.

`--output pretty|json` overrides `log.format` in **both** directions — including
overriding a config file that says `json` back to `pretty`. (It previously
could not: the check was an OR.)

An ENV variable that is set but unusable is **warned about, not silently
ignored**: `HUGINN_UI_PORT=abc`, `HUGINN_UI_BIND=0.0.0.0.0`,
`HUGINN_LOG_FORMAT=xml` and `HUGINN_UI_ENABLED=yes` each log a warning and leave
the previous value in place. `HUGINN_UI_ENABLED` accepts `true`/`false`/`1`/`0`.
A typo in `HUGINN_UI_BIND` therefore keeps the narrower address rather than
falling back to something wider.

These warnings appear *after* the tracing subscriber starts — the log level to
start it with comes from the very config being read — so they arrive a few lines
below `huginn starting`, not before it.

## Related

- [`architecture.md`](architecture.md) — what these settings actually control
- [`influxdb.md`](influxdb.md) — the schema the `influx` section writes into
- [`hardening.md`](hardening.md) — the secret files and why they are files
- [`troubleshooting.md`](troubleshooting.md) — when a setting does not do what you expect
