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
#   6. The Prometheus endpoint on :9464 rejects scrapes without the API key
#      (401) and serves huginn_probe_* gauges with it
#   7. InfluxDB contains probe_result measurements (Flux query)

set -euo pipefail

HUGINN_URL="http://localhost:9116"
INFLUX_URL="http://localhost:8086"
INFLUX_TOKEN="integration-test-token-huginn-ci"
INFLUX_ORG="testorg"
INFLUX_BUCKET="testbucket"

# The stack this script asserts against. Overridable so the file is not the only
# thing that knows its own name, but it is the one CI brings up.
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.integration.yml}"

# Every probe in config/config.integration.yaml, which between them cover all
# eight advertised probe types. Keep this list and that file in step: the count
# is what the readiness polls wait for, so a probe added there without being
# added here would simply never be checked.
EXPECTED_PROBE_NAMES="influxdb-tcp influxdb-http influxdb-https-type docker-dns docker-dns-udp tls-cert smtp-banner imap-greeting"
# shellcheck disable=SC2086  # word splitting is the point: one name per line.
EXPECTED_PROBES=$(printf '%s\n' $EXPECTED_PROBE_NAMES | wc -l | tr -d ' ')

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
  if [ "$COUNT" -ge "$EXPECTED_PROBES" ]; then
    pass "/metrics/latest has $COUNT results"
    break
  fi
  [ "$i" -eq 15 ] && fail "/metrics/latest had fewer than $EXPECTED_PROBES results after 30s (got: $COUNT)"
  sleep 2
done

# ── 4. Assert expected probe names are present ────────────────────────────────
METRICS=$(curl -sf "$HUGINN_URL/metrics/latest")
python3 -c "
import json, sys
data = json.loads('''$METRICS''')
names = [r['probe_name'] for r in data]
expected = [
    'influxdb-tcp', 'influxdb-http', 'influxdb-https-type',
    'docker-dns', 'docker-dns-udp', 'tls-cert',
    'smtp-banner', 'imap-greeting',
]
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

# ── 7. Assert the Prometheus endpoint serves huginn_probe_* gauges ────────────
# Auth is enabled in config.integration.yaml: no key ⇒ 401, correct key ⇒ 200.
METRICS_KEY="integration-test-metrics-key"
UNAUTHED=$(curl -s -o /dev/null -w '%{http_code}' "http://localhost:9464/metrics")
[ "$UNAUTHED" = "401" ] || fail "Prometheus endpoint without key returned $UNAUTHED, expected 401"
pass "Prometheus endpoint rejects unauthenticated scrapes (401)"
PROM=$(curl -sf -H "Authorization: Bearer $METRICS_KEY" "http://localhost:9464/metrics") \
  || fail "Prometheus endpoint on :9464 not reachable with the API key"
echo "$PROM" | grep -q '^# TYPE huginn_probe_success gauge$' \
  || fail "Prometheus output lacks the huginn_probe_success family"
echo "$PROM" | grep -q 'huginn_probe_success{probe="tls-cert",type="tls",target="tls-endpoint:443"} 1' \
  || fail "Prometheus output lacks the tls-cert success sample"
echo "$PROM" | grep -q 'huginn_probe_tls_cert_expiry_days{probe="tls-cert"' \
  || fail "Prometheus output lacks the tls_cert_expiry_days gauge"
pass "Prometheus endpoint serves huginn_probe_* gauges"

# ── 7b. The container's own health check ──────────────────────────────────────
# `huginn healthcheck` inside the running container, which is exactly what the
# image's HEALTHCHECK runs — and the only place it can be tested, because the
# liveness listener is bound to loopback inside the container and is deliberately
# unreachable from here.
if docker compose -f "$COMPOSE_FILE" exec -T huginn \
     /usr/local/bin/huginn --config /etc/huginn/config.yaml healthcheck > /dev/null 2>&1; then
  pass "huginn healthcheck reports the container healthy"
else
  fail "huginn healthcheck failed inside the container — the image's HEALTHCHECK would mark it unhealthy"
fi

# And Docker's own verdict, which is what an orchestrator acts on. `starting` is
# a legitimate transient state during start_period, so this polls for `healthy`
# rather than reading it once.
info "Waiting for Docker to report the container healthy (up to 60s)..."
for i in $(seq 1 30); do
  CID=$(docker compose -f "$COMPOSE_FILE" ps -q huginn)
  STATUS=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$CID" 2>/dev/null || echo "none")
  [ "$STATUS" = "healthy" ] && break
  if [ "$STATUS" = "none" ]; then
    fail "the container declares no HEALTHCHECK — the image should carry one"
  fi
  [ "$i" -eq 30 ] && fail "container health stayed '$STATUS' for 60s"
  sleep 2
done
pass "Docker reports the huginn container healthy"

# ── 8. Wait for InfluxDB to hold every expected series ────────────────────────
# Polled, not slept. A fixed `sleep 5` was here, which is both slower than it
# needs to be on a fast machine and not long enough on a loaded CI runner —
# docs/testing.md says poll, and this file was the one place that did not.
info "Waiting for all $EXPECTED_PROBES probes to reach InfluxDB (up to 60s)..."
FLUX_SERIES='from(bucket:"'"$INFLUX_BUCKET"'")
  |> range(start: -5m)
  |> filter(fn: (r) => r._measurement == "probe_result")
  |> keep(columns: ["probe_name"])
  |> group()
  |> distinct(column: "probe_name")'

for i in $(seq 1 30); do
  SERIES=$(curl -sf -X POST "$INFLUX_URL/api/v2/query?org=$INFLUX_ORG" \
    -H "Authorization: Token $INFLUX_TOKEN" \
    -H "Content-Type: application/vnd.flux" \
    --data "$FLUX_SERIES" 2>/dev/null || true)

  # Parsed as CSV rather than grepped. Flux returns annotated CSV with CRLF line
  # endings, so an anchored grep pattern is fragile; an unanchored one would let
  # `docker-dns` also match `docker-dns-udp` and overcount. This reports which
  # probes are missing, which is the thing worth knowing when it fails.
  MISSING=$(printf '%s' "$SERIES" | EXPECTED="$EXPECTED_PROBE_NAMES" python3 -c "
import csv, io, os, sys
want = set(os.environ['EXPECTED'].split())
seen = {c.strip() for row in csv.reader(io.StringIO(sys.stdin.read())) for c in row}
print(' '.join(sorted(want - seen)))
" 2>/dev/null || echo "$EXPECTED_PROBE_NAMES")

  [ -z "$MISSING" ] && break
  [ "$i" -eq 30 ] && fail "these probes never reached InfluxDB within 60s: $MISSING"
  sleep 2
done
pass "All $EXPECTED_PROBES probes have written to InfluxDB"

# ── 9. Assert the fields InfluxDB actually holds ──────────────────────────────
# The previous check asked only whether *some* probe_result existed, which a
# single probe writing a single field would satisfy — so the schema that
# docs/influxdb.md calls a stable surface was effectively untested. This asserts
# the fields by name, and that the tls probe's per-type metric arrives too.
FLUX_FIELDS=$(curl -sf \
  -X POST "$INFLUX_URL/api/v2/query?org=$INFLUX_ORG" \
  -H "Authorization: Token $INFLUX_TOKEN" \
  -H "Content-Type: application/vnd.flux" \
  --data 'from(bucket:"'"$INFLUX_BUCKET"'")
    |> range(start: -5m)
    |> filter(fn: (r) => r._measurement == "probe_result")
    |> keep(columns: ["_field"])
    |> group()
    |> distinct(column: "_field")' \
  2>&1 || echo "CURL_ERROR")

case "$FLUX_FIELDS" in
  *CURL_ERROR*) fail "Could not reach the InfluxDB query API" ;;
esac

printf '%s' "$FLUX_FIELDS" | python3 -c "
import csv, io, sys

# up and response_ms are written by every probe; tls_cert_expiry_days only by
# the tls one, and it is the path by which any future per-type metric reaches
# InfluxDB — so its absence would mean the whole ProbeResult.metrics map is not
# arriving, not merely that one number is missing.
required = {'up', 'response_ms', 'tls_cert_expiry_days'}
found = {c.strip() for row in csv.reader(io.StringIO(sys.stdin.read())) for c in row}
missing = sorted(required - found)
if missing:
    print('missing fields in InfluxDB:', missing)
    print('fields present:', sorted(f for f in found if f and not f.startswith('_') and f != 'result' and f != 'table'))
    sys.exit(1)
print('probe_result fields present:', sorted(required))
" && pass "InfluxDB holds the documented probe_result fields" \
  || fail "InfluxDB is missing documented probe_result fields"

echo ""
echo "════════════════════════════════════════════════"
echo -e "${GREEN}  All system integration tests passed! ✓${NC}"
echo "════════════════════════════════════════════════"
echo ""
