# GitLab Setup Guide

Everything you need to bring hugin.dev live on GitLab — from zero to a running pipeline with protected branches and DockerHub publishing.

---

## Prerequisites

| Tool | Version |
|---|---|
| Git | ≥ 2.35 |
| Docker Desktop | ≥ 24 (for local testing) |
| GitLab account | gitlab.com or self-hosted |
| DockerHub account | hub.docker.com (free tier is enough) |

---

## Step 1 — Create the GitLab Project

1. Log in to [gitlab.com](https://gitlab.com) (or your self-hosted instance)
2. Click **New project → Create blank project**
3. Fill in:
   - **Project name**: `hugin-dev`
   - **Visibility**: Private (recommended) or Public
   - **Initialize repository with README**: ❌ uncheck (we bring our own)
4. Click **Create project**
5. Note the SSH or HTTPS clone URL shown on the empty project page

---

## Step 2 — Add the Remote and Push

The repository already has two branches ready: `main` (default) and `dev`.

```bash
# Add GitLab as remote  (replace with your project URL)
git remote add origin git@gitlab.com:YOUR_USERNAME/hugin-dev.git

# Push both branches
git push -u origin main
git push -u origin dev

# Verify
git remote -v
```

After pushing, GitLab will detect `.gitlab-ci.yml` and automatically trigger the first pipeline.

> **Note:** If GitLab asks you to set a default branch, choose `main`.

---

## Step 3 — Configure CI/CD Variables

GitLab CI uses **CI/CD Variables** (not `.env` files) for secrets.

Go to: **Settings → CI/CD → Variables → Expand → Add variable**

| Variable | Value | Masked | Protected |
|---|---|:---:|:---:|
| `DOCKERHUB_USERNAME` | Your DockerHub username | ❌ | ❌ |
| `DOCKERHUB_TOKEN` | DockerHub Access Token (see below) | ✅ | ✅ |

### Creating a DockerHub Access Token

1. Log in to [hub.docker.com](https://hub.docker.com)
2. Click your avatar → **Account Settings → Security → New Access Token**
3. Name: `gitlab-ci-hugin-dev`
4. Permissions: **Read & Write**
5. Copy the token — it is shown only once

> **Protected** variables are only available to pipelines running on protected branches/tags. Set `DOCKERHUB_TOKEN` to protected so it is never accessible from feature branches.

---

## Step 4 — Configure Protected Branches

Protected branches prevent direct pushes. Only merge requests are allowed.

Go to: **Settings → Repository → Protected branches**

### `main` branch

| Setting | Value |
|---|---|
| Branch name | `main` |
| Allowed to merge | Maintainers |
| Allowed to push | **No one** |
| Allowed to force push | ❌ off |

### `dev` branch

| Setting | Value |
|---|---|
| Branch name | `dev` |
| Allowed to merge | Developers + Maintainers |
| Allowed to push | **No one** |
| Allowed to force push | ❌ off |

After saving, any attempt to `git push origin dev` or `git push origin main` directly will be rejected with:

```
remote: GitLab: You are not allowed to push code to protected branches on this project.
```

---

## Step 5 — Enable Job Token for Git Tag Pushing

The `publish:release` pipeline job creates a git tag (`v0.1.0`) automatically when releasing to main. This requires the job token to have write permission.

Go to: **Settings → CI/CD → Token Access**

- Enable: **Allow job token to write to the current project's repository** ✅

Without this, the tagging step is skipped silently (it falls back to a no-op).

---

## Step 6 — Configure Merge Request Requirements

Go to: **Settings → Merge requests**

| Setting | Recommended value |
|---|---|
| Merge method | Merge commit (default) |
| Merge checks → Pipelines must succeed | ✅ |
| Merge checks → All discussions must be resolved | ✅ |
| Merge request approvals → Required approvals | 1 |

**Pipelines must succeed** is the key setting — it ensures no MR can be merged if any CI job fails (including the Trivy blocking scan).

---

## Step 7 — Configure Required Pipeline Jobs (MR approval rules)

For stricter enforcement, go to: **Settings → CI/CD → Merge request approvals** and add approval rules that require specific jobs to pass before merge.

Alternatively, since **Pipelines must succeed** is enabled and all jobs are in the pipeline for MR events, any failing job (Trivy, tests, coverage) automatically blocks the merge.

---

## Branch Model Summary

```
feature/my-feature
       │
       │  Merge Request  (runs: lint, test, audit, coverage, system-integration)
       ▼
      dev  ─── push ──► DockerHub  :dev  +  :0.1.0-dev
       │
       │  Merge Request  (runs: all above + Trivy CVE scan)
       │                  Trivy blocks merge if fixable CRITICAL/HIGH CVEs found
       ▼
      main ─── push ──► DockerHub  :latest  +  :0.1.0
                                              git tag v0.1.0 created
```

---

## Pipeline Overview

| Stage | Job | Trigger |
|---|---|---|
| lint | `fmt-clippy` | All MRs + dev/main push |
| test | `test:stable` | All MRs + dev/main push |
| test | `test:beta` | All MRs + dev/main push |
| test | `audit` | All MRs + dev/main push |
| test | `coverage` | All MRs + dev/main push |
| integration | `system-integration` | All MRs + dev/main push |
| security | `trivy` | MR→main + push main only |
| publish | `publish:dev` | Push to dev only |
| publish | `publish:release` | Push to main only |

The `DOCKERHUB_TOKEN` secret is **only injected** into `publish:*` jobs (protected variable), and only for push events — never for MR pipelines.

---

## Creating a New Feature Branch

```bash
# Always branch from dev
git checkout dev
git pull origin dev
git checkout -b feature/my-new-probe

# ... make changes ...

git add -A
git commit -m "feat: add ICMP probe"
git push origin feature/my-new-probe

# Then open a Merge Request to dev on GitLab
```

---

## Releasing a New Version

1. Update `CHANGELOG.md` — add a new `## [x.y.z] - YYYY-MM-DD` entry at the top
2. Open a MR from `dev` into `main`
3. All checks must pass including Trivy
4. Merge the MR
5. GitLab pipeline automatically:
   - Builds multi-platform image (`linux/amd64`, `linux/arm64`)
   - Pushes to DockerHub as `:latest` and `:x.y.z`
   - Creates git tag `vx.y.z`

---

## Viewing Security Results

After a Trivy scan runs on a MR to `main`:

1. Open the Merge Request
2. Click the **Security** tab (appears automatically when SAST results exist)
3. All CRITICAL/HIGH/MEDIUM findings are listed with CVE IDs, severity, and fix availability
4. If fixable CRITICAL/HIGH CVEs exist → pipeline fails → merge is blocked

To view the full SARIF report:
- Go to the pipeline job → **Artifacts → gl-sast-report.json**

---

## Troubleshooting

### Pipeline not starting after push

Make sure `.gitlab-ci.yml` is in the root of the repository and is valid YAML:

```bash
# Validate locally (requires Python)
pip install pyyaml
python -c "import yaml; yaml.safe_load(open('.gitlab-ci.yml'))"
```

Or use the GitLab UI: **CI/CD → Editor → Validate** tab.

### Docker build fails in CI

The `system-integration` and `trivy` jobs use Docker-in-Docker (DinD). Make sure the GitLab Runner being used has the Docker executor configured. On gitlab.com shared runners this works by default.

For self-hosted runners, the runner must be registered with `--executor docker` and have `privileged = true` in `/etc/gitlab-runner/config.toml`.

### DOCKERHUB_TOKEN not injected (publish job fails with unauthorized)

Check that `DOCKERHUB_TOKEN` is set as a **Protected** variable and that the pipeline is running on a protected branch (`main` or `dev`).

### git tag push fails in publish:release

Enable **Settings → CI/CD → Token Access → Allow job token to write to the current project's repository**.

### Trivy scan fails with "no such image"

The `trivy` job builds the image in a DinD environment. Make sure the Docker socket is shared correctly. The job uses `DOCKER_HOST: tcp://docker:2376` — do not override this.
