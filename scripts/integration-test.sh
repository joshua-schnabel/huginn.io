#!/usr/bin/env bash
# scripts/integration-test.sh
#
# System Integration Test for huginn.io.
# Assumes docker-compose.integration.yml is up and services are starting.
#
# Assertions:
#   1. huginn /health returns "OK"
#   2. /metrics/latest returns at least 5 probe results
#   3. Expected probe names (tcp, http, dns, udp, tls probes) are present
#   4. All probes are up: true
#   5. The TLS probe reports a positive tls_cert_expiry_days metric
#   6. InfluxDB contains probe_result measurements (Flux query)

set -euo pipefail

HUGINN_URL="http://localhost:9116"
INFLUX_URL="http://localhost:8086"
INFLUX_TOKEN="integration-test-token-huginn-ci"
INFLUX_ORG="testorg"
INFLUX_BUCKET="testbucket"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}✓ $*${NC}"; }
fail() { echo -e "${RED}✗ $*${NC}"; exit 1; }
info() { echo -e "${YELLOW}→ $*${NC}"; }

echo ""
echo "════════════════════════════════════════════════"
echo "  huginn.io — System Integration Test"
echo "════════════════════════════════════════════════"
echo ""

# ── 1. Wait for huginn /health ─────────────────────────────────────────────
info "Waiting for huginn /health (up to 60s)..."
for i in $(seq 1 30); do
  if curl -sf "$HUGINN_URL/health" > /dev/null 2>&1; then
    pass "huginn is responding"
    break
  fi
  [ "$i" -eq 30 ] && fail "huginn did not respond after 60s"
  sleep 2
done

# ── 2. Assert /health content ─────────────────────────────────────────────────
HEALTH=$(curl -sf "$HUGINN_URL/health")
[ "$HEALTH" = "OK" ] && pass "/health returned 'OK'" || fail "/health returned unexpected: $HEALTH"

# ── 3. Wait for probe results (probes run every 2s) ───────────────────────────
info "Waiting for probe results in /metrics/latest (up to 30s)..."
for i in $(seq 1 15); do
  COUNT=$(curl -sf "$HUGINN_URL/metrics/latest" \
    | python3 -c "import json,sys; print(len(json.load(sys.stdin)))" 2>/dev/null || echo 0)
  if [ "$COUNT" -ge 5 ]; then
    pass "/metrics/latest has $COUNT results"
    break
  fi
  [ "$i" -eq 15 ] && fail "/metrics/latest had fewer than 5 results after 30s (got: $COUNT)"
  sleep 2
done

# ── 4. Assert expected probe names are present ────────────────────────────────
METRICS=$(curl -sf "$HUGINN_URL/metrics/latest")
python3 -c "
import json, sys
data = json.loads('''$METRICS''')
names = [r['probe_name'] for r in data]
expected = ['influxdb-tcp', 'influxdb-http', 'docker-dns', 'docker-dns-udp', 'tls-cert']
missing = [n for n in expected if n not in names]
if missing:
    print('Missing probes:', missing)
    sys.exit(1)
print('Probe names present:', names)
" && pass "Expected probe names found" || fail "Expected probe names missing"

# ── 5. Assert all probes are up ───────────────────────────────────────────────
python3 -c "
import json, sys
data = json.loads('''$METRICS''')
down = [r['probe_name'] for r in data if not r.get('up', False)]
if down:
    print('Probes reporting DOWN:', down)
    sys.exit(1)
print('All', len(data), 'probes are UP')
" && pass "All probes are UP" || fail "One or more probes are DOWN (check container logs)"

# ── 6. Assert the TLS probe reports a positive expiry metric ──────────────────
python3 -c "
import json, sys
data = json.loads('''$METRICS''')
tls = next(r for r in data if r['probe_name'] == 'tls-cert')
days = tls.get('metrics', {}).get('tls_cert_expiry_days')
if days is None:
    print('tls-cert result has no tls_cert_expiry_days metric:', tls)
    sys.exit(1)
if days <= 0:
    print('tls_cert_expiry_days should be positive for a fresh cert, got:', days)
    sys.exit(1)
print('tls_cert_expiry_days =', days)
" && pass "TLS probe reports positive tls_cert_expiry_days" || fail "TLS expiry metric missing or non-positive"

# ── 7. Wait for InfluxDB write (batch_size=1 so it flushes immediately) ───────
info "Waiting for InfluxDB write (up to 15s)..."
sleep 5

# ── 8. Query InfluxDB for probe_result measurements ───────────────────────────
FLUX_RESULT=$(curl -sf \
  -X POST "$INFLUX_URL/api/v2/query?org=$INFLUX_ORG" \
  -H "Authorization: Token $INFLUX_TOKEN" \
  -H "Content-Type: application/vnd.flux" \
  --data "from(bucket:\"$INFLUX_BUCKET\") |> range(start: -2m) |> filter(fn: (r) => r._measurement == \"probe_result\") |> count()" \
  2>&1 || echo "CURL_ERROR")

if echo "$FLUX_RESULT" | grep -q "probe_result"; then
  pass "InfluxDB contains probe_result measurements"
elif echo "$FLUX_RESULT" | grep -q "CURL_ERROR"; then
  fail "Could not reach InfluxDB API"
else
  fail "No probe_result data in InfluxDB. Response: $FLUX_RESULT"
fi

echo ""
echo "════════════════════════════════════════════════"
echo -e "${GREEN}  All system integration tests passed! ✓${NC}"
echo "════════════════════════════════════════════════"
echo ""
