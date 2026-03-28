# Security

## Secrets Management

huginn.io follows a strict **no-secrets-in-ENV** policy.

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
INFLUX_TOKEN=mytoken huginn   # token visible in ps, /proc/environ, logs
```

## Docker Secrets
```yaml
# docker-compose.yml
services:
  huginn:
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

`cargo-deny` replaces `cargo-audit` and adds license and registry checks:

```bash
cargo deny check
```

Configuration in `deny.toml`:
- **Advisories** — RustSec advisory database (like cargo-audit)
- **Licenses** — only approved SPDX licenses (MIT, Apache-2.0, BSD, ISC, …)
- **Bans** — forbidden crates; warns on duplicate versions
- **Sources** — only `crates.io` as registry

Runs automatically in CI (`supply-chain` job).

## Static Analysis (SAST)

Semgrep scans all Rust source on every PR:
- `p/rust` — Rust security patterns (unsafe, integer overflows, …)
- `p/secrets` — hardcoded secrets in source code

Findings appear in the GitHub Security tab. ERROR-severity findings block the PR.

## Image Scanning

```bash
# Scan locally with Trivy
docker build -t huginn .
trivy image --severity HIGH,CRITICAL huginn
```

Also runs automatically in CI before any image push.
