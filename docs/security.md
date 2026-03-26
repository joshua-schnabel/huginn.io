# Security

## Secrets Management

hugin.dev follows a strict **no-secrets-in-ENV** policy.

### ✅ Correct: Token in a file
```yaml
influx:
  token_file: "/run/secrets/influx_token"
```
The token is read from the file at startup. The file should be:
- Owned by `root` or the service user
- Mode `0600` (readable only by owner)
- Mounted as a Docker secret (ephemeral tmpfs, never persisted to disk)

### ❌ Wrong: Token in ENV
```bash
# DO NOT do this
INFLUX_TOKEN=mytoken hugin-dev   # token visible in ps, /proc/environ, logs
```

## Docker Secrets
```yaml
# docker-compose.yml
services:
  hugin-dev:
    secrets:
      - influx_token
    environment:
      INFLUX_TOKEN_FILE: /run/secrets/influx_token  # ← path, not value

secrets:
  influx_token:
    file: ./secrets/influx_token.txt  # ← never commit this file
```

Add to `.gitignore`:
```
secrets/
*.token
.env
```

## Container Hardening

| Measure | Details |
|---|---|
| **Distroless base** | `gcr.io/distroless/cc-debian12` — no shell, no apt, minimal surface |
| **Non-root user** | Runs as `nonroot:nonroot` |
| **No capabilities** | No `--cap-add`, no privileged mode needed |
| **Read-only config** | Config file mounted `:ro` in compose |
| **rustls** | TLS via pure-Rust rustls — no OpenSSL dependency |

## Dependency Audit

```bash
# Install
cargo install cargo-audit --locked

# Run
cargo audit
```

Run automatically in CI (`audit` job in `.github/workflows/ci.yml`).

## Image Scanning

```bash
# Scan locally with Trivy
docker build -t hugin-dev .
trivy image --severity HIGH,CRITICAL hugin-dev
```

Also runs automatically in CI before any image push.
