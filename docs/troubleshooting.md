# Troubleshooting

## "Cannot read config file"
- Check the path: `--config /path/to/config.yaml`
- In Docker: ensure the volume mount is correct and the file exists inside the container

## "Secret file error at '/run/secrets/influx_token'"
- The token file does not exist or is not readable
- In Docker Compose: verify the `secrets:` section and that `./secrets/influx_token.txt` exists on the host
- Permissions: `chmod 600 secrets/influx_token.txt`

## "InfluxDB write error: HTTP 401"
- The token in the secret file is wrong or expired
- Regenerate in InfluxDB UI → *Load Data → API Tokens*

## "InfluxDB write error: HTTP 404"
- Wrong `org` or `bucket` name in config/ENV

## Other InfluxDB write errors
- **429 / 408 / 5xx and network errors are retried** with exponential backoff
  (`Retry-After` is honoured) — a burst of these during an InfluxDB restart is
  normal and loses nothing while the retry buffer has room.
- **400 / 403 / 413 / 422 drop the batch immediately** — InfluxDB will never
  accept it (malformed data, forbidden token, oversized body), so retrying would
  block every good batch behind it. The drop is logged at error level.
- `single InfluxDB batch exceeds max_buffered_bytes`: lower `influx.batch_size`
  or raise `max_buffered_bytes`.

## huginn refuses to start with a config error
Validation runs at startup and names the offending key — e.g. duplicate probe
names, an empty `name`/`target`, `batch_timeout_ms: 0`, a hostname instead of an
IP in `ui.bind`, or a negative `tls_expiry_fail_days`. The message says what to
fix; see the validation rules in [`configuration.md`](configuration.md).

## Probes always DOWN
- Check network connectivity from the container: `docker compose exec huginn /bin/sh` (not available with distroless — use `docker run --rm --network ... alpine ping <host>`)
- Check firewall rules on the target host
- Increase `timeout_secs` for slow targets
- `smtp`/`imap` against port 465/993: those ports speak TLS immediately, but the
  probes expect a plaintext greeting — probe `:25`/`:143` instead
  (see the protocol limits in [`configuration.md`](configuration.md))

## TLS probe reports DOWN
- `certificate expired N days ago` / `certificate expires in N days`: the
  certificate is past (or inside) your `tls_expiry_fail_days` window — that is
  the alert working, not a probe failure. The `tls_cert_expiry_days` metric is
  still written alongside the DOWN result.
- `TLS handshake completed but no peer certificate was exposed`: the endpoint
  did not present a usable certificate to the HTTPS client.
- Connection errors: the target must speak **HTTPS** on the probed port — raw
  TLS services (IMAPS `:993`, SMTPS `:465`, LDAPS) are not supported by this
  probe.

## Debug UI not showing
- Ensure `ui.enabled: true` in config (or `HUGINN_UI_ENABLED=true`)
- Port 9116 must be published: `ports: ["9116:9116"]` in compose
- **In a container, set `ui.bind: "0.0.0.0"` (or `HUGINN_UI_BIND=0.0.0.0`).**
  The default is `127.0.0.1`, and a published port reaches the container's
  bridge IP, not its loopback — so the port looks open on the host but the
  connection is refused or reset. The log line `Web UI listening on
  http://127.0.0.1:9116` inside the container confirms this is the cause.
  `docker-compose.yml` and `config/config.integration.yaml` already set it.

## Enable verbose logging
```bash
RUST_LOG=debug huginn --config config.yaml
# or in compose:
environment:
  HUGINN_LOG_LEVEL: debug
```

## Related

- [`configuration.md`](configuration.md) — every key and its default
- [`architecture.md`](architecture.md) — what the process is doing when it fails
- [`hardening.md`](hardening.md) — the secret-file rules several of these hit
- [`risks.md`](risks.md) — known limits that look like bugs
