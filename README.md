<div align="center">

<img src="docs/logo.png" alt="Huginn — a low-poly raven" width="200">

# huginn.io

**Uptime and latency monitoring that fits in one YAML file.**

[![CI](https://img.shields.io/github/actions/workflow/status/joshua-schnabel/huginn.io/ci.yml?branch=dev&label=CI&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/actions/workflows/ci.yml)
[![Security](https://img.shields.io/github/actions/workflow/status/joshua-schnabel/huginn.io/security.yml?branch=dev&label=security&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/actions/workflows/security.yml)
[![Coverage](https://img.shields.io/github/actions/workflow/status/joshua-schnabel/huginn.io/ci.yml?branch=dev&label=coverage%20%E2%89%A5%2080%25&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/joshua-schnabel/huginn.io?logo=github&logoColor=white)](LICENSE)
[![Issues](https://img.shields.io/github/issues/joshua-schnabel/huginn.io?logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/issues)
[![Last commit](https://img.shields.io/github/last-commit/joshua-schnabel/huginn.io/dev?label=last%20change&logo=github&logoColor=white)](https://github.com/joshua-schnabel/huginn.io/commits/dev)  
[![Docker image version](https://img.shields.io/docker/v/jschnabel/huginn?sort=semver&label=image&color=yellow&logo=docker&logoColor=white)](https://hub.docker.com/r/jschnabel/huginn/tags)
[![Docker image size](https://img.shields.io/docker/image-size/jschnabel/huginn?sort=semver&logo=docker&logoColor=white)](https://hub.docker.com/r/jschnabel/huginn/tags)
[![Docker pulls](https://img.shields.io/docker/pulls/jschnabel/huginn?logo=docker&logoColor=white)](https://hub.docker.com/r/jschnabel/huginn)

</div>

> *Huginn* (Old Norse: *Thought*) is one of Odin's two ravens. He flies out each
> day, sees everything, and reports back. **huginn.io** does the seeing for your
> infrastructure; its sibling [muninn.io](https://github.com/joshua-schnabel/muninn.io)
> does the remembering.

You write this:

```yaml
influx:
  url: "http://influxdb:8086"
  org: "myorg"
  bucket: "monitoring"
  token_file: "/run/secrets/influx_token"

probes:
  - name: "web-homepage"
    type: https
    target: "https://example.com"
    interval_secs: 30
    timeout_secs: 5
    expected_status: 200

  - name: "cert-expiry"
    type: tls
    target: "example.com:443"
    interval_secs: 3600
    timeout_secs: 10
```

huginn runs each probe on its own schedule, measures up/down and response time,
and writes the results to InfluxDB as batched line protocol. Every probe type
adds what only it can know — an HTTP status code, the days left on a
certificate — as extra fields on the same result.

The ways uptime monitoring usually goes wrong are quiet ones: a monitor that
dies from the thing it is monitoring; a backend outage that discards the
measurements taken during it; a certificate probe that reports DOWN when the
certificate expires and gives you no number to alert on beforehand; a token
sitting in an environment variable that `docker inspect` will print to anyone.
huginn is built around those four.

## Quick start

**1. Write `config.yaml`.** Copy the annotated
[`config/config.example.yaml`](config/config.example.yaml) and delete what you do
not need.

**2. Run it.** The token is a file, never an environment variable:

```bash
docker run -d --name huginn \
  -v ./config.yaml:/etc/huginn/config.yaml:ro \
  -v ./influx_token.txt:/run/secrets/influx_token:ro \
  jschnabel/huginn:latest
```

Published multi-arch (`linux/amd64`, `linux/arm64`) to Docker Hub and mirrored
byte-identically to `ghcr.io/joshua-schnabel/huginn`. Pin a version rather than
the moving `dev` tag, which carries pre-release builds from the `dev` branch.

**3. Check it.** Without a mounted config the image falls back to the baked-in
example, which probes `example.com` placeholders — fine for a smoke test, not
for real use.

Or build the whole stack from source, InfluxDB included:

```bash
mkdir -p secrets
echo "mytoken"    > secrets/influx_token.txt
echo "mypassword" > secrets/influx_admin_password.txt
chmod 600 secrets/*.txt

cp config/config.example.yaml config/config.yaml
docker compose up -d
```

## Two optional listeners

Both are **off by default** and gated independently, so enabling one never
enables the other. Both bind `127.0.0.1` by default.

| Port | Serves | Enabled by |
|---|---|---|
| `9116` | **Debug UI** — live dashboard over SSE, plus `/health`, `/metrics/latest`, `/events` | `ui.enabled` / `HUGINN_UI_ENABLED` |
| `9464` | **Prometheus** — `/metrics` in text format, optional `Authorization: Bearer` | `metrics.enabled` / `HUGINN_METRICS_ENABLED` |

**In a container the loopback default is wrong on purpose.** A published port
reaches the container's bridge IP, not its loopback, so exposing either listener
takes two deliberate settings — `enabled` *and* `bind: "0.0.0.0"`. This is the
single most common "why can't I reach it" question, and the answer is that it
should be hard to do by accident: the debug UI is unauthenticated and publishes
every probe target. [ADR-0007](docs/adr/0007-debug-ui-has-no-cli-flag.md)

A Prometheus scrape config, with and without the API key, is in
[`docs/configuration.md`](docs/configuration.md).

## What you get

| | |
|---|---|
| **Probes** | `tcp` · `http` · `https` · `smtp` · `imap` · `udp` · `dns` · `tls` cert expiry |
| **Backend** | InfluxDB 2.x — batched line protocol, retried on failure, bounded in memory |
| **Config** | One YAML file plus `HUGINN_*` overrides. Precedence: CLI > ENV > YAML > default |
| **Secrets** | File paths only — never inline, never from the environment, fail-closed |
| **Output** | Coloured console, or structured JSON with `--output json` |
| **Listeners** | Optional debug UI and Prometheus endpoint, gated separately |
| **Container** | Distroless · nonroot · read-only root filesystem · all capabilities dropped |
| **Gates** | Semgrep · Trivy · cargo-deny · shellcheck · actionlint · ≥ 80 % coverage |

**A probe never panics.** A failure becomes a result that says it failed, and is
published like any other. A monitor that dies from what it is monitoring has
told you nothing, and stops telling you anything about everything else.

## Security

huginn holds a database token and ships a container. Both are stated plainly
rather than softened:

- **Every secret is a file path.** No key anywhere takes a token inline, and no
  `HUGINN_*` variable carries one. An environment variable is readable from
  `/proc`, copied into every child, and printed by `docker inspect`. A missing,
  unreadable *or empty* file stops startup — an empty token used to be accepted,
  and produced a monitor that looked healthy while InfluxDB rejected every
  write. [ADR-0002](docs/adr/0002-secrets-from-files-only.md)
- **The debug UI is unauthenticated**, and `metrics.api_key_file` protects only
  the Prometheus listener — the UI serves the same probe inventory without a
  key. Off by default, loopback by default, and published on `127.0.0.1` by the
  shipped compose file. [R2](docs/risks.md)
- **The TLS probe deliberately accepts invalid certificates**, because reading an
  expired certificate is the entire point. One narrowly scoped client, no
  credentials on the connection, verification on everywhere else.
  [ADR-0006](docs/adr/0006-tls-probe-skips-verification.md)
- **TLS is rustls only.** `openssl`, `openssl-sys`, `native-tls` and
  `tokio-native-tls` are banned in `deny.toml`, so the policy is a gate rather
  than an intention. [ADR-0003](docs/adr/0003-rustls-only.md)

Full posture in [`docs/hardening.md`](docs/hardening.md); the 2026-08-02 audit,
with findings, reproductions and accepted risks, in
[`docs/security-audit.md`](docs/security-audit.md). To report a vulnerability:
[`docs/SECURITY.md`](docs/SECURITY.md).

## Known limitations

- **No alerting.** huginn measures and writes; deciding what is worth waking
  someone for belongs where the rules and the silencing already live.
- **No configuration reload.** Change the YAML, restart the process.
- **The retry queue is in memory.** A backend outage that outlives the process
  loses what was buffered, and a long one drops the oldest batches by design.
  [ADR-0004](docs/adr/0004-bounded-retry-queue.md)
- **The TLS probe only covers HTTPS ports.** IMAPS, SMTPS and other raw TLS
  ports are out of scope. [R3](docs/risks.md)

## Development

```bash
cargo t-all         # every test in the workspace
cargo lint          # clippy --all-targets --all-features -- -D warnings
cargo fmt-check     # formatting, as CI checks it
cargo audit-all     # cargo-deny: advisories, licences, bans, sources
cargo cov-ci        # coverage gate, >= 80 % workspace lines
```

The system suite needs the stack:

```bash
docker compose -f docker-compose.integration.yml up -d --build
bash scripts/integration-test.sh
```

Run locally against the example config with `cargo dev`. The debug UI has no CLI
flag — `HUGINN_UI_ENABLED=true cargo dev`. The MSRV is `rust-version` in
[`Cargo.toml`](Cargo.toml); the Docker builder sits at or above it and is the
real MSRV gate, because CI runs floating stable.

Start at [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md); if you are an AI coding
agent, read [`AGENTS.md`](AGENTS.md) first.

## Documentation

| | |
|---|---|
| [Architecture](docs/architecture.md) | Crates, the event bus, startup and shutdown |
| [Configuration](docs/configuration.md) | Every key: type, default, effect |
| [InfluxDB](docs/influxdb.md) | Setup and the measurement/field schema |
| [Troubleshooting](docs/troubleshooting.md) | Symptom, cause, fix |
| [Hardening](docs/hardening.md) | Container and secret posture |
| [Security audit](docs/security-audit.md) | The 2026-08-02 findings and what stays open |
| [Testing](docs/testing.md) | Test pyramid, coverage, the no-sleep rule |
| [CI/CD](docs/ci-cd.md) | Pipeline, release path, repository setup |
| [Workflows](docs/workflows.md) | Every workflow: triggers, jobs, gotchas |
| [Releasing](docs/releasing.md) | Cutting a release, one-click or by hand |
| [Versioning](docs/versioning.md) | SemVer policy and the stable surface |
| [Roadmap](docs/roadmap.md) | What is still open |
| [Risks](docs/risks.md) | Open risks and questions |
| [Decisions](docs/adr/) | Eight ADRs |

`CONTRIBUTING.md` and `SECURITY.md` keep GitHub's ALL-CAPS names so GitHub
surfaces them; every other guide is lowercase.

## Related

[muninn.io](https://github.com/joshua-schnabel/muninn.io) — the sibling project,
uniform server monitoring built on Telegraf, by the same maintainer. It was
built on huginn's conventions and the two are kept aligned deliberately: same
README shape, same doc map, same pipeline, same rules for AI agents.

## License

MIT. See [LICENSE](LICENSE).
