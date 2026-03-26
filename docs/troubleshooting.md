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
- Check network connectivity from the container: `docker compose exec hugin-dev /bin/sh` (not available with distroless — use `docker run --rm --network ... alpine ping <host>`)
- Check firewall rules on the target host
- Increase `timeout_secs` for slow targets

## Debug UI not showing
- Ensure `ui.enabled: true` in config (or `HUGIN_UI_ENABLED=true`)
- Port 9116 must be published: `ports: ["9116:9116"]` in compose

## Enable verbose logging
```bash
RUST_LOG=debug hugin-dev --config config.yaml
# or in compose:
environment:
  HUGIN_LOG_LEVEL: debug
```
