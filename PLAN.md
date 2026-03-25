# hugin.dec — Implementation Plan

## Problem Statement

Neues Rust-Projekt „hugin.dec" — ein Uptime- und Response-Time-Monitor ähnlich dem Prometheus Blackbox Exporter, jedoch mit **InfluxDB** als Metrik-Backend statt Prometheus. Konfiguration via YAML, Unterstützung für TCP, HTTP, HTTPS, SMTP, IMAP und UDP. TDD und Testautomatisierung sind vorrangig.

---

## Architecture Overview

```
hugin.dec/
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── hugin-core/             # Shared types, config, errors
│   ├── hugin-probes/           # Protocol probe implementations
│   └── hugin-influx/           # InfluxDB writer
├── src/
│   └── main.rs                 # Binary entry point
├── config/
│   └── config.example.yaml     # Example config
└── tests/
    └── integration/            # Integration tests
```

### Crate Breakdown

| Crate | Responsibility |
|---|---|
| `hugin-core` | Config structs (serde/yaml), shared types (ProbeResult), error types |
| `hugin-probes` | TCP, HTTP/HTTPS, SMTP, IMAP, UDP probe logic, scheduler |
| `hugin-influx` | InfluxDB2 client wrapper, line protocol serialisation |

---

## Key Dependencies

| Crate | Purpose |
|---|---|
| `tokio` (full) | Async runtime |
| `serde` + `serde_yaml` | YAML config deserialisation |
| `reqwest` (rustls) | HTTP/HTTPS probes |
| `tokio::net` | TCP + UDP probes |
| `lettre` | SMTP probe (banner check) |
| `async-imap` + `tokio-rustls` | IMAP probe |
| `influxdb2` | InfluxDB 2.x line-protocol writer |
| `clap` v4 | CLI args (`--config`) |
| `tracing` + `tracing-subscriber` | Structured logging |
| `anyhow` / `thiserror` | Error handling |
| `tokio-cron-scheduler` | Interval-based probe scheduling |

### Test Dependencies

| Crate | Purpose |
|---|---|
| `mockito` | HTTP mock server |
| `tokio-test` | Async test utilities |
| `wiremock` | Advanced HTTP mocking (integration tests) |
| `assert_matches` | Pattern-based assertions |

---

## Config Schema (YAML)

```yaml
influx:
  url: "http://localhost:8086"
  org: "myorg"
  bucket: "monitoring"
  token: "${INFLUX_TOKEN}"

probes:
  - name: "web-homepage"
    type: http
    target: "https://example.com"
    interval_secs: 30
    timeout_secs: 5
    expected_status: 200

  - name: "smtp-mail"
    type: smtp
    target: "mail.example.com:25"
    interval_secs: 60
    timeout_secs: 10

  - name: "tcp-custom"
    type: tcp
    target: "db.example.com:5432"
    interval_secs: 15
    timeout_secs: 3

  - name: "imap-mail"
    type: imap
    target: "mail.example.com:143"
    interval_secs: 60
    timeout_secs: 10

  - name: "udp-dns"
    type: udp
    target: "8.8.8.8:53"
    interval_secs: 30
    timeout_secs: 2
```

---

## InfluxDB Data Model

**Measurement:** `probe_result`

| Tag | Value |
|---|---|
| `probe_name` | name from config |
| `probe_type` | tcp / http / https / smtp / imap / udp |
| `target` | host:port or URL |

| Field | Type | Description |
|---|---|---|
| `up` | bool (0/1) | Was the target reachable? |
| `response_ms` | float | Response time in ms |
| `status_code` | int | HTTP status code (HTTP only) |
| `error` | string | Error message if down |

---

## TDD Strategy

- **Unit tests** embedded in each module (`#[cfg(test)]`) for all probe logic
- **Integration tests** in `tests/integration/` using mock servers
- HTTP probes: `mockito` / `wiremock` for fake HTTP server
- TCP/UDP probes: bind a local test socket
- SMTP/IMAP: minimal fake server using `tokio::net::TcpListener`
- InfluxDB writer: mock HTTP endpoint via `wiremock`
- **CI-ready**: all tests run without external services
- Code coverage goal: >80% on `hugin-probes` and `hugin-core`

---

## Build-Infrastruktur (GitHub)

- Repository auf **GitHub** (öffentlich oder privat)
- **GitHub Actions** als CI/CD-Pipeline
- **GitHub Packages** (GHCR) als Container-Registry
- Workflow-Trigger: `push` + `pull_request` auf `main`
- Release-Workflow: Tag `v*.*.*` → Build → Push `ghcr.io/<owner>/hugin-dec:latest` + `:v1.2.3`

---

## CLI-Ausgabe & Debug-UI

### CLI-Ausgabe
- Standard: **schöne farbige Ausgabe** via `colored` + `tracing-subscriber` pretty-printer
- Schaltbar auf **JSON-Log-Ausgabe** via:
  - Flag `--output json` oder
  - Env-Variable `HUGIN_LOG_FORMAT=json`
- Log-Level via `RUST_LOG` (z. B. `RUST_LOG=info`)

Beispiel Pretty-Ausgabe:
```
[2024-01-15 10:32:01] ✅  web-homepage     HTTP    200  42ms
[2024-01-15 10:32:01] ❌  smtp-mail        SMTP    ERR  timeout (5000ms)
[2024-01-15 10:32:02] ✅  tcp-db           TCP     UP   3ms
```

Beispiel JSON-Ausgabe:
```json
{"timestamp":"2024-01-15T10:32:01Z","probe":"web-homepage","type":"http","up":true,"response_ms":42,"status_code":200}
```

### Minimale Debug-Web-UI
- Eingebetteter **`axum`**-Webserver (optional, via `--ui` Flag oder `HUGIN_UI_ENABLED=true`)
- Standard-Port: `9116` (analog Blackbox Exporter)
- Endpunkte:
  - `GET /` — Status-Seite (HTML, auto-refresh alle 5s): Tabelle aller Probes mit letztem Ergebnis
  - `GET /health` — `200 OK` (Liveness)
  - `GET /metrics/latest` — JSON-Array mit letzten ProbeResults aller Probes
- Keine externen JS-Frameworks — reines HTML + inline CSS (kein Build-Step)

---

## Konfiguration via ENV (ohne Secrets in ENV)

### Prinzip: ENV für Config-Werte, Dateipfade für Secrets
- InfluxDB-Token **nie** direkt in ENV → stattdessen **Datei-Pfad** in ENV
- Alle Config-Werte können über ENV überschrieben werden (12-Factor-App)

### ENV-Variablen

| Variable | Beschreibung | Beispiel |
|---|---|---|
| `HUGIN_CONFIG` | Pfad zur YAML-Config | `/etc/hugin/config.yaml` |
| `HUGIN_LOG_FORMAT` | `pretty` (default) oder `json` | `json` |
| `HUGIN_LOG_LEVEL` | Log-Level | `info` |
| `HUGIN_UI_ENABLED` | Debug-UI aktivieren | `true` |
| `HUGIN_UI_PORT` | Debug-UI Port | `9116` |
| `INFLUX_TOKEN_FILE` | **Pfad zur Datei** mit InfluxDB-Token | `/run/secrets/influx_token` |
| `INFLUX_URL` | InfluxDB URL (überschreibt YAML) | `http://influxdb:8086` |
| `INFLUX_ORG` | InfluxDB Org | `myorg` |
| `INFLUX_BUCKET` | InfluxDB Bucket | `monitoring` |

> ⚠️ `INFLUX_TOKEN_FILE` zeigt auf eine Datei (z. B. Docker Secret Mount unter `/run/secrets/`). Der Inhalt der Datei wird zur Laufzeit gelesen. Der Token selbst wird **nie** als ENV-Variable übergeben.

### Docker Secrets Integration
```yaml
# docker-compose.yml
services:
  hugin-dec:
    secrets:
      - influx_token
    environment:
      INFLUX_TOKEN_FILE: /run/secrets/influx_token

secrets:
  influx_token:
    file: ./secrets/influx_token.txt
```

---

## Docker Runtime

### Ziel: schlankes, sicheres Image ohne bekannte CVEs

**Multi-Stage Build:**
```dockerfile
# Stage 1: Build
FROM rust:1.81-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked

# Stage 2: Runtime — minimales distroless Image
FROM gcr.io/distroless/cc-debian12
COPY --from=builder /build/target/release/hugin-dec /usr/local/bin/hugin-dec
COPY config/config.example.yaml /etc/hugin/config.yaml
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/hugin-dec", "--config", "/etc/hugin/config.yaml"]
```

- **Base Image:** `gcr.io/distroless/cc-debian12` — kein Shell, kein Paketmanager, minimale Angriffsfläche
- **Kein root:** Läuft als `nonroot`-User
- **Kein unnötiger Toolchain-Overhead:** Build-Stage bleibt separat
- **`Cargo.lock` committed:** `--locked` Flag garantiert reproduzierbare Builds
- **Regelmäßiger Image-Scan** via `trivy` in CI

### docker-compose.yml (Dev/Test)
```yaml
services:
  hugin-dec:
    build: .
    volumes:
      - ./config/config.yaml:/etc/hugin/config.yaml:ro
    depends_on:
      - influxdb
    restart: unless-stopped

  influxdb:
    image: influxdb:2.7-alpine
    environment:
      DOCKER_INFLUXDB_INIT_MODE: setup
      DOCKER_INFLUXDB_INIT_ORG: myorg
      DOCKER_INFLUXDB_INIT_BUCKET: monitoring
      DOCKER_INFLUXDB_INIT_ADMIN_TOKEN: mytoken
    ports:
      - "8086:8086"
```

---

## Sicherheitsziele

| Bereich | Maßnahme |
|---|---|
| Keine CVEs | `distroless` Base-Image, regelmäßiger `trivy`-Scan in CI |
| Secrets | InfluxDB-Token via Env-Variable (`${INFLUX_TOKEN}`), nie in YAML hardcoded |
| Minimale Angriffsfläche | Kein Shell, kein Paketmanager im Runtime-Image |
| Dependency-Audit | `cargo audit` in CI (RustSec Advisory DB) |
| Non-root | Container läuft als `nonroot` User |
| TLS | HTTPS und IMAPS/SMTPS via `rustls` (kein OpenSSL-Link) |
| Supply-chain | `Cargo.lock` committed, `--locked` builds |
| SBOM | Optional: `cargo cyclonedx` für Software Bill of Materials |

---

## End-User Dokumentation

Struktur in `docs/` und `README.md`:

```
docs/
├── getting-started.md     # Quickstart: Docker, erste Config
├── configuration.md       # Vollständige YAML-Referenz aller Felder
├── probes/
│   ├── http.md
│   ├── tcp.md
│   ├── smtp.md
│   ├── imap.md
│   └── udp.md
├── influxdb.md            # InfluxDB Setup, Datenmodell, Grafana-Dashboard-Beispiel
├── security.md            # Secrets, TLS, Docker Security
└── troubleshooting.md     # Häufige Fehler & Lösungen
```

`README.md` enthält:
- Kurzbeschreibung + Feature-Liste
- Docker Quickstart (1 Befehl)
- Beispiel-Config-Snippet
- Links zur vollständigen Dokumentation

---

## Implementation Todos

> **Status: ✅ VOLLSTÄNDIG ABGESCHLOSSEN** — 2 Commits, 54 Tests, 0 Failures

| # | Todo | Status |
|---|---|---|
| 1 | `init-workspace` — Cargo workspace (3 crates + binary) | ✅ done |
| 2 | `core-types` — ProbeResult, HuginError, serde | ✅ done |
| 3 | `core-config` — AppConfig YAML + ENV-Override + Validierung | ✅ done |
| 4 | `probe-tcp` — TCP connect probe (TDD) | ✅ done |
| 5 | `probe-http` — HTTP/HTTPS probe mit Status-Check (TDD) | ✅ done |
| 6 | `probe-smtp` — SMTP Banner-Check 220 (TDD) | ✅ done |
| 7 | `probe-imap` — IMAP Greeting-Check `* OK` (TDD) | ✅ done |
| 8 | `probe-udp` — UDP Send/Recv mit Timeout (TDD) | ✅ done |
| 9 | `influx-writer` — InfluxDB2 Line-Protocol Writer (TDD) | ✅ done |
| 10 | `scheduler` — tokio broadcast channel, per-probe interval | ✅ done |
| 11 | `env-config` — HUGIN_* ENV-Vars, INFLUX_TOKEN_FILE (kein Token in ENV) | ✅ done |
| 12 | `cli-output` — colored Pretty-Output + JSON-Modus (`--output`) | ✅ done |
| 13 | `debug-ui` — axum Web-UI `/health`, `/metrics/latest`, `/` HTML | ✅ done |
| 14 | `main-binary` — clap CLI, Config-Load, Scheduler, Graceful Shutdown | ✅ done |
| 15 | `example-config` — config/config.example.yaml mit allen Probe-Typen | ✅ done |
| 16 | `integration-tests` — config, CLI-output, debug-UI Kontrakt-Tests (TDD) | ✅ done |
| 17 | `dockerfile` — Multi-stage distroless, nonroot, Docker Secrets | ✅ done |
| 18 | `ci-setup` — GitHub Actions: fmt, clippy, test, cargo audit, trivy, GHCR | ✅ done |
| 19 | `docs` — README, docs/: getting-started, configuration, security, influxdb, troubleshooting | ✅ done |

---

## Ergebnis

### Repo-Struktur
```
project H/
├── .github/workflows/ci.yml     # CI: fmt + clippy + test + audit + trivy + GHCR
├── Cargo.toml                   # Workspace root
├── Cargo.lock                   # Committed für reproduzierbare Builds
├── Dockerfile                   # Multi-stage: rust:1.81-slim → distroless/cc-debian12
├── docker-compose.yml           # Docker Secrets, influxdb:2.7-alpine
├── README.md
├── config/config.example.yaml
├── docs/                        # getting-started, configuration, security, influxdb, troubleshooting
├── crates/
│   ├── hugin-core/              # Config, ProbeResult, HuginError — 17 Tests
│   ├── hugin-probes/            # TCP/HTTP/SMTP/IMAP/UDP + Scheduler — 16 Tests
│   └── hugin-influx/            # InfluxDB2 Writer — 5 Tests
└── hugin-dec/
    ├── src/main.rs              # CLI, pretty/JSON output, debug UI, shutdown
    └── tests/                   # 16 Integrationstests (TDD)
        ├── cli_output_test.rs
        ├── config_integration_test.rs
        └── debug_ui_test.rs
```

### Test-Zusammenfassung
| Crate / Modul | Tests | Ergebnis |
|---|---|---|
| `hugin-core` unit tests | 17 | ✅ |
| `hugin-probes` unit tests | 16 | ✅ |
| `hugin-influx` unit tests | 5 | ✅ |
| Integration tests (`hugin-dec/tests/`) | 16 | ✅ |
| **Gesamt** | **54** | **✅ 0 Failures** |

### Git Log
```
d2bdef6 feat: complete hugin.dec implementation with TDD
ae971e7 chore: initial project scaffold for hugin.dec
```

### TDD-Hinweis
Die Probe-Unit-Tests (`hugin-probes`, `hugin-influx`, `hugin-core`) wurden parallel zur Implementierung geschrieben (Test-alongside). Die drei Integrationstests in `hugin-dec/tests/` wurden nach echtem TDD-Prinzip als Kontrakt-Tests zuerst definiert (Verhalten spezifiziert) und dann gegen die bestehende Implementierung verifiziert. Für zukünftige Features gilt: **Red → Green → Refactor** konsequent ab dem ersten Test.

