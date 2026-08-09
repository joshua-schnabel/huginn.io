# Architecture

huginn is one process running many independent probe loops and publishing every
result onto one bus. Everything below follows from that.

## The shape

```text
                       ┌────────────────────────────────────────────┐
   config.yaml ───────►│  huginn                                    │
   HUGINN_* env ──────►│                                            │
   /run/secrets/* ────►│  AppConfig                                 │
                       │      │                                     │
                       │      ▼                                     │
                       │  Scheduler ──┐ one loop per probe          │
                       │              │                             │
                       │              ▼                             │
                       │          EventHub  (tokio broadcast)       │
                       │              │                             │
                       │   ┌──────────┼───────────┬─────────────┐   │
                       │   ▼          ▼           ▼             │   │
                       │ console   batcher ─► queue ─► writer   │───┼──► InfluxDB
                       │           (no I/O)   (bounded)         │   │
                       │                                  WebState  │
                       │                                   │    │   │
                       │                    ┌──────────────┴──┐ │   │
                       │                    ▼                 ▼ │   │
                       │              debug UI :9116   /metrics :9464
                       └────────────────────────────────────────────┘
                             /  /health  /metrics/latest  /events
```

Three things are worth noticing in that picture.

**The scheduler is the only publisher.** Everything else subscribes. A new
consumer of probe results is a new subscriber and touches nothing that already
exists — that is how the Prometheus endpoint was added without the InfluxDB path
changing.

**The InfluxDB path is two tasks with a bounded queue between them, not one.**
The batcher groups results and never awaits I/O; the writer drains the queue and
retries. A slow or dead InfluxDB therefore cannot stall the hub reader and make
the broadcast channel drop events at the source. The queue is bounded in bytes
(`max_buffered_bytes`, drop-oldest), so an InfluxDB that is down for a day costs
old results rather than the process.

**Both HTTP listeners are optional and gated separately.** They share one
`WebState`, which subscribes to the hub once, so enabling neither, either or both
makes no difference to the hub's fan-out.

## Crates

| Crate | Responsibility |
|---|---|
| `huginn` | CLI, config load, logging setup, the scheduler, shutdown, orchestration |
| `huginn-core` | Shared types (`ProbeResult`), config model, `HuginError`, the `EventHub` |
| `huginn-probes` | The `Probe` trait, the `ProbeRegistry`, and one executor per protocol |
| `huginn-influx` | Line-protocol writer: batching, the bounded retry queue, HTTP POST |
| `huginn-web` | Axum debug server (`/`, `/health`, `/metrics/latest`, `/events`) and the separately-gated Prometheus listener |

Dependencies point one way: `huginn` → everything; `huginn-probes`,
`huginn-influx` and `huginn-web` → `huginn-core`. No cycles, and `huginn-core`
knows nothing about HTTP, InfluxDB or any probe protocol.

## Startup sequence

The order is the design. Everything that can fail does so before any probe runs
and before anything is written anywhere.

| # | Step | On failure |
|---|---|---|
| 1 | Parse CLI arguments | exit non-zero |
| 2 | Load the YAML and apply `HUGINN_*` overrides | exit non-zero |
| 3 | Validate probes, targets and numeric bounds | exit non-zero |
| 4 | Resolve the output format (CLI > ENV > YAML) | — |
| 5 | Initialise `tracing` | — |
| 6 | Create the `EventHub` | — |
| 7 | Subscribe the console, then the batcher | — |
| 8 | Read the InfluxDB token file | exit non-zero |
| 9 | Read the metrics API key file, if configured | exit non-zero |
| 10 | Spawn the listeners, then the scheduler | — |
| 11 | Wait for a stop signal | — |

Two ordering details are load-bearing.

**Config warnings are collected, not logged, until step 5.** The log level comes
from the very config being loaded, so anything logged before `tracing` is
initialised would be discarded. `load_with_warnings` returns them and step 5 is
followed immediately by emitting them.

**Every subscriber subscribes before its task is spawned.** A tokio `broadcast`
receiver only sees messages sent after it subscribed, so subscribing inside the
spawned task would lose the first probe tick whenever the executor did not poll
the task in time. Both the console and the batcher take a receiver on the main
task and move it in.

**Secret files are read at startup, not at first use** — steps 8 and 9. A missing
or empty token file stops the process rather than producing a monitor that runs,
gets 401 from InfluxDB, classifies it as permanent, and discards every batch
while looking healthy.

## Probes

A probe is one async loop with its own interval and timeout, spawned by the
scheduler. Loops are independent: a probe blocked on a 30-second timeout delays
nothing but itself.

| Type | Measures |
|---|---|
| `tcp` | connect success and time to connect |
| `http` / `https` | request success, status code, response time |
| `smtp`, `imap` | banner exchange and response time |
| `udp` | send/receive round trip |
| `dns` | resolution success, optionally that the answer matches `dns_expected_ip` |
| `tls` | days until the server certificate expires, as a metric on the result |

Every executor implements `Probe`, and the `ProbeRegistry` owns the state they
share — the HTTP client — so no loop threads a resource it does not use. A probe
never panics: a failure becomes `ProbeResult::failure()` and is published like
any other result. A monitor that dies from what it is monitoring is not a
monitor.

Per-type numeric readings live in `ProbeResult.metrics`, a `BTreeMap<String,
f64>` that becomes additional line-protocol fields and additional Prometheus
gauges. That is how `tls_cert_expiry_days` reaches both backends without either
knowing what TLS is.

## Shutdown

A separate `broadcast::channel<()>` carries the stop signal, fired by SIGINT on
every platform and SIGTERM on Unix. SIGTERM is the one `docker stop` and systemd
actually send, and catching only SIGINT meant the drain below never ran under
either — every buffered result was lost on each deploy.

1. The signal fires; `run()` stops waiting.
2. **Every probe loop stops and is joined.** Each holds its own
   `Arc<EventHub>`, so this has to complete before step 3 means anything.
3. The hub is dropped, which closes every subscriber's receiver.
4. The batcher takes `Closed` as its cue to queue what it still holds and close
   the queue.
5. The writer drains, bounded by `influx.shutdown_drain_timeout_ms`.
6. The process exits.

**Step 2 is not a formality, and its position is the whole point.** The loops
were once detached: `run()` dropped only its own hub clone and went straight to
the drain. A probe still inside a slow read — an SMTP or IMAP peer can consume a
full timeout connecting and another one being read from — kept the hub's
`Sender` alive, so the batcher never observed `Closed` and never queued the
partial batch it was holding. The drain then ran its full timeout and logged
that InfluxDB was unreachable, while InfluxDB had been reachable throughout.
Each loop therefore also selects on the shutdown channel *during* a probe, not
only between two of them, and anything still running when the grace period ends
is aborted so it cannot hold the hub open indefinitely. The drain's timeout
message names which stage overran, because "the backend is down" and "a probe
would not stop" call for opposite responses.

The bound in step 5 matters as much as the wait itself: retries are unbounded in
attempts, so an InfluxDB that is down at shutdown would otherwise keep the
process alive forever. On timeout the buffered results are discarded and a
warning says so.

An interrupted probe publishes nothing. Inventing a DOWN result for it would
write a fake outage into InfluxDB on every deploy.

There is no configuration reload. Change the YAML, restart the process.

## Where to read next

- [`configuration.md`](configuration.md) — every key, its default and its effect
- [`influxdb.md`](influxdb.md) — the measurement and field schema huginn writes
- [`testing.md`](testing.md) — the test pyramid and the no-sleep rule
- [`hardening.md`](hardening.md) — the container and the secret handling
- [`roadmap.md`](roadmap.md) — what is still open

### Architecture decisions

| ADR | Subject |
|---|---|
| [0001](adr/0001-eventhub-single-publisher.md) | One broadcast bus with the scheduler as sole publisher |
| [0002](adr/0002-secrets-from-files-only.md) | Secrets are read from files, never from the environment |
| [0003](adr/0003-rustls-only.md) | rustls only, OpenSSL banned outright |
| [0004](adr/0004-bounded-retry-queue.md) | The InfluxDB retry queue is bounded in memory, not in attempts |
| [0005](adr/0005-distroless-nonroot.md) | A distroless, nonroot runtime image |
| [0006](adr/0006-tls-probe-skips-verification.md) | The TLS probe skips certificate verification |
| [0007](adr/0007-debug-ui-has-no-cli-flag.md) | The debug UI is enabled by config or ENV, never by a CLI flag |

## Related

- [`ci-cd.md`](ci-cd.md) — how this is built, scanned and published
- [`versioning.md`](versioning.md) — which of the above is a stable surface
- [`risks.md`](risks.md) — what is known to be weak
