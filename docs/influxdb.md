# InfluxDB Setup

## Data Model

**Measurement:** `probe_result`

### Tags (indexed, filterable)

| Tag | Example |
|---|---|
| `probe_name` | `web-homepage` |
| `probe_type` | `http`, `tcp`, `smtp`, … |
| `target` | `https://example.com` |

### Fields

| Field | Type | Description |
|---|---|---|
| `up` | integer (0/1) | 1 = reachable, 0 = down |
| `response_ms` | float | Response time in milliseconds |
| `status_code` | integer | HTTP status code (HTTP probes only) |
| `error` | string | Error message when down |
| _metrics_ | float | Per-probe-type extra readings, if any — each `ProbeResult.metrics` entry becomes its own field (e.g. `tls_cert_expiry_days`, `packet_loss_pct`). None emitted yet. |

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

## Grafana Dashboard

Import a dashboard using the queries above. Recommended panels:
- **Stat panel** — Current `up` value per probe (green/red)
- **Time series** — `response_ms` over time per probe
- **Table** — Latest results with error column
