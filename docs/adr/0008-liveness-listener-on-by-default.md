# ADR-0008 — A liveness listener, on by default, fixed to loopback

**Status:** accepted · **Date:** 2026-08-09

## Context

The runtime image had no `HEALTHCHECK`. An orchestrator could see that huginn's
process existed, and nothing more — a huginn whose runtime had stopped
scheduling would keep its container in the `running` state indefinitely, which
for a monitor is the failure mode that matters most: it is not measuring, and
nothing says so.

Adding one is not a one-liner, because the image is distroless
([ADR-0005](0005-distroless-nonroot.md)). There is no shell and no `curl`, so
the check has to be the binary itself: `huginn healthcheck`. That subcommand
needs something to ask.

The obvious candidate was the debug UI's existing `/health`. It does not work.
The UI is **off by default** ([ADR-0007](0007-debug-ui-has-no-cli-flag.md)), so
a `HEALTHCHECK` built on it reports *unhealthy* on every stock deployment. A
health check that is wrong out of the box is worse than none: it teaches
operators that the status column is noise, and then it is noise on the day it
would have told them something.

So the endpoint the check depends on has to be available without configuration —
which means a listener that is on by default, in a project whose stated posture
is that both existing listeners are off by default and bind loopback
([`hardening.md`](../hardening.md),
[ADR-0009](0009-debug-ui-stays-unauthenticated.md)).

## Decision

**A third listener, dedicated to liveness, on by default** (`health.enabled`,
default `true`; `health.port`, default `9115`).

**It is fixed to `127.0.0.1` and there is no `bind` key.** Every other listener
has one; this one does not, and that omission is the entire security argument.
Docker runs `HEALTHCHECK` *inside* the container, so loopback is exactly where
the check needs it — while a published port reaches the container's bridge IP
and therefore never reaches this socket at all. The listener cannot be exposed
to a network by any configuration, only by turning it off and using something
else.

**It serves `GET /health` and nothing else**, and the response is the string
`OK`. No probe names, no targets, no error strings, no counts — nothing that
would make it an infrastructure map the way the debug UI is.

**Liveness, not readiness.** A 200 means the process is running and its async
runtime is still scheduling. It deliberately does not consider whether probes
are succeeding or whether InfluxDB is reachable.

**Startup fails if it cannot bind.** The listener is bound on the main task
before the scheduler starts, not inside a spawned one.

## Consequences

- The image carries a `HEALTHCHECK` that works on an unmodified deployment, and
  `docker ps` reports something true.
- The default configuration now opens a socket it previously did not. On
  loopback, serving two bytes of constant text, in a distroless image with no
  other process in it — but it is a socket, and this ADR is where that is
  admitted rather than left implicit.
- **`health.port` is fixed, so several huginns on one host collide.** In
  containers this never arises: each has its own network namespace. Outside
  them — or under `network_mode: host` — the second instance fails at startup
  with a message naming the port. That is the right failure, and it is why the
  binary-lifecycle tests, which run several huginns in parallel on one host, set
  the port explicitly.
- Startup can now fail for a reason unrelated to probing or InfluxDB. It says
  which listener and which port, because "on by default" means it is the one
  nobody remembers enabling.
- `huginn healthcheck` with `health.enabled: false` reports that the setting is
  off, rather than a connection error that reads like a dead process.
- Config validation rejects a `ui` or `metrics` listener placed on the health
  port while it is bound to loopback — otherwise one of the two would lose the
  bind at runtime with only a logged error.

## Alternatives considered

**Reuse the debug UI's `/health`.** Rejected: it is off by default, so the check
would be wrong on every stock deployment. Turning the UI on to get a health
check would trade an unauthenticated infrastructure map for a liveness signal,
which is a much worse deal than opening one constant-response socket.

**No `HEALTHCHECK` in the image; let operators add one.** Rejected, though it is
the smallest change. It leaves the default deployment with no liveness signal at
all, and the operators least likely to write their own are exactly those running
the image unmodified.

**A liveness file touched by the daemon, checked by the subcommand.** Rejected:
it needs a writable path in an image whose root filesystem is read-only by
design ([`hardening.md`](../hardening.md)), so every deployment manifest would
have to grow a tmpfs mount. A socket costs less and changes no manifest.

**Readiness semantics — report unhealthy when InfluxDB is unreachable.**
Rejected outright. The orchestrator would restart huginn because something
huginn monitors is down, taking out the monitor at the moment it is most needed,
and the retry queue exists precisely so that a backend outage is survivable
([ADR-0004](0004-bounded-retry-queue.md)).

**Make `bind` configurable, defaulting to loopback, like the other two.**
Rejected: the other two are off by default, so a wide `bind` is something an
operator opted into twice. This one is on by default, and a key that lets an
on-by-default listener reach a network is a key that will eventually be set.

## Related

- [ADR-0005](0005-distroless-nonroot.md) — why there is no shell to check with
- [ADR-0007](0007-debug-ui-has-no-cli-flag.md) — why the UI is off by default
- [`hardening.md`](../hardening.md) — the container posture this sits inside
- [`configuration.md`](../configuration.md) — the `health` keys
