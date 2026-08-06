# ADR-0001 — One broadcast bus, with the scheduler as its only publisher

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

huginn produces one kind of event — a finished `ProbeResult` — and several
things want it: the console, the InfluxDB writer, the debug UI, the Prometheus
endpoint. That list has grown twice already and will grow again.

The obvious first shape is direct calls: the probe loop writes to InfluxDB, and
prints, and pushes to the UI. It works with one consumer and becomes a knot with
three. Every new consumer edits the probe loop, every consumer's latency is the
probe's latency, and a consumer that blocks blocks the measurement.

## Decision

`huginn-core::event::EventHub` wraps a tokio `broadcast::Sender<ProbeEvent>`.

The scheduler is the **only** publisher. Everything else subscribes. Consumers
are spawned tasks that own a `Receiver` and never talk to each other.

Two rules come with it:

1. **Subscribe before spawning.** A `broadcast::Receiver` only sees messages sent
   after it was created, so taking the receiver inside the spawned task loses
   the first probe tick whenever the executor does not poll that task promptly.
   The receiver is created on the main task and moved in.
2. **No subscriber may await slow I/O in its receive loop.** The channel has a
   fixed capacity (`event_hub_capacity`); a subscriber that falls behind makes
   the channel drop messages for *everyone*. This is why the InfluxDB path is
   split — see [ADR-0004](0004-bounded-retry-queue.md).

## Consequences

- A new consumer is a new subscriber. The Prometheus endpoint was added without
  touching the InfluxDB path or the scheduler.
- Consumers cannot slow down measurement, only themselves.
- A lagging subscriber is visible: `RecvError::Lagged(n)` is logged with the
  count rather than swallowed.
- Shutdown has a natural sequence: dropping the hub closes every receiver, which
  is how the batcher learns to flush.
- The bus is in-process and unpersisted. A result that is produced while a
  consumer is down is gone for that consumer. For the console and the UI that is
  correct; for InfluxDB it is what the retry queue exists to cover.

## Alternatives considered

**An mpsc channel per consumer, fanned out by the scheduler.** Rejected: the
scheduler would have to know its consumers, which is the coupling this removes.

**A shared `Vec<ProbeResult>` behind a mutex, polled by consumers.** Rejected:
polling adds latency to the live UI for no benefit, and lock contention would sit
directly in the probe loops.
