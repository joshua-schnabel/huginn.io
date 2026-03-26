# Getting Started

## Prerequisites

- Docker & Docker Compose (recommended), **or** Rust 1.81+
- An InfluxDB 2.x instance

## Docker Quickstart

```bash
git clone https://github.com/your-org/hugin-dev.git
cd hugin-dev

# Create secrets (never commit these files)
mkdir -p secrets
echo "your-influxdb-token"    > secrets/influx_token.txt
echo "your-admin-password"    > secrets/influx_admin_password.txt
chmod 600 secrets/*.txt

# Copy and adjust config
cp config/config.example.yaml config/config.yaml
$EDITOR config/config.yaml

# Start everything
docker compose up -d

# Tail logs
docker compose logs -f hugin-dev
```

Open **http://localhost:9116** for the debug UI.

## Binary (Rust)

```bash
cargo build --release --locked

# Create a token file
echo "mytoken" > /etc/hugin/influx_token
chmod 600 /etc/hugin/influx_token

./target/release/hugin-dev --config config/config.yaml
```

## JSON Output

```bash
./target/release/hugin-dev --output json | jq .
# or via ENV:
HUGIN_LOG_FORMAT=json hugin-dev
```
