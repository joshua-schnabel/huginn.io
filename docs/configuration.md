# Configuration Reference

## YAML Structure

```yaml
influx:          # InfluxDB connection (required)
ui:              # Debug web UI (optional)
log:             # Logging settings (optional)
probes:          # List of probes (required)
```

## `influx` section

| Key | Type | Required | Default | Description |
|---|---|---|---|---|
| `url` | string | ✅ | — | InfluxDB base URL |
| `org` | string | ✅ | — | InfluxDB organisation |
| `bucket` | string | ✅ | — | Target bucket |
| `token_file` | string | ✅ | — | Path to file containing the token |
| `batch_size` | int | — | `10` | Write when this many points are buffered |
| `batch_timeout_ms` | int | — | `1000` | Write after this many ms even if batch is not full |

> ⚠️ **Never** set the token directly in YAML or as an ENV variable.
> Use `token_file` pointing to a Docker secret or a protected file.

## `ui` section

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Enable debug web UI |
| `port` | int | `9116` | Listening port |

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
| `type` | enum | ✅ | — | `tcp`, `http`, `https`, `smtp`, `imap`, `udp`, `dns` |
| `target` | string | ✅ | — | `host:port` or URL (DNS: nameserver `IP:port`) |
| `interval_secs` | int | — | `30` | Probe interval in seconds |
| `timeout_secs` | int | — | `5` | Connection/read timeout |
| `expected_status` | int | — | `200` | HTTP/HTTPS only: expected status code |
| `dns_query` | string | — | `example.com` | DNS only: hostname to resolve |
| `dns_expected_ip` | string | — | — | DNS only: if set, probe fails when resolved IP doesn't match |

### DNS probe example

```yaml
- name: my-dns
  type: dns
  target: "8.8.8.8:53"        # nameserver IP:port
  dns_query: "example.com"
  dns_expected_ip: "93.184.216.34"   # optional IP validation
  interval_secs: 60
```

## App-level settings

| Key | Type | Default | Description |
|---|---|---|---|
| `event_hub_capacity` | int | `256` | Broadcast channel capacity for internal probe events |

## ENV Overrides

All values can be overridden without editing the YAML file:

| Variable | Overrides |
|---|---|
| `HUGINN_CONFIG` | Config file path |
| `HUGINN_LOG_FORMAT` | `log.format` |
| `HUGINN_LOG_LEVEL` | `log.level` |
| `HUGINN_UI_ENABLED` | `ui.enabled` |
| `HUGINN_UI_PORT` | `ui.port` |
| `INFLUX_URL` | `influx.url` |
| `INFLUX_ORG` | `influx.org` |
| `INFLUX_BUCKET` | `influx.bucket` |
| `INFLUX_TOKEN_FILE` | `influx.token_file` |

Priority: **ENV > YAML > built-in defaults**
