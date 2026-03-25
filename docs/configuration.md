# Configuration Reference

## YAML Structure

```yaml
influx:          # InfluxDB connection (required)
ui:              # Debug web UI (optional)
log:             # Logging settings (optional)
probes:          # List of probes (required)
```

## `influx` section

| Key | Type | Required | Description |
|---|---|---|---|
| `url` | string | ✅ | InfluxDB base URL |
| `org` | string | ✅ | InfluxDB organisation |
| `bucket` | string | ✅ | Target bucket |
| `token_file` | string | ✅ | Path to file containing the token |

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
| `type` | enum | ✅ | — | `tcp`, `http`, `https`, `smtp`, `imap`, `udp` |
| `target` | string | ✅ | — | `host:port` or URL |
| `interval_secs` | int | — | `30` | Probe interval in seconds (must be > 0) |
| `timeout_secs` | int | — | `5` | Connection/read timeout (must be > 0) |
| `expected_status` | int | — | `200` | HTTP only: expected status code |

## ENV Overrides

All values can be overridden without editing the YAML file:

| Variable | Overrides |
|---|---|
| `HUGIN_CONFIG` | Config file path |
| `HUGIN_LOG_FORMAT` | `log.format` |
| `HUGIN_LOG_LEVEL` | `log.level` |
| `HUGIN_UI_ENABLED` | `ui.enabled` |
| `HUGIN_UI_PORT` | `ui.port` |
| `INFLUX_URL` | `influx.url` |
| `INFLUX_ORG` | `influx.org` |
| `INFLUX_BUCKET` | `influx.bucket` |
| `INFLUX_TOKEN_FILE` | `influx.token_file` |

Priority: **ENV > YAML > built-in defaults**
