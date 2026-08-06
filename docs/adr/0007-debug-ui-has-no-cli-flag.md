# ADR-0007 — The debug UI is enabled by config or ENV, and binds loopback

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

The debug UI is an unauthenticated live view of every probe result, served over
plain HTTP. It is genuinely useful during setup and genuinely dangerous left on:
it discloses every host huginn monitors, and it will answer anyone who can reach
the port.

Two questions follow: how is it turned on, and what does it bind to when it is.

## Decision

**There is no CLI flag.** The UI is enabled by `ui.enabled: true` in the config
file or `HUGINN_UI_ENABLED=true` in the environment — the same two places that
configure everything else. The Prometheus endpoint is gated the same way, and
separately (`metrics.enabled`), so enabling one never enables the other.

**It binds `127.0.0.1` by default** (`ui.bind`, `HUGINN_UI_BIND`). Reaching it
from outside the machine is an explicit act.

## Consequences

- Whether the UI is on is a property of the deployment's configuration, visible
  in the file that is under review, rather than of an argv that lives in a shell
  history or a `docker run` line nobody re-reads.
- **In a container the default is wrong on purpose.** A published port reaches
  nothing unless `ui.bind` is `0.0.0.0`, so exposing the UI to a network takes
  two deliberate settings, not one. This surprises people once; the README,
  `configuration.md` and AGENTS.md all say it, because it is the single most
  common "why can't I reach it" question.
- A cargo alias cannot express it either — aliases cannot set environment
  variables — so `.cargo/config.toml` documents the `HUGINN_UI_ENABLED=true
  cargo dev` invocation instead of hiding it.
- The UI still has no authentication. That is why loopback is the default and
  why the shipped compose file publishes on `127.0.0.1`. Authentication is a
  [`roadmap.md`](../roadmap.md) item, not a claim made here.
- Both listeners send strict security response headers (a CSP without
  `unsafe-inline`, `nosniff`, `DENY`, `no-referrer`, `no-store`), so the
  unauthenticated surface at least cannot be framed or trivially scraped
  cross-origin.

## Alternatives considered

**A `--ui` flag.** Rejected: it makes "is the debug UI exposed" a question you
answer by finding out how the process was started, which in a container means
reading someone's compose file, orchestrator manifest or shell history.

**Bind `0.0.0.0` by default because that is what containers need.** Rejected:
the default should be the safe one, and the container case is exactly where
getting it wrong is worst — a published port would expose the UI to whatever the
host is reachable from.
