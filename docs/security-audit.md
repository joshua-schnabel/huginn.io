# Security audit — 2026-08-02

A full security review of huginn.io before the first release: adversarial
testing against a running instance plus a line-by-line code review, across the
whole product rather than the application code alone.

This is a point-in-time report. Practices live in
[`hardening.md`](hardening.md); to report a vulnerability see
[`SECURITY.md`](SECURITY.md).

## Scope and method

| | |
|---|---|
| **Reviewed** | `9a8a371` (`dev`) — all five crates, the debug-UI assets, `Dockerfile`, both compose files, all six workflows, `deny.toml`, `Cargo.lock`, config and secrets handling, docs |
| **Toolchain** | rustc 1.94.0, cargo-deny 0.20.2, Docker 29.3.1 |
| **Runtime tested** | `huginn:ci` built from this tree on `gcr.io/distroless/cc-debian12`, run via `docker-compose.integration.yml` |
| **Out of scope** | InfluxDB itself, the host OS, the GitHub platform |

Seven domains were worked through: the HTTP surface, probe inputs, data egress,
secrets and config, container and compose, CI/CD, supply chain. For each, attack
hypotheses were formed and then **proved or disproved** — dynamically where
possible, by tracing the code path otherwise. Disproved hypotheses are recorded
too (["Checked, not exploitable"](#checked-not-exploitable)): knowing a thing was
tested is part of the result.

The attack setup: a hostile SMTP server answering with a banner that carries ANSI
and OSC escape sequences, HTML, and line-protocol metacharacters; probe names and
targets seeded with the same payloads; a connection-flood harness; and the full
method/path/auth matrix against both listeners.

## Findings

| ID | Severity | Component | Finding | Status |
|---|---|---|---|---|
| [F-01](#f-01) | **Medium** | `huginn-core`, probes | Control-character injection from a monitored host into console, logs and InfluxDB | **Fixed** |
| [F-02](#f-02) | Low | `huginn-web` | No security response headers on either listener | **Fixed** |
| [F-03](#f-03) | Low | `huginn-web` | No connection cap or request timeout on either listener | **Mitigated** — bounded by container limits, documented |
| [F-04](#f-04) | Low | `docker-compose.yml` | Shipped compose ran without container hardening and published both ports to `0.0.0.0` | **Fixed** |
| [F-05](#f-05) | **Medium** | `huginn-core` | An empty InfluxDB token file started the process, which then discarded every batch | **Fixed** |
| [F-06](#f-06) | Low | `deny.toml` | The "rustls only, no OpenSSL" policy was documented but not enforced | **Fixed** |

No CRITICAL or HIGH finding was identified.

---

### F-01 — Control-character injection from a monitored host {#f-01}

**Severity:** Medium · **Status:** Fixed

`ProbeResult.error` is the one field a *remote* host writes into. The SMTP and
IMAP probes copy the peer's banner/greeting into it verbatim
(`smtp.rs`, `imap.rs` — `format!("unexpected banner: {}", banner.trim())`), and
`trim()` only removes surrounding whitespace, leaving interior control bytes
intact. That string then reached three sinks unfiltered.

**Reproduction.** A server answering port 2525 with

```
5xx \033[31mFAKE-DOWN\033[0m \033]0;PWNED\007 <img src=x onerror=alert(1)> q=" b=\ c=, e== end
```

is configured as an `smtp` probe target. Observed on the shipped image:

- **Console** (`main.rs` `print_result`, the default pretty format) printed the
  bytes raw — `cat -v` showed `^[[31m` and `^[]0;PWNED^G`, i.e. a live colour
  change and an OSC "set terminal title" sequence executing in the operator's
  terminal.
- **InfluxDB** stored the raw bytes; a Flux query returned them unchanged, so the
  payload **persists** and fires again in any consumer that prints the stored
  value later (`influx query` in a terminal, a log viewer, a dashboard).
- The `tracing` path was **not** affected: `scheduler.rs` logs `error = ?…`, and
  Rust's `Debug` for `str` already escapes control characters.

**Impact.** A compromised or malicious monitored host — exactly the thing huginn
is pointed at — can drive the operator's terminal: recolour output, move the
cursor, set the window title, and with `\r` overwrite the line just written to
forge or hide log lines. It is not code execution, but it corrupts the record an
operator relies on during an incident, and it is stored, not transient.

**Fix.** `ProbeResult::failure` now escapes the whole Unicode Cc range
(U+0000–U+001F, U+007F–U+009F): `\n`, `\r`, `\t` by name, the rest as `\xHH`.
It sits at the single point where every probe builds a failure, so all sinks are
covered at once rather than one guard per consumer; the existing per-sink
escaping (line protocol, exposition format, HTML) stays as a second layer.
C1 is included because some terminals decode it as the 8-bit form of the same
escapes.

**Verified.** Unit tests in `crates/huginn-core/src/types.rs`
(`failure_escapes_ansi_escapes_from_a_hostile_banner`,
`failure_escapes_newlines_and_carriage_returns`,
`failure_leaves_ordinary_errors_untouched`,
`escape_control_chars_covers_c0_c1_and_del`), plus the same live attack replayed
against the rebuilt image: the console now shows the literal text `\x1b[31m…`
and InfluxDB stores the escaped form.

---

### F-02 — No security response headers {#f-02}

**Severity:** Low · **Status:** Fixed

Neither listener sent `Content-Security-Policy`, `X-Content-Type-Options`,
`X-Frame-Options` or `Referrer-Policy`; responses carried only `content-type`,
`content-length` and `date`. `index.html` had no `<meta>` CSP either.

**Impact.** Defence in depth only — the UI's own escaping holds (see
[Checked](#checked-not-exploitable)) — but it is the layer that would contain a
future escaping mistake, and `nosniff` matters for `/metrics/latest` and
`/metrics`, which echo operator- and remote-supplied strings inside a non-HTML
content type.

**Fix.** New `crates/huginn-web/src/headers.rs`: an `axum::middleware::from_fn`
layer on both routers sending `default-src 'none'; script-src 'self'; style-src
'self'; connect-src 'self'; img-src 'none'; base-uri 'none'; form-action 'none';
frame-ancestors 'none'`, plus `nosniff`, `X-Frame-Options: DENY`,
`Referrer-Policy: no-referrer` and `Cache-Control: no-store`. Hand-rolled rather
than `tower-http` because a new direct dependency needs approval (AGENTS.md §3)
and this is a dozen lines of constants. The CSP omits `unsafe-inline` on
purpose — that is what makes an injected inline event handler inert.

**Verified.** Unit tests in `headers.rs` plus an end-to-end test
(`huginn/tests/debug_ui_test.rs::ui_responses_carry_the_security_headers`) that
asserts the headers on the wire for `/`, `/metrics/latest`, `/health` and
`/assets/app.js`; confirmed live on both listeners, including on the `401`.

---

### F-03 — No connection cap or request timeout {#f-03}

**Severity:** Low · **Status:** Mitigated and documented — not fixed in code

Both listeners are `axum::serve` with no timeout or concurrency layer. A client
that opens a socket and sends a partial request line holds the connection, and
its task, indefinitely.

**Measured** on the shipped image: 4 000 idle half-open connections were opened
without any being refused; container RSS rose from **29.5 MiB to 113.3 MiB**
(≈21 KiB per connection) and both listeners kept answering normally
(`/health` in 2 ms). Nothing observed caps the count, so growth continues until
memory runs out.

**Why it is not fixed in code.** A header-read timeout means either a
`tower-http` `TimeoutLayer` — a new direct dependency, which needs Joshua's
approval — or replacing `axum::serve` with a hand-rolled hyper accept loop, which
is a large change to make on the strength of a Low finding. **Open decision for
the maintainer**; recommendation is the `tower-http` layer (the crate is already
in the dependency tree transitively via reqwest).

**Mitigated by** `mem_limit: 256m` and `pids_limit: 128` in compose (the process
is restarted by `restart: unless-stopped` rather than taking the host with it),
and by both listeners binding loopback by default. Documented in
[`hardening.md`](hardening.md#no-request-limits-on-the-http-listeners).

---

### F-04 — Compose ran without container hardening {#f-04}

**Severity:** Low · **Status:** Fixed

`docker inspect` of the shipped compose stack showed `ReadonlyRootfs: false`,
`CapDrop: []`, `SecurityOpt: []`, `Memory: 0`, `PidsLimit: <nil>` — so the
container kept Docker's full default capability set and had no resource bound.
Both `9116` (the unauthenticated debug UI) and `8086` (InfluxDB, freshly
initialised with an admin token) were published on `0.0.0.0`.

Confirmed good already: `User: nonroot:nonroot`, `Privileged: false`, no secret
in the container environment (`Config.Env` holds only `HUGINN_UI_ENABLED`, `PATH`
and `SSL_CERT_FILE`), and the token reaching the process as a file.

**Fix.** `docker-compose.yml` now sets `read_only: true`, `cap_drop: [ALL]`,
`security_opt: [no-new-privileges:true]`, `mem_limit: 256m`, `pids_limit: 128`,
and publishes both ports on `127.0.0.1`. InfluxDB keeps a writable rootfs (it
has a data directory) but gains `no-new-privileges` and loopback publishing.

**Verified.** A container started with exactly those flags runs normally: probes
execute, both listeners serve, no permission error — huginn writes nothing to
its filesystem, so the read-only rootfs costs nothing.

---

### F-05 — An empty InfluxDB token file was accepted {#f-05}

**Severity:** Medium · **Status:** Fixed

`InfluxConfig::read_token` trimmed the file and returned whatever was left,
including the empty string — unlike `MetricsConfig::read_api_key`, which
already rejected an empty key file explicitly. The asymmetry contradicted the
fail-closed rule the docs state.

**Reproduction.** Started with a zero-byte token file against a reachable
InfluxDB: the process started with no error and no warning, sent
`Authorization: Token ` with no value, and InfluxDB answered `401`. Because
`401` is a 4xx, the writer classifies it as permanent and drops the batch:

```
ERROR huginn_influx::writer: InfluxDB rejected the write — discarding this batch …
       status: 401, body: {"code":"unauthorized","message":"unauthorized access"}
```

**Impact.** A monitor that looks healthy — probes run, the UI updates, `/metrics`
serves — while permanently discarding every measurement. A truncated secret
file, an empty Docker secret or a botched deploy produces silent, total data
loss, and the only signal is a per-batch error line in the log.

**Fix.** `read_token` now returns `HuginError::Secret { … "InfluxDB token file
is empty" }`, matching `read_api_key`. Startup aborts.

**Verified.** `read_token_rejects_an_empty_file` (empty and whitespace-only), and
live: the container now exits `1` with
`Secret file error at '/run/secrets/influx_token': InfluxDB token file is empty`.

---

### F-06 — The no-OpenSSL policy was not enforced {#f-06}

**Severity:** Low · **Status:** Fixed

"TLS is rustls only, no OpenSSL" is stated in AGENTS.md §6/§9 and
`hardening.md`, but `deny.toml` had an empty `[bans].deny` list — and the license
allow-list even permitted `OpenSSL`. The tree is rustls-only today purely
because every dependency happens to be configured that way; a transitive
dependency flipping a default feature would pull in a C TLS stack, another system
library to keep patched, and a build that breaks the distroless image, with CI
staying green throughout. `sources.unknown-git` was `warn`, so a git dependency
would only have scrolled past.

**Fix.** `[bans].deny` now lists `openssl`, `openssl-sys`, `native-tls` and
`tokio-native-tls`; `unknown-git` is `deny`; the unused `OpenSSL` license
allow-entry is removed, since allowing the license while banning the
implementation was a contradiction.

**Verified.** `cargo deny check` passes (`advisories ok, bans ok, licenses ok,
sources ok`), and the ban mechanism was proved to fire by running cargo-deny
against a throwaway copy of the config that also banned `ring` — a crate that
*is* in the tree — which failed as expected (`error[banned]: crate 'ring =
0.17.14' is explicitly banned`). The repository config was not modified for that
check.

---

## Checked, not exploitable

Hypotheses that were tested and did not hold. Recorded because "we looked" is
part of the result — and because a later change could turn any of these into a
real finding.

- **Stored XSS in the debug UI.** `app.js` assigns into `row.innerHTML`, which
  looked like the strongest candidate. It is not exploitable: every value passes
  through `escHtml()` (`& < > "`) and lands in element text context, and the row
  `id` is filtered to `[A-Za-z0-9_-]`. Verified live with `<img src=x
  onerror=alert(1)>` as both a probe name and a remote banner. `escHtml` does not
  escape `'`, which is safe today only because nothing is interpolated into an
  attribute — worth remembering before adding one. The CSP from F-02 now backs
  this up.
- **InfluxDB line-protocol injection.** A probe target of
  `bad"host\,x=1 y:9` (which passes validation — `has_port_suffix` only checks
  the part after the last `:`) round-tripped through the line protocol into
  InfluxDB as a single correct tag value. `escape_tag` escapes backslash first,
  which is what makes a trailing backslash safe; `escape_field_str` handles
  `"`, `\`, `\n`, `\r`.
- **Prometheus exposition injection.** Same payloads emerged as
  `target="bad\"host\\,x=1 y:9"` — structurally valid. Metric names from the
  per-probe `metrics` map are sanitised to `[a-zA-Z0-9_:]`.
- **`/metrics` authentication bypass.** Full matrix: no header, wrong key,
  correct key, and a lowercase `bearer` scheme — `401 / 401 / 200 / 401`, and no
  metric data appears in a 401 body. The comparison is constant-time. The
  lowercase rejection is a deviation from RFC 7235 (the scheme is
  case-insensitive) but is a client-compatibility nit, not a bypass.
- **Method and path handling.** `POST/PUT/DELETE/TRACE/OPTIONS/PATCH` → `405`;
  `/assets/../../etc/passwd`, `//`, `/health%00` → `404`. Routes are static, no
  filesystem is reachable.
- **Panic on hostile input.** Three `expect()` calls exist in non-test code
  (`queue.rs:117` behind a just-checked invariant, and the two reqwest client
  builders at startup); everything else is in test modules. Mutex poisoning is
  handled with `unwrap_or_else(|e| e.into_inner())`. No `unsafe` anywhere in the
  workspace.
- **Unbounded reads from a hostile peer.** SMTP/IMAP/UDP read into fixed 512-byte
  buffers; the HTTP probe never reads the response body (only `send()`, then the
  status); every await path is wrapped in `with_probe_timeout`.
- **Malformed certificates.** `x509-parser` returns a `Result` that the TLS probe
  propagates into a `ProbeResult::failure`; there is no unwrap on the parse.
- **YAML parser denial of service.** A billion-laughs config (nine levels of
  alias expansion) was loaded in a 512 MiB container: huginn started normally.
  The config file is also a trusted, read-only, operator-supplied mount.
- **Secrets in the image or environment.** `.dockerignore` excludes `secrets/`,
  so the builder stage's `COPY . .` cannot bake a token into a layer; the running
  container's environment holds no secret; the baked
  `config.example.yaml` contains only paths.
- **Workflow script injection.** Every `run:` block was reviewed. `auto-pr.yml`
  reads the branch name from `$GITHUB_REF_NAME` (an environment variable), not
  `${{ }}` interpolation; the only interpolations in shell bodies are
  `github.repository`, `runner.temp` and `vars.*`. No `pull_request_target`
  anywhere. All actions are pinned by 40-character commit SHA. `permissions:` are
  scoped per job, with `contents: write` limited to the jobs that tag or push.
- **Supply chain.** `cargo deny check` clean: no RustSec advisory, no banned
  crate, no non-crates.io source, licenses within the allow-list.

## Accepted risk

Recorded deliberately, not hidden — these are open by decision.

1. **The debug UI is unauthenticated.** `/`, `/events` and `/metrics/latest`
   serve the complete probe inventory — names, targets, error strings — to anyone
   who can reach the port. It is a debug UI, off by default, bound to loopback by
   default, and now published on `127.0.0.1` by compose. Note that
   `metrics.api_key_file` protects **only** the Prometheus listener: enabling it
   while the UI is exposed protects nothing, since the UI serves the same data.
2. **The TLS probe accepts invalid certificates.** Required — it exists to read
   expired and self-signed certificates. Narrowly scoped to one client, carries
   no secrets, trusts nothing from the peer. See
   [`hardening.md`](hardening.md#deliberate-exception-the-tls-probe-skips-certificate-verification).
3. **17 unfixable base-image CVEs stay visible** as open code-scanning alerts
   (Debian packages in `gcr.io/distroless/cc-debian12`, all LOW/MEDIUM, no
   patched version published upstream). They are accepted, monitored risk and are
   deliberately *not* filtered out of the Security tab — the open alert is the
   audit trail. Only fixable CRITICAL/HIGH findings block the pipeline.
4. **No request timeout on the HTTP listeners** — [F-03](#f-03), pending a
   decision on adding `tower-http`.

## Recommendations

Not done in this pass, in rough priority order:

1. Decide on [F-03](#f-03): add a `tower-http` timeout + concurrency-limit layer,
   or accept the container-limit mitigation as sufficient.
2. Consider warning at startup when a secret file is group- or world-readable.
   The docs prescribe mode `0600`; nothing checks it.
3. Consider a `HEALTHCHECK` in the `Dockerfile`. It needs a health subcommand on
   the binary — distroless has no shell or `curl` — so it is a small feature, not
   a one-liner.
4. Re-run this audit after any change to the probe result path, the HTTP
   listeners, or the container definition.

## Related

- [`hardening.md`](hardening.md) — the posture the findings changed
- [`risks.md`](risks.md) — where the accepted risks live now
- [`roadmap.md`](roadmap.md) — where the recommendations live now
- [`SECURITY.md`](SECURITY.md) — how to report the next one
