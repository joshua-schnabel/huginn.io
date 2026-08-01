# huginn.io

![Huginn — a low-poly raven](docs/logo.png)

> *Huginn* (Old Norse: *Thought*) is one of Odin's ravens. Every day he flies across the world, observes everything, and reports back to Odin. **huginn.io** does the same for your infrastructure.

Lightweight uptime & latency monitor. Configures via YAML, writes to InfluxDB, ships as a distroless Docker image.

[![CI](https://img.shields.io/github/actions/workflow/status/joshua-schnabel/huginn.io/ci.yml?branch=dev&label=CI&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/actions/workflows/ci.yml)
[![Security](https://img.shields.io/github/actions/workflow/status/joshua-schnabel/huginn.io/security.yml?branch=dev&label=security&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/actions/workflows/security.yml)
[![License](https://img.shields.io/github/license/joshua-schnabel/huginn.io?logo=github&logoColor=white)](LICENSE)
[![Issues](https://img.shields.io/github/issues/joshua-schnabel/huginn.io?logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/issues)
[![Last commit](https://img.shields.io/github/last-commit/joshua-schnabel/huginn.io/dev?label=last%20change&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/commits/dev)  
[![Docker image version](https://img.shields.io/docker/v/jschnabel/huginn?sort=semver&label=image&color=yellow&logo=docker&logoColor=white)](https://hub.docker.com/r/jschnabel/huginn/tags)
[![Docker image size](https://img.shields.io/docker/image-size/jschnabel/huginn?sort=semver&logo=docker&logoColor=white)](https://hub.docker.com/r/jschnabel/huginn/tags)
[![Docker pulls](https://img.shields.io/docker/pulls/jschnabel/huginn?logo=docker&logoColor=white)](https://hub.docker.com/r/jschnabel/huginn)

## Features

| | |
|---|---|
| **Probes** | TCP · HTTP · HTTPS · SMTP · IMAP · UDP · DNS |
| **Backend** | InfluxDB 2.x — batch line protocol, `rustls`, token via file |
| **Config** | YAML + ENV overrides |
| **Output** | Coloured CLI or `--output json` |
| **Debug UI** | Live push updates via SSE at `:9116` (optional) |
| **Security** | Distroless · nonroot · [Semgrep SAST](https://semgrep.dev) · Trivy CVE scan · cargo-deny |
| **CI/CD** | GitHub Actions · DockerHub |

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
- [Releasing](docs/releasing.md)
- [Troubleshooting](docs/troubleshooting.md)

## License

MIT
