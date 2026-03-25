# 🦅 hugin.dec

**Uptime & response-time monitoring** — YAML-configurable, InfluxDB backend, similar to the Prometheus Blackbox Exporter.

## Features

| Feature | Details |
|---|---|
| **Protocols** | TCP · HTTP · HTTPS · SMTP · IMAP · UDP |
| **Backend** | InfluxDB 2.x (line protocol) |
| **Config** | YAML + ENV overrides |
| **Output** | Coloured pretty-print or JSON (`--output json`) |
| **Debug UI** | Minimal web UI at `:9116` (optional) |
| **Security** | Distroless image · nonroot · rustls · no secrets in ENV |
| **CI/CD** | GitHub Actions · cargo audit · trivy · GHCR |

## Quickstart (Docker)

```bash
# 1. Create secrets directory (never commit these!)
mkdir -p secrets
echo "mytoken"    > secrets/influx_token.txt
echo "mypassword" > secrets/influx_admin_password.txt
chmod 600 secrets/*.txt

# 2. Copy and edit config
cp config/config.example.yaml config/config.yaml

# 3. Start
docker compose up -d

# 4. Open debug UI
open http://localhost:9116
```

## CLI Usage

```
hugin-dec [OPTIONS]

Options:
  -c, --config <FILE>   Config file path [env: HUGIN_CONFIG] [default: /etc/hugin/config.yaml]
      --output <FMT>    Output format: pretty | json [env: HUGIN_LOG_FORMAT] [default: pretty]
  -h, --help            Print help
  -V, --version         Print version
```

## Example Output (pretty)

```
[2024-01-15 10:32:01]  ✅  web-homepage             HTTP    42.1ms
[2024-01-15 10:32:01]  ✅  postgres                 TCP     3.2ms
[2024-01-15 10:32:02]  ❌  smtp-mail                SMTP    5000.0ms  timeout after 5s
```

## Documentation

- [Getting Started](docs/getting-started.md)
- [Configuration Reference](docs/configuration.md)
- [Probe Guides](docs/probes/)
- [InfluxDB Setup](docs/influxdb.md)
- [Security](docs/security.md)
- [Troubleshooting](docs/troubleshooting.md)

## Building from Source

```bash
cargo build --release --locked
./target/release/hugin-dec --config config/config.example.yaml
```

## License

MIT
