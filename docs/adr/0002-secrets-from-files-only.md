# ADR-0002 — Secrets are read from files, never from the environment

**Status:** accepted · **Date:** 2026-08-06 (documenting a decision already in
the code)

## Context

huginn holds two secrets: the InfluxDB write token, and optionally an API key
protecting the Prometheus endpoint.

The conventional container answer is an environment variable. It is also the
worst available option for a value that must not leak. A process environment is
readable from `/proc/<pid>/environ`, is copied into every child process, is
printed by `docker inspect` to anyone who can reach the daemon, appears in
orchestrator manifests that live in version control, and lands in crash dumps
and error reporters that helpfully serialise the environment.

None of that is exotic. All of it is the default.

## Decision

Every secret is configured as a **path** — `influx.token_file`,
`metrics.api_key_file` — and read from that file at startup. No key anywhere
accepts a secret value inline, and no `HUGINN_*` variable carries one.

The behaviour is fail-closed: a missing file, an unreadable file, or an **empty**
file stops the process. It is never treated as "no secret configured".

## Consequences

- The token can be a Docker secret, a Kubernetes secret volume, or a tmpfs file
  at mode `0600`, and huginn does not need to know which.
- The value never appears in `docker inspect`, in a compose file, or in a
  manifest.
- Errors name the path, never the contents.
- An operator who expects `HUGINN_INFLUX_TOKEN` to work is refused at startup
  rather than silently unauthenticated. That is a deliberate friction.
- Rotating a secret means restarting the process; the file is read once. There is
  no reload, in line with the rest of the configuration model.

**The empty-file rule earned its place.** An empty token file used to be accepted
as an empty token. The process started, InfluxDB answered 401, and the writer
classified that 4xx as permanent and discarded every batch — a monitor that
looked healthy while losing all of its data. Fail-closed makes the same mistake
loud and immediate.

## Alternatives considered

**Environment variable with a `_FILE` suffix convention as an alternative.**
Rejected: offering both means the insecure one gets used, and the documentation
then has to explain when it is acceptable. It is not.

**Reading the secret lazily, at first use.** Rejected: it moves a fatal
configuration error from second one to whenever the first write happens, which is
exactly the class of delayed failure this project treats as worse than a crash.
