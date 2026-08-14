# Security audits

Full security reviews of huginn.io: adversarial testing against a running
instance plus a line-by-line code review, across the whole product rather than
the application code alone.

Each pass is a **point-in-time report** and stays here once written — a superseded
finding is still the record of what was true, and the passes read against each
other. Newest first. Practices live in [`hardening.md`](hardening.md); to report
a vulnerability see [`SECURITY.md`](SECURITY.md).

Finding IDs run continuously across passes, so `F-03` means one thing in this
document forever.

| Pass | Date | Findings |
|---|---|---|
| [2](#pass-2) | 2026-08-12 | F-07 … F-11, all Low |
| [1](#pass-1) | 2026-08-02 | F-01 … F-06, no CRITICAL or HIGH |

---

# Pass 2 — 2026-08-12 {#pass-2}

The repeat the first pass asked for. Its closing recommendation was to re-run
after any change to the probe result path, the HTTP listeners or the container
definition; all three had changed, across 58 files and roughly 5 500 inserted
lines.

The point of the repeat is not the volume. It is that several of the first
pass's conclusions had quietly stopped being true:

- It reviewed **two** listeners, both off by default. There are **three** now,
  and `health` is **on by default** (`crates/huginn-web/src/health.rs`, new).
- Its "no stored XSS" entry rests on the row `id` being filtered to
  `[A-Za-z0-9_-]`. That filter was removed when the debug UI stopped deriving
  DOM ids from probe names — the assurance had lost its own reasoning.
- The fixes for F-02 and F-03 (`headers.rs`, `serve.rs`) were written *in
  response* to that pass and so had never themselves been audited.
- The TLS probe now performs its own handshake with a custom `ServerCertVerifier`.
  The accepted-risk text describing it described code that no longer existed.

## Scope and method

| | |
|---|---|
| **Reviewed** | `cc5f574` (`dev`) — contains `v1.0.0` in full plus eight unreleased commits. Findings that also affect the published `v1.0.0` are marked **[shipped]** |
| **Toolchain** | rustc 1.94.0, cargo-deny 0.20.2, Docker 29.3.1 — **identical to pass 1**, so differences below are the code's and not the tools' |
| **Runtime tested** | `huginn:ci` built from this tree, run under `docker-compose.integration.yml` and, separately, under the hardening `docker-compose.yml` applies |
| **Out of scope** | InfluxDB itself, the host OS, the GitHub platform |

Method unchanged from pass 1, deliberately: per domain, form attack hypotheses
and **prove or disprove** them — dynamically where possible, by tracing the code
path otherwise. Disproved hypotheses are recorded
([below](#checked-2)), because a later change turns any of them into a real
finding. That is not theoretical here: it is exactly what happened to the stored-XSS
entry above.

Eight domains rather than seven. CI/CD was one bullet in pass 1 and is a domain
of its own now: the pipeline has since gained skopeo with registry credentials,
the digest recorded in the release tag, `RELEASE_PAT`, a GPG signing key, three
fan-in gates and stage chaining.

The attack setup: a hostile SMTP server whose banner carries ANSI SGR, an OSC
"set window title" sequence, HTML and line-protocol metacharacters; probe
**names** and **targets** seeded with the same payloads plus `__proto__`,
`constructor` and the `db.primary` / `db/primary` collision pair; a half-open
connection flood; and the full method/path/auth matrix against all three
listeners.

## Findings

| ID | Severity | Component | Finding | Status |
|---|---|---|---|---|
| [F-07](#f-07) | Low | `huginn-web`, `huginn-influx` | Control characters in operator-supplied probe names reach console, Prometheus labels and InfluxDB tags raw **[shipped]** | Open |
| [F-08](#f-08) | Low | `docker-compose.integration.yml` | The integration suite exercises the container **unhardened**, so no gate protects the hardening that ships **[shipped]** | Open |
| [F-09](#f-09) | Low | `huginn-web` | The connection cap trades memory exhaustion for denial of service on the flooded listener, and the residual is undocumented **[shipped]** | Open |
| [F-10](#f-10) | Low | repository settings | Nothing enforces SHA-pinning of actions; it is held by hand | Open |
| [F-11](#f-11) | Low | `ci.yml` | The `publish` job's security comment is false — third-party actions do run between the credentialed checkout and the tag push | Open |

**No CRITICAL or HIGH finding was identified.** Neither did pass 1; that is a
statement about two passes, not a guarantee about a third.

---

### F-07 — Control characters from configuration reach three sinks raw {#f-07}

**Severity:** Low · **Status:** Open · **[shipped]**

F-01's fix escapes the whole Unicode Cc range in `ProbeResult::failure`, which
is where every probe builds a failure — so the string a *remote* host writes is
covered at every sink at once. Nothing does the same for the strings the
*operator* writes: `probe.name` and `probe.target`. Name validation requires
non-empty and unique, and nothing else.

**Reproduction.** A probe configured as

```
name: "evil\x1b[31mNAME\x1b]0;PWNED\x07<img src=x onerror=alert(1)> q=\" b=\\ c=, e=="
```

against the image built from this tree:

| Sink | Result |
|---|---|
| Console — pretty formatter and the `tracing` line | **raw** |
| `/metrics` label values | **raw** — 3 exposition lines, 6 × ESC, 3 × BEL |
| InfluxDB tag values | **raw, and persisted** — 240 × ESC, 120 × BEL in a three-minute query result |
| `/metrics/latest` (JSON) | escaped on the wire — `serde_json` emits control characters as `\u001b`, per the JSON grammar |
| Debug UI | inert — `escHtml` output lands in element text context, where ESC and BEL do nothing |

`escape_label` and `escape_tag` are not at fault: both correctly escape what
their formats define as special (`"`, `\`, `,`, newlines), and the exposition
format and line protocol simply say nothing about C0. The same payload's quote
and backslash came back correctly escaped in the same lines.

**Impact.** The same shape as F-01, including persistence: anyone who `curl`s
`/metrics`, reads the container's logs, or prints a stored InfluxDB tag in a
terminal has the sequences executed — colour, cursor, window title, and with
`\r` the ability to overwrite the line just written and so forge or hide output.

**Why Low rather than Medium.** F-01's input was a monitored host — the untrusted
party huginn is pointed at. This input is the operator's own configuration file,
a trusted read-only mount. Whoever can write it already decides what huginn does,
so no privilege boundary is crossed. It is a defence-in-depth gap and an
inconsistency, not an escalation.

**Worth noting where it sits.** Pass 1 explicitly tested "Prometheus exposition
injection" and found it sound — with quote, backslash and comma payloads. The C0
range in a label value was not among them, which is how a checked domain still
had a gap.

---

### F-08 — The shipped hardening is exercised by no gate {#f-08}

**Severity:** Low · **Status:** Open · **[shipped]**

`docker-compose.yml` carries the full F-04 hardening. `docker-compose.integration.yml`
— the stack the system integration suite runs against, in CI and locally — carries
none of it.

**Reproduction.** `docker inspect` of the running integration stack:

```
ReadonlyRootfs: false     CapDrop: []        SecurityOpt: []
Memory:         0         PidsLimit: <nil>
```

against `docker-compose.yml`, which sets `read_only: true`, `cap_drop: [ALL]`,
`no-new-privileges:true`, `mem_limit: 256m` and `pids_limit: 128`. Confirmed
good in both: `User: nonroot:nonroot`, `Privileged: false`, loopback-only port
publishing, and no secret in `Config.Env`.

**Impact.** Every gate that claims to test the container tests the *unhardened*
one. A change that cannot survive a read-only rootfs — a temp file, a cache
directory — or that exceeds 128 processes or 256 MiB would pass CI and fail on
first deployment. The hardening is a documented part of the product
([`hardening.md`](hardening.md)) with nothing holding it up.

**The configuration itself is sound today**, which is what makes this a missing
gate rather than a broken setting. Verified by running the same image under
exactly the shipped flags: `read_only`, `cap_drop ALL`, `no-new-privileges`,
`--memory 256m`, `--pids-limit 128` → container `healthy`, 64 probe results,
writes accepted by InfluxDB, and **zero** filesystem or permission errors in the
log.

---

### F-09 — The connection cap trades memory for availability {#f-09}

**Severity:** Low · **Status:** Open · **[shipped]**

F-03's fix works, and its measured effect is large. Repeating pass 1's flood —
4 000 half-open connections, each sending a partial request head and then
nothing:

| | 2026-08-02 | 2026-08-12 |
|---|---|---|
| Container RSS | 29.5 → 113.3 MiB | 8.55 → **19.12 MiB** |
| Process tasks (PIDs) | — | 40 → **40** |
| Connections closed by the server | 0 | **3 894 of 4 000** |

The 256-permit cap and the 10-second header-read deadline both fire. What is not
written down anywhere is the other half of the trade: while the flood runs, the
**flooded listener stops serving**. Three of five legitimate requests to it timed
out at 8 s; the two that were served took 0.75 s and 2.8 s.

**The blast radius is narrow, and that is the important part.** The permits are
per listener. During a flood of the UI listener, measured over five samples
each: `metrics` answered in 3–4 ms and `health` in 0.3 ms, every time. The
container stayed `healthy` with `FailingStreak: 0`, and `huginn healthcheck`
exited 0 — so a flood of a published debug port cannot fail the HEALTHCHECK and
make an orchestrator restart a working monitor. That was the amplification worth
checking, and it does not exist.

**Impact.** An attacker who can reach a listener can deny that listener for the
length of the timeout window, repeatedly, at negligible cost. Both listeners are
off by default and bind loopback; the debug UI is unauthenticated by decision
([ADR-0009](adr/0009-debug-ui-stays-unauthenticated.md)). The process, the
probes and the InfluxDB writer are unaffected — measurement continues throughout.

**This is close to being an accepted risk rather than a finding**, and may become
one: bounding memory at the cost of latency on a debug surface is the right
trade. It is filed as a finding because nobody has been asked to accept it —
F-03's text presents the fix as complete and says nothing about what replaced the
memory growth.

---

### F-10 — SHA-pinning is practice, not policy {#f-10}

**Severity:** Low · **Status:** Open

`AGENTS.md` §9 and pass 1 both state that every action is pinned to a
40-character commit SHA, because a tag is movable and a compromised upstream
would otherwise reach CI with no Dependabot PR. **It is true today** — all 13
distinct `uses:` references across the six workflows are SHA-40 pinned, verified
mechanically.

Nothing enforces it. The repository's Actions settings report:

```
allowed_actions: "all"      sha_pinning_required: false
```

and no gate covers the gap either: actionlint has no such rule, Semgrep's
`p/rust` and `p/secrets` do not look at `uses:`, and Trivy scans images. The next
workflow edit may write `actions/checkout@v5` and every check will stay green.

**Impact.** Latent. It is the exact shape of F-06 — a policy that holds only
because everyone has so far remembered it — and GitHub now offers the switch that
would make it structural.

---

### F-11 — `publish`'s security comment is false {#f-11}

**Severity:** Low · **Status:** Open

`ci.yml`'s `publish` job is the one checkout in that file that keeps its git
credentials, because its last step runs `git push origin "$TAG"` with
`RELEASE_PAT`. The comment justifying that reads:

> Nothing between here and there executes third-party code — no cargo, no build
> — so the token stays inside a job that only runs skopeo, jq and git.

Four third-party actions run between that checkout and the tag push:
`actions/download-artifact`, `docker/metadata-action`,
`docker/setup-buildx-action` and `docker/login-action`. Each executes with the
workspace — including the `.git/config` holding the credential — in reach.

**Impact.** Small in practice: all four are SHA-pinned, and two of them are
GitHub's and Docker's own. The finding is the **claim**, not the exposure. It is
written where a maintainer goes to decide whether a change to this job is safe,
and it tells them a job property that does not hold — so the next action added
here will be added on the strength of reasoning that was already wrong.

The other two credentialed checkouts in the repository (`release-dispatch.yml`'s
`release`, `release.yml`'s `prepare-dev`) were checked for the same thing: both
run only shell steps after checkout, so for those the claim does hold.

## Checked, not exploitable {#checked-2}

Tested and did not hold. Recorded because a later change turns any of these into
a finding — see what happened to pass 1's stored-XSS entry.

- **F-01 still holds for remote input.** The hostile 5xx banner reaches the
  console as the literal text `\x1b[31m…`. The `^[` sequences visible in the same
  log lines are huginn's own formatter colours, not the attacker's.
- **Stored XSS in the debug UI, re-tested against the rewritten `app.js`** rather
  than inherited. `escHtml` still covers `& < > "` and still does not cover `'`,
  which remains safe only because nothing is interpolated into an attribute. The
  removed DOM-id derivation cannot be a collision source any more because there is
  no derived id: rows are held in a `Map` keyed on the raw name. `__proto__` and
  `constructor` were configured as probe names — a `Map` is not an object, so
  neither reaches a prototype. The `db.primary` / `db/primary` pair produced two
  rows.
- **The `health` listener discloses nothing.** It serves the two bytes `OK` and
  nothing else. `GET /health` → 200; `POST /health` → 405; `/`, `/metrics`,
  `/events`, `/health/../etc/passwd` → 404. It is not reachable from the host at
  all (no published port, and it has no `bind` key to widen), and was reached for
  this test only by entering the container's network namespace.
- **Security headers on every listener and every status.** CSP,
  `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy` and
  `Cache-Control: no-store` are present on `ui` 200/404, on `metrics` 200 **and
  401**, on `/assets/app.js`, and on `health`.
- **`/metrics` authentication.** No header, wrong key, lowercase `bearer`, `Basic`
  scheme, key as a query parameter, a one-character-short prefix and the key with
  one character appended → `401` for all seven; the exact `Bearer <key>` → `200`.
  The 401 body is 13 bytes and contains no metric data. The lowercase rejection
  remains a deviation from RFC 7235 and remains a client-compatibility nit.
- **Method and path handling.** `POST/PUT/DELETE/PATCH/OPTIONS/TRACE` → 405 on
  both `ui` and `metrics`; `/assets/../../etc/passwd`, `//`, `/health%00`,
  `/../Dockerfile` and a percent-encoded traversal → 404.
- **The TLS probe's dangerous verifier is contained.** `ReadOnlyCertVerifier` is
  module-private with no `pub` export, constructed once (`tls.rs:199`), on a
  config built `with_no_client_auth()` — it presents no credential. The other two
  TLS clients in the workspace (`huginn-influx`'s writer, the HTTP probe) are
  `reqwest::Client::builder()` with default verification, and
  `danger_accept_invalid_certs` appears nowhere in the tree. Pass 1's
  accepted-risk wording therefore still describes the rewritten code.
- **Secret files are fail-closed.** Empty → refuses, naming the reason.
  Whitespace-only → refuses. Missing → refuses. Each stops startup and prints
  the path and cause without printing content.
- **A malformed token does not lose data silently** — the failure F-05 was about.
  With a token containing a NUL byte, huginn starts, and every write fails as a
  **retryable** transport error rather than a permanent rejection, so batches
  queue instead of being discarded: `queue_batches 70`,
  `batches_rejected_total 0`, `batches_dropped_total 0`,
  `last_write_success_timestamp_seconds 0`. That is precisely the signature the
  write-path metrics were added to produce.
- **Strict config loading rejects and explains.** An invented key was refused at
  startup with the offending key named, the valid alternatives listed, and the
  line and column given.
- **Workflow script injection.** Only seven `${{ }}` interpolations exist inside
  any `run:` block across the six workflows, and they are `github.repository`
  (once) and `runner.temp` (six times). Everything attacker-adjacent —
  `vars.*`, `secrets.*`, step outputs, `github.actor` — reaches the shell through
  `env:`, which is the correct pattern and an improvement on pass 1, when
  `vars.*` still appeared in shell bodies. No `pull_request_target` anywhere.
- **Credential reach.** Every secret is confined to a `push`-only job
  (`ci.yml`'s `push` and `publish`, both `if: github.event_name == 'push'`) or to
  a release workflow. Fourteen of the seventeen checkouts set
  `persist-credentials: false`; the three that do not are the three that push
  (see F-11). `dependabot-auto-merge.yml` declares `contents: write` on an
  `on: pull_request` trigger, which a fork cannot obtain — GitHub caps a fork
  PR's token at read regardless of the declared permissions — and its job is
  additionally gated on the PR author being `dependabot[bot]`.
- **A hand-pushed tag cannot publish.** `ci.yml` does not run on tags at all.
  `release.yml` does, and refuses a tag whose commit is not an ancestor of
  `main`; it then compares the `image-digest:` recorded in the tag's annotation
  against what the registry currently serves and exits 1 on a mismatch, on an
  unresolvable image, and only degrades to `verified=no` for tags predating the
  mechanism.
- **Supply chain.** `cargo deny check` → `advisories ok, bans ok, licenses ok,
  sources ok`. The ban mechanism was proved to fire, as in pass 1, by additionally
  banning `ring` — a crate that *is* in the tree — in a throwaway copy of the
  config: `error[banned]: crate 'ring = 0.17.14' is explicitly banned`. The
  repository's `deny.toml` was not modified (`git diff` empty).
- **The four direct dependencies added since pass 1 added nothing to the tree.**
  `rustls` 0.23.43, `tokio-rustls` 0.26.4, `hyper-util` 0.1.20 and `x509-parser`
  0.18.1 are present at **identical versions** in `Cargo.lock` at pass 1's
  commit — the claim that promoting them to direct dependencies cost no
  supply-chain surface is exact.
- **The two pins Dependabot cannot update are current.** `semgrep/semgrep:1.172.0`
  and `rhysd/actionlint:1.7.12` are each the latest upstream release as of
  2026-08-12 (published 2026-07-28 and 2026-03-30). The manual-bump risk is real
  and has not yet materialised.

## Accepted risk

Unchanged in substance from pass 1, re-confirmed against the current code.

1. **The debug UI is unauthenticated.** Now a recorded decision rather than an
   open question — [ADR-0009](adr/0009-debug-ui-stays-unauthenticated.md).
   `metrics.api_key_file` still protects only the Prometheus listener.
2. **The TLS probe accepts invalid certificates.** Required, and re-verified
   against the rewritten implementation above rather than carried forward.
3. **Base-image CVEs stay visible as open alerts.** Recounted on 2026-08-12:
   **19 total — 0 CRITICAL, 0 HIGH, 6 MEDIUM, 13 LOW, and none with a published
   fix.** Pass 1 counted 17 on 2026-08-02. The blocking gate is fixable
   CRITICAL/HIGH, which is empty.

## Recommendations

In rough priority order.

1. **Extend the Cc escaping to configuration-supplied strings**, or reject
   control characters in probe names and targets at config load. Rejecting is
   more in keeping with the rest of 1.0's strict loading: a name nobody can
   safely display is a name worth refusing (F-07).
2. **Give the integration stack the hardening the shipped stack has**, so the
   suite tests what ships (F-08).
3. **Turn on `sha_pinning_required`** in the repository's Actions settings, and
   consider narrowing `allowed_actions` (F-10).
4. **Correct or remove the `publish` comment**, or better, set
   `persist-credentials: false` there too and hand the token only to the final
   `git push` step — which would make the claim true instead of documenting that
   it is not (F-11).
5. **Decide on F-09** and record the outcome, either as an accepted risk in
   [`risks.md`](risks.md) or as a change.
6. **Validate secret files as legal HTTP header values at load**, so a token
   containing a control byte is refused at startup rather than after the queue
   has filled. Nothing is lost silently today, so this is hardening, not a fix.
7. **Turn off "Allow GitHub Actions to create and approve pull requests"**
   (`can_approve_pull_request_reviews: true`). It grants nothing today, because
   the rulesets require zero approving reviews — which is exactly why removing it
   costs nothing and closes a path that would matter the day that changes.
8. **Re-run this audit** after any change to the probe result path, the HTTP
   listeners, or the container definition. Unchanged from pass 1, and it was the
   right advice: every one of those had changed, and each had invalidated
   something.

---

# Pass 1 — 2026-08-02 {#pass-1}

The pre-release review. Kept in full: its findings are the record of what was
true, and pass 2 above is largely a conversation with it.

**What of it still stands, as of pass 2:** all six fixes hold. F-01's escaping
covers every remote path tested; F-02's headers are on all three listeners;
F-03's limits are measured above; F-05's fail-closed behaviour was re-tested
across four bad-file cases; F-06's bans were proved to fire. Two of its
"checked, not exploitable" entries are superseded — the stored-XSS entry rested
on a filter that no longer exists (re-tested and still sound, for different
reasons), and the `expect()` count has changed. F-04's fix reached the shipped
compose file but not the integration one, which is F-08.

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
| [F-03](#f-03) | Low | `huginn-web` | No connection cap or request timeout on either listener | **Fixed** — 256-connection cap and a 10 s header-read timeout |
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

**Severity:** Low · **Status:** Fixed

Both listeners were `axum::serve` with no timeout or concurrency layer. A client
that opened a socket and sent a partial request line held the connection, and
its task, indefinitely.

**Measured** on the shipped image at the time: 4 000 idle half-open connections
were opened without any being refused; container RSS rose from **29.5 MiB to
113.3 MiB** (≈21 KiB per connection) and both listeners kept answering normally
(`/health` in 2 ms). Nothing observed capped the count.

**Fix.** A 256-connection cap per listener and a 10-second header-read timeout,
in `crates/huginn-web/src/serve.rs`.

**Not the fix this report originally recommended**, and the difference is worth
keeping: it suggested a `tower-http` `TimeoutLayer`, which would not have
worked. A `tower` layer wraps the *service*, and the service is not reached
until hyper has parsed a request — a request head that never completes never
arrives, so the layer never runs. Both limits had to sit below the service: the
cap is a semaphore permit taken before `accept`, so peers wait in the kernel
backlog rather than each costing a task, and the deadline is hyper's own
`header_read_timeout`. It bounds the head and not the connection, so the SSE
stream on `/events` is unaffected.

`hyper-util` became a direct dependency and added nothing to the supply chain —
it was already in the tree under axum.

**Verified.** Unit tests in `serve.rs`: a half-open connection is closed by the
server (`read` returns 0), and a well-formed request is still answered.

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

## Recommendations

Not done in this pass, in rough priority order:

1. ~~Decide on [F-03](#f-03)~~ — **done**, and not the way this recommendation
   suggested; the finding above records why.
2. ~~Consider warning at startup when a secret file is group- or world-readable~~
   — **done**. Raised again as M-01 of muninn.io's own review, where the same
   gap is sharper because that image carries a shell; the fix landed in both.
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
