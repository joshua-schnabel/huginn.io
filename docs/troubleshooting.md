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

## Probes always DOWN
- Check network connectivity from the container: `docker compose exec huginn /bin/sh` (not available with distroless — use `docker run --rm --network ... alpine ping <host>`)
- Check firewall rules on the target host
- Increase `timeout_secs` for slow targets

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
