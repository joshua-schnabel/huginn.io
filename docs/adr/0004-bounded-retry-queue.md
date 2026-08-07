# ADR-0004 — The InfluxDB retry queue is bounded in memory, not in attempts

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

InfluxDB goes away. Restarts, deploys, disk-full, a network partition — an
uptime monitor that loses its data whenever its backend hiccups is reporting on
its own availability more than on anything else's.

The first shape wrote from the hub subscriber directly: receive a result, POST
it, log an error on failure. That has two faults, and the second is the subtle
one.

Failed writes were discarded. And because the subscriber awaited HTTP inside its
receive loop, a slow InfluxDB made the subscriber lag, which makes a tokio
`broadcast` channel drop messages **for every subscriber** — so a slow backend
silently degraded the console and the UI as well
([ADR-0001](0001-eventhub-single-publisher.md), rule 2).

## Decision

Split the path in two, with a bounded queue between them.

```text
EventHub ──► run_batcher ──► RetryQueue ──► run_writer ──► InfluxDB
             (never awaits    (bounded,      (retries,
              I/O)             drop-oldest)   backs off)
```

- **`run_batcher`** groups results and flushes on `batch_size` **or**
  `batch_timeout_ms`, whichever comes first. It never awaits I/O, so it can
  always keep up with the hub.
- **`RetryQueue`** is bounded by `max_buffered_bytes` and drops the **oldest**
  batch when full.
- **`run_writer`** drains the queue, classifies failures, and backs off
  exponentially between `retry_initial_backoff_ms` and `retry_max_backoff_ms`.

Failure classification decides retry:

| Response | Treated as | Action |
|---|---|---|
| transport error, 5xx, 429, 408 | retryable | exponential backoff, retry |
| any other 4xx | permanent | drop the batch, log it |

Retries are **unbounded in attempts** and **bounded in memory**.

## Consequences

- A backend outage costs the oldest data, not the newest, and not the process.
  For a monitor, the most recent state is the valuable one.
- A slow InfluxDB can no longer make the hub drop events for unrelated
  subscribers.
- Memory use has a configured ceiling, so the failure mode is bounded loss rather
  than an OOM kill.
- A permanently misconfigured backend (wrong bucket, wrong org → 404/401) does
  not accumulate an unbounded queue of batches that can never succeed.
- Shutdown needs a bounded drain window, because unbounded attempts would
  otherwise keep the process alive forever against a dead backend. That is
  `influx.shutdown_drain_timeout_ms`.

## Alternatives considered

**Bound the retries instead (give up after N attempts).** Rejected: N is
impossible to choose. Too low loses data during a normal restart; high enough to
survive a restart is indistinguishable from unbounded, without the memory
ceiling that actually protects the process.

**Persist the queue to disk.** Rejected for now: it turns a stateless container
into one with a volume, and the failure it protects against — huginn itself
restarting during a backend outage — is one where the orchestrator is already
restarting things. Recorded in [`risks.md`](../risks.md) rather than dismissed.

**Drop the newest batch when full.** Rejected: the newest result is the one an
alert rule is about to read.
