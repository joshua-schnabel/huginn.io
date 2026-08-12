# CI/CD and repository setup

The branch model, the pipeline's shape, and the repository configuration it
depends on. Each workflow explained one by one is
[`workflows.md`](workflows.md); cutting a release is
[`releasing.md`](releasing.md).

## Branch model

```text
feature/my-feature
       │  pull request (squash)
       ▼
      dev  ──────────────── push → :dev + :x.y.z-dev
       │  pull request (merge commit)
       ▼
      main ──────────────── push → :latest + :x.y.z, then tag vX.Y.Z
```

| Branch | Purpose | Protected |
|---|---|:---:|
| `main` | Releases | yes |
| `dev` | Integration | yes |
| `feature/*` `fix/*` `chore/*` `docs/*` `test/*` | Work | no |

No direct pushes to `main` or `dev`. Everything goes through a pull request,
including Dependabot's — which is why every ecosystem sets `target-branch: dev`.
A bump merged straight into `main` is a change `dev` never saw, so the next
release PR would silently revert it.

## Pipeline shape

```text
── Source ────────────────────────────────────────────────────────────────
  check ─┬ test (stable + beta·canary) ─┐
         ├ supply-chain ────────────────┤
         ├ coverage ────────────────────┤
         └ version-gate ────────────────┤
                                        ▼
                                   SOURCE GATE
── Image ───────────────────────────────┼─────────────────────────────────
                                        ▼
             build (per arch, native) → image.tar artefact
                ├ scan  (Trivy + SBOM, on the artefact)
                └ integration (load + compose, on the artefact)
                                        ▼
                                    IMAGE GATE
── Release (push events only) ──────────┼─────────────────────────────────
                                        ▼
             push (per arch) → skopeo by digest → digest artefact
                                        ▼
             publish → manifest, ghcr mirror, staging cleanup, git tag
                                        ▼
                              release.yml (Release + SBOM + housekeeping PR)
```

**The gates are the stage boundaries**, not a verdict beside them: `build`
depends on `source-gate` and `push` on `image-gate`, so each stage's membership
is written down exactly once, in that gate's `needs`. `build` used to re-list
the five source jobs, which made the gate's list a second copy of the same set
— and a job wired into only one of the two would either block the merge after
the build had already started, or gate nothing at all.

**The image is built exactly once per architecture** and uploaded as a tarball.
`scan`, `integration` and `push` all consume *that same artefact*, so the bytes
scanned, tested and published are byte-identical. `push` reaches a registry only
behind `image-gate`, which covers `scan` **and** `integration`, and `publish`
needs `push` — so nothing unscanned can be pushed.

**`coverage` hangs off `check`, not off `test`**, so it runs alongside the two
toolchain legs. `cargo llvm-cov --all` runs the whole suite itself, instrumented,
so waiting for `test` bought no extra confidence — only the duration of a full
suite on the critical path, part of it spent waiting on the beta leg, which
`continue-on-error` forbids from blocking anything. The cost is a coverage run
that is also spent on a red push; both jobs are still in `source-gate`, so
neither stops blocking.

Job display names carry a `Source ·` / `Image ·` / `Release ·` / `Security ·`
prefix. GitHub has no stages of its own — the run graph is drawn from `needs`
alone — so the prefixes are what makes the job list group by stage, and each
gate writes its members and their results to the run summary. The three **gate**
names deliberately carry no prefix; see "Repository settings" below.

CI runs on **every** pull request, not only those targeting `main` or `dev`. It
was once scoped to those two, so feature branches had no gate — which is how
`feature/refactoring` and `dev` diverged into two different "CI fixes", one of
which silently dropped the keep-alive in `run()` and left the daemon exiting at
startup.

**Publish lives in `ci.yml` on purpose.** As its own workflow it triggered on
`push` in parallel with CI and depended on nothing, so a commit with failing
tests still shipped `:latest` — every documented gate was bypassable on the only
path that reaches users. A `workflow_run` trigger would fix the ordering and
introduce a subtler trap: `github.ref` there resolves to the default branch, so
every `github.ref == 'refs/heads/dev'` check silently reads false. A job with
`needs` has neither problem.

## Jobs per trigger

| Job | any PR | push dev | push main | push tag `v*` |
|---|:---:|:---:|:---:|:---:|
| Source · Format & Lint | yes | yes | yes | — |
| Source · Tests (stable) | yes 🚫 | yes | yes | — |
| Source · Tests (beta, canary) | yes | yes | yes | — |
| Source · Supply-chain (cargo-deny) | yes 🚫 | yes | yes | — |
| Source · Coverage ≥ 80 % | yes 🚫 | yes | yes | — |
| Source · Version gate | yes 🚫† | ➖ | yes 🚫 | — |
| Security · Semgrep · ShellCheck · Actionlint | yes 🚫* | yes | yes | — |
| Image · Build (per arch, native) | yes 🚫 | yes | yes | — |
| Image · Trivy SARIF + SBOM | yes | yes | yes | — |
| Image · Trivy blocking scan | yes 🚫 | yes 🚫 | yes 🚫 | — |
| Image · System integration test | yes 🚫 | yes | yes | — |
| Release · Publish → Docker Hub + ghcr | — | `:dev` + `:x.y.z-dev` | `:latest` + `:x.y.z` | — |
| GitHub Release + housekeeping PR | — | — | — | yes |

🚫 blocks · 🚫* Semgrep blocks only on ERROR-severity findings; ShellCheck and
actionlint block on any finding · 🚫† enforced only on a PR whose base is `main`
· ➖ runs as a deliberate no-op rather than skipping, because `Source gate`
counts anything that is not `success` as a failure.

**Everything marked 🚫 blocks through one of the three gates**, never by being
named in the ruleset itself — "Repository settings" below says why, and which
two jobs are deliberately in no gate.

**`ci.yml` does not run on tags at all** — the whole `v*` column is
`release.yml`'s. It used to run there, and that is what made every release build
twice: `publish` pushes the tag, the tag started `ci.yml` again, and the second
run republished `:x.y.z` from a rebuild while `release.yml` was already writing
notes and an SBOM about the first. The tag is the pipeline's output. Whatever
`main` built, scanned and integration-tested is what ships, and `release.yml`
checks the tag's recorded digest against the registry before it publishes
anything.

## What can reach a credential

Two shapes are worth knowing when editing a workflow here.

**No job runs `cargo` with a write token.** `actions/checkout` persists its token
into `.git/config`, and `cargo` compiles and executes `build.rs` and proc-macros
from every dependency in the tree. Every checkout therefore sets
`persist-credentials: false` except the three that push — `ci.yml`'s `publish`,
`release.yml`'s `prepare-dev` and `release-dispatch.yml` — none of which runs
cargo. `release.yml` is split into `test-report` (`contents: read`, runs cargo,
uploads an artefact) and `github-release` (`contents: write`, downloads it) for
exactly this reason. If you add a cargo step, it belongs in the first job.

**Credentials go through stdin or a file, never argv.** `/proc/<pid>/cmdline` is
readable by every process on the runner. `skopeo login --password-stdin` with
`REGISTRY_AUTH_FILE` replaces `--dest-creds`; `jq -n --arg` piped into
`curl --data @-` replaces `-d "{…$TOKEN…}"`; `curl -K -` replaces
`-H "Authorization: …"`.

**Permissions are least-privilege per job.** The default is `contents: read`.
`publish` needs `contents: write` for the tag and `packages: write` for the ghcr
mirror; the Trivy and Semgrep jobs need `security-events: write` for the SARIF
upload, and in `security.yml` that permission is scoped to the Semgrep job alone
— ShellCheck and actionlint only read the tree.

## Source scanning

| Tool | Layer | Finds | Blocks |
|---|---|---|---|
| `cargo deny` | dependencies | RustSec advisories, licences, banned crates, unknown registries | every PR and push |
| Semgrep `p/rust` · `p/secrets` | source | unsafe patterns, taint flows, hardcoded secrets | ERROR severity |
| ShellCheck | `scripts/*.sh` | quoting and expansion bugs | severity ≥ warning |
| actionlint | workflows | unknown inputs, bad `needs`, shell errors in `run:` | any finding |
| Trivy | image | OS and library CVEs | fixable CRITICAL/HIGH |

Semgrep runs twice: a full pass to SARIF that never blocks, and a blocking pass
on ERROR severity. GitHub code scanning ignores SARIF's `suppressions` property,
so a `nosemgrep`-suppressed finding would stay open in the Security tab forever —
the upload therefore strips suppressed results, and the in-code
`// nosemgrep: <rule>` comment with its reasoning is the single source of truth
for an accepted finding.

**`deny.toml` must stay parseable, not merely present.** `severity-threshold`,
`unlicensed` and `copyleft` were removed in cargo-deny ≥ 0.14 and CI installs the
latest release — so the config failed to *load*, and the supply-chain gate was
not running at all while reporting nothing. Repairing it immediately surfaced
four real advisories. If you edit it, run `cargo audit-all` locally: a config
error and a clean scan look very different, and only one of them is good news.

### Suppressed image findings

`scan` blocks on **fixable** CRITICAL/HIGH only. A gate that blocks on findings
nobody can act on is a gate that gets switched off — and the distroless base
carries 17 unfixable LOW/MEDIUM CVEs that stay visible as open alerts on purpose
([R4](risks.md)).

`.trivyignore.yaml` is read by the blocking scan only, so a suppressed finding
still reaches the Security tab. It is empty, and its rules are written down for
when it is not: every entry carries `expired_at`, and every entry argues why the
vulnerable code cannot be reached in huginn's deployment.

## Architectures

`linux/amd64` and `linux/arm64`, each built and integration-tested on its
**native** runner (`ubuntu-latest` and `ubuntu-24.04-arm`). No QEMU — emulation
had made the arm64 build the pipeline's dominant cost.

`publish` assembles the multi-arch manifest from the two digests and mirrors it
to ghcr with `skopeo copy --all`, which copies the manifest list and every blob,
so both registries carry byte-identical images from one build.

## Releasing

The full runbook, both paths, is [`releasing.md`](releasing.md). The shape:

1. The version comes from `CHANGELOG.md`. The **version gate** enforces valid
   SemVer, agreement with `Cargo.toml`'s workspace version, and strictly greater
   than the last `v*` tag, on any PR whose base is `main`.
2. Merging into `main` publishes the image and creates the tag `vX.Y.Z` — no new
   bytes, only tags on already-scanned digests. The tag is annotated with the
   manifest digest.
3. The tag push starts `release.yml` — and **only** `release.yml`; `ci.yml`
   ignores tags, so nothing rebuilds. It verifies the recorded digest against the
   registry, then creates the GitHub Release, SBOM, test report, and an
   auto-merging housekeeping PR into `dev`.

**Never hand-push a `v*` tag.** The pipeline creates them after every gate.

### Why the tag is pushed with `RELEASE_PAT`

GitHub does not start a workflow from an event that `GITHUB_TOKEN` created — the
recursion guard. A tag pushed with the built-in token fires nothing, so
`release.yml` never runs: the image, the mirror and the tag ship, and the
Release, the SBOM, the test report and the housekeeping PR do not. muninn.io hit
exactly this at its v0.1.0. The tag push uses `RELEASE_PAT` where available and
falls back to `GITHUB_TOKEN`, which still creates the tag — you then finish the
release by hand.

### Driving `release.yml` by hand

`release.yml` has a `workflow_dispatch` entry point taking an existing tag. It
does the same work the tag push would have done and creates nothing twice:
`gh release view` makes creation idempotent, uploads use `--clobber`, and every
step reads the tag rather than the branch the button was pressed on.

## Repository settings — maintainer, by hand

Deliberately not automated. Changing repository settings, secrets or rulesets is
outside what an agent does here (`AGENTS.md` §3), so this is the checklist.

**Branch protection is two rulesets** (Settings → Rules → Rulesets), not classic
branch protection: one on `refs/heads/dev`, one on `refs/heads/main`. Neither has
a bypass actor, so the maintainer goes through a pull request as well.

Both rulesets:

- require a pull request before merging — with no approving review, this being a
  solo project, and a reviewer nobody can be would block every merge;
- require exactly **three** status checks, each a fan-in job that runs no build
  and no test of its own:
  - `Source gate` — covers `check`, `test`, `supply-chain`, `coverage`, `version-gate`
  - `Image gate` — covers `build`, `scan`, `integration`
  - `Security gate` — covers `shellcheck`, `actionlint`, `semgrep`
- require branches to be up to date before merging;
- require signed commits;
- disallow force pushes and deletion.

They differ in one rule: `dev` permits a squash merge, `main` only a merge
commit, per the branch model above.

Those three membership lists are a reader's aid. The authority is each gate's
`needs` in the workflow, and that is the only copy that can be wrong without
anybody noticing — which is the whole point of the arrangement.

### Picking the three names

GitHub's name field suggests every check it has **seen recently** on the
repository, not what the workflows produce today. The list therefore mixes three
kinds of entry and marks none of them:

- **the three gates** — the only ones to pick;
- **stage members**, recognisable by the `·` in the name: `Source · Version
  gate`, `Source · Format & Lint`, `Image · Trivy scan linux/amd64`. These are
  real and would report — and adding one is still wrong. It is already covered
  by its gate, and a second required list maintained by hand is precisely the
  failure the gates removed;
- **names that no longer exist**: `Version gate`, `Format & Lint`, `Trivy scan
  linux/amd64` and the rest of the pre-stage naming, still offered because they
  ran here recently. Picking one blocks every pull request indefinitely, because
  it never reports again.

`version-gate` is the trap in that list. It is named "gate" and is **not** one of
the three: it is an ordinary source-stage job that validates the release
version, and `Source gate` covers it. Anyone counting gates by name counts four.

Old names age out of the picker by themselves once they leave GitHub's recent
history. Until then, ignore everything that is not one of the three.

Take the entries attributed to **GitHub Actions**. The picker shows the app
beside each name and the ruleset stores it (`integration_id`), which pins the
requirement to the workflow meant to satisfy it — Semgrep and Trivy post check
runs from their own apps, so name alone is not unique.

**Verify the result instead of trusting the click-through.** Both rulesets must
come back with the same three lines:

```bash
gh api repos/joshua-schnabel/huginn.io/rulesets -q '.[].id' | while read -r id; do
  gh api "repos/joshua-schnabel/huginn.io/rulesets/$id" \
    -q '"\(.name):", (.rules[]
        | select(.type == "required_status_checks")
        | .parameters.required_status_checks[] | "  \(.context)")'
done
```

### Why three names instead of nine

A check that is not in the required set is an *indicator*: it runs, it goes
visibly red, and it stops nobody. Keeping that set in step with `ci.yml` by hand
is the failure this replaces, and it had already happened here — `ShellCheck`
and `Actionlint` ran on every push and pull request for several releases while a
PR could merge straight past them, and the fix was recorded in this very file as
a one-line ruleset change that nobody made. The version gate was in the same
position and merely got away with it, because `build` happened to list it in
`needs`.

Each gate derives its verdict from its own `needs` — the same list the pipeline
must maintain anyway to order itself. A job added to `needs` is covered the
moment it is added, rather than the moment somebody remembers to edit a
repository setting that is invisible from the code.

Four consequences worth knowing before anyone changes this:

- **The three gate names are a fixed surface.** `build` and `push` now depend on
  their gate, and the ruleset names it, so the string appears in two places that
  cannot see each other. Rename a required check and it never reports again —
  and a check that never reports blocks every pull request indefinitely. This is
  why the gates alone carry no `Source ·` / `Image ·` / `Security ·` prefix.
- **`if: always()` on each gate is load-bearing.** Without it, a gate whose
  dependency failed is *skipped* rather than failed — and GitHub counts a
  skipped required check as satisfied. The gate would be green by absence in
  exactly the case it exists for. It is also what makes the gate safe to depend
  on: a job that can never be skipped always resolves to a real verdict, so
  `build` and `push` are skipped precisely when their stage did not pass.
- **`Tests (beta)` still does not block.** Its leg carries `continue-on-error`,
  so it reports `success` to `needs` even when it fails. It stays a canary.
- **`push` and `publish` are deliberately in no gate.** Both are `push`-only, so
  on a pull request they report `skipped`, and the gates treat anything that is
  not `success` as a failure.

> **Changing the ruleset is a manual step, and its order matters.** The gate
> jobs must exist on the default branch and have reported once before they are
> made required — a required check that never reports blocks every pull request
> indefinitely. Merge first, then swap the required set.

**If a gate ever has to be renamed**, that same rule turns it into three moves
rather than an edit: land the rename so the new name reports once, add the new
name to the required set *beside* the old one, then remove the old. Done as a
single edit it leaves a window in which the required name is one that no run
produces, and the repository locks itself. The cheaper answer is not to rename
them at all, which is why the gates carry no stage prefix and both workflows say
so at the job.

**The image jobs are required, deliberately.** `Image gate` waits on `build`, so
a documentation-only PR waits for two container builds, and an advisory
published that morning against something in the image blocks a branch that never
touched the image — that happened on 2026-08-06. The decision is that this is
the right way round: a finding that blocks is a finding someone looks at, and
the alternative lets a fixable CRITICAL reach `dev` and be caught one step
later, at `publish`.

**Everything else that runs, blocks.** `ShellCheck` and `Actionlint` block
through `Security gate`; the version gate blocks through `Source gate` rather
than by being listed in `build`'s `needs`. That transitive route is gone on
purpose — `build` names only the gate now, so there is one list of the source
stage instead of two that can drift.

**Enable "Allow auto-merge"** (Settings → General → Pull Requests). Both
`dependabot-auto-merge.yml` and the release housekeeping PR queue their merges
with `gh pr merge --auto`, which does nothing without it.

**Enable both "Allow merge commits" and "Allow squash merging"** (same page).
The branch model uses one of each: `feature/* → dev` squashes, `dev → main`
keeps a merge commit, and `release-dispatch.yml` asks for `--merge` explicitly.
With merge commits disabled the release PR is opened but auto-merge warns and
you merge it by hand.

**Enable "Allow GitHub Actions to create and approve pull requests"** (Settings →
Actions → General → Workflow permissions). Without it the API refuses with *"GitHub
Actions is not permitted to create or approve pull requests"*, and `auto-pr.yml`
cannot open its draft PR, nor can the post-release housekeeping PR be opened.
Both warn instead of failing — the branch is pushed either way — but you then
open the PR yourself. The setting is off by default on new repositories.

**After the first publish**, set the `huginn` package in GitHub → Packages to
**Public**; it defaults to private, and `docker pull ghcr.io/…` would need auth.

**Repository variables**

| Name | Value | Needed for |
|---|---|---|
| `DOCKERHUB_USERNAME` | the Docker Hub account owning `<user>/huginn` | `push`, `publish`, the ghcr mirror |

**Secrets**

| Name | Needed for | Consequence if absent |
|---|---|---|
| `DOCKERHUB_TOKEN` | pushing the image | `push` fails with a message naming it; nothing is published |
| `RELEASE_PAT` | the release tag push, the release-dispatch PR, the housekeeping PR | **`release.yml` never runs** — see above. The PRs are still opened, but CI does not trigger on them and auto-merge hangs |

| `GPG_PRIVATE_KEY` | signing the commits `release.yml` and `release-dispatch.yml` create | those jobs fail with a message naming the secret. Without it the commits are unsigned, and both branch rulesets carry `required_signatures` — the pull request opens, every check goes green, and the merge button stays blocked with nothing failing |
| `GPG_PASSPHRASE` | only if the signing key has one | gpg cannot use the key |

**The signing key.** Generate one **for CI**, not a copy of a personal key: if it
leaks, revoking a dedicated key costs nothing, and a personal one costs
everything it ever signed. No passphrase is the normal choice — the private key
is already a secret, and a passphrase beside it in the same secret store adds no
layer.

```bash
gpg --batch --quick-generate-key "<name> <email>" ed25519 sign never
gpg --armor --export-secret-keys <email>   # -> the GPG_PRIVATE_KEY secret
gpg --armor --export <email>               # -> GitHub, Settings -> SSH and GPG keys
```

The email has to be **verified on the GitHub account** that holds the public key,
or the signature is valid and GitHub still labels the commit *Unverified*. The
workflows read the name and email out of the key's UID rather than hard-coding
them, so whichever identity you pick is the one that signs — but that identity
and the account must agree.

`GITHUB_TOKEN` is built in and needs no setup. It carries the ghcr mirror
(`packages: write`), the git tag when `RELEASE_PAT` is absent, and the SARIF
uploads (`security-events: write`).

Use a Docker Hub **access token**, not the account password, scoped to
Read/Write/**Delete** on this repository. Delete is required: `publish` removes
the two `staging-*` tags once the multi-arch manifest points at their digests.

## Dependencies

Dependabot opens one grouped PR per ecosystem — cargo, github-actions, docker —
weekly, against `dev`, with a 3-day cooldown. `dependabot-auto-merge.yml`
auto-merges patch and minor bumps; a group containing a major waits for review,
so you review exactly when a breaking bump is present. Security updates are not
grouped and not delayed by the cooldown.

Dependabot does **not** update `container:` or `docker run` references
(dependabot-core#5819). The Semgrep and actionlint images in `security.yml` are
pinned by digest and need a manual bump; both say so at the pin.

## Local equivalents

```bash
cargo fmt-check && cargo lint && cargo t-all && cargo audit-all && cargo cov-ci
shellcheck --severity=warning scripts/*.sh
actionlint
docker compose -f docker-compose.integration.yml up -d --build && bash scripts/integration-test.sh
```

Run them before pushing. The image jobs take tens of minutes, and a red pipeline
is a slower way to learn that `cargo fmt` was not run.

## Related

- [`workflows.md`](workflows.md) — each workflow, job by job
- [`releasing.md`](releasing.md) — the release runbook
- [`hardening.md`](hardening.md) — what the pipeline is protecting
- [`testing.md`](testing.md) — what the test jobs actually run
