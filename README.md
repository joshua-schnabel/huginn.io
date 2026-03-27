# hugin.dev

![Huginn — a low-poly raven](docs/logo.png)

> *Huginn* (Old Norse: *Thought*) is one of Odin's ravens. Every day he flies across the world, observes everything, and reports back to Odin. **hugin.dev** does the same for your infrastructure.

Lightweight uptime & latency monitor. Configures via YAML, writes to InfluxDB, ships as a distroless Docker image.

[![CI](https://github.com/OWNER/hugin-dev/actions/workflows/ci.yml/badge.svg)](https://github.com/OWNER/hugin-dev/actions/workflows/ci.yml)
[![SAST](https://github.com/OWNER/hugin-dev/actions/workflows/sast.yml/badge.svg)](https://github.com/OWNER/hugin-dev/actions/workflows/sast.yml)

## Features

| | |
|---|---|
| **Protocols** | TCP · HTTP · HTTPS · SMTP · IMAP · UDP · DNS |
| **Backend** | InfluxDB 2.x — batch line protocol, `rustls`, token via file |
| **Config** | YAML + ENV overrides |
| **Output** | Coloured CLI or `--output json` |
| **Debug UI** | Live push updates via SSE at `:9116` (optional) |
| **Security** | Distroless · nonroot · Semgrep SAST · Trivy CVE scan · cargo-deny |
| **CI/CD** | GitHub Actions + GitLab CI · DockerHub |

## Quickstart

```bash
mkdir -p secrets
echo "mytoken"    > secrets/influx_token.txt
echo "mypassword" > secrets/influx_admin_password.txt
chmod 600 secrets/*.txt

cp config/config.example.yaml config/config.yaml
docker compose up -d
open http://localhost:9116
```

## Development

```bash
cargo test --workspace                             # all tests
cargo llvm-cov --workspace --open                  # coverage HTML report
cargo fmt --all && cargo clippy --all-targets -- -D warnings
cargo deny check                                   # supply-chain audit
cargo build --release --locked                     # production binary
```

## Documentation

- [Configuration Reference](docs/configuration.md)
- [Testing Guide](docs/testing.md)
- [Security](docs/security.md)
- [CI/CD](docs/ci-cd.md)
- [Troubleshooting](docs/troubleshooting.md)

## License

MIT
