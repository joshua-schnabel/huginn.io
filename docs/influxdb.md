# InfluxDB Setup

huginn writes raw [line protocol](https://docs.influxdata.com/influxdb/v2/reference/syntax/line-protocol/)
to the InfluxDB 2.x write API — `POST /api/v2/write?org=…&bucket=…&precision=ms`
— with millisecond timestamps and no SDK. The org, bucket and token come from
the `influx` config section (see [`configuration.md`](configuration.md)); the
bucket and token must already exist, huginn creates nothing. A minimal setup:

```bash
influx bucket create --name monitoring --org myorg
influx auth create --org myorg --write-bucket <bucket-id> --description huginn
```

Scope the token to **write-only on that one bucket** — huginn never reads.

## Data Model

**Measurement:** `probe_result`

### Tags (indexed, filterable)

| Tag | Example |
|---|---|
| `probe_name` | `web-homepage` |
| `probe_type` | `tcp`, `http`, `https`, `smtp`, `imap`, `udp`, `dns`, `tls` — the configured `type`, so `http` and `https` stay distinct series |
| `target` | `https://example.com` |

### Fields

| Field | Type | Description |
|---|---|---|
| `up` | integer (0/1) | 1 = reachable, 0 = down |
| `response_ms` | float | Response time in milliseconds |
| `status_code` | integer | HTTP status code (HTTP probes only) |
| `error` | string | Error message when down |
| _metrics_ | float | Per-probe-type extra readings — each `ProbeResult.metrics` entry becomes its own field. Currently: `tls_cert_expiry_days` (TLS probe; negative once expired, present on DOWN results too). InfluxDB is schemaless, so future keys need no migration. |

## Example Flux Queries

**Uptime % over last 24h:**
```flux
from(bucket: "monitoring")
  |> range(start: -24h)
  |> filter(fn: (r) => r._measurement == "probe_result" and r._field == "up")
  |> mean()
  |> map(fn: (r) => ({r with _value: r._value * 100.0}))
```

**Average response time per probe:**
```flux
from(bucket: "monitoring")
  |> range(start: -1h)
  |> filter(fn: (r) => r._measurement == "probe_result" and r._field == "response_ms")
  |> group(columns: ["probe_name"])
  |> mean()
```

**Certificates expiring within 30 days (for alerting):**
```flux
from(bucket: "monitoring")
  |> range(start: -1h)
  |> filter(fn: (r) => r._measurement == "probe_result" and r._field == "tls_cert_expiry_days")
  |> last()
  |> filter(fn: (r) => r._value < 30.0)
```

## Grafana Dashboard

Import a dashboard using the queries above. Recommended panels:
- **Stat panel** — Current `up` value per probe (green/red)
- **Time series** — `response_ms` over time per probe
- **Table** — Latest results with error column
