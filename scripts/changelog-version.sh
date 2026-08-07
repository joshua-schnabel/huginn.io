#!/usr/bin/env bash
#
# Read the topmost released version from CHANGELOG.md and print it.
#
# The version is validated against SemVer 2.0.0 *before* it is printed, so the
# only thing a caller can ever receive is `x.y.z[-prerelease][+build]`. That
# matters because the value is a workflow input: CHANGELOG.md is editable by
# anyone who can land a commit, and an unvalidated heading such as
# `## [0.1.2$(...)]` would otherwise reach a shell as command substitution.
# Validating here — rather than in one gate job — keeps every consumer safe,
# including pushes to `dev`, where the release version gate is a no-op.
#
# Usage: scripts/changelog-version.sh [path/to/CHANGELOG.md]
# Prints the version on stdout; exits non-zero with a message on stderr if the
# file has no versioned entry or the entry is not valid SemVer.

set -euo pipefail

CHANGELOG="${1:-CHANGELOG.md}"

if [ ! -f "$CHANGELOG" ]; then
  echo "::error::$CHANGELOG not found" >&2
  exit 1
fi

# `## [1.2.3] - 2026-01-01` → `1.2.3`. The leading digit class skips
# `## [Unreleased]`, which must never be treated as a release. The `|| true`
# keeps a no-match `grep` (exit 1) from tripping `set -e`/`pipefail` before the
# explicit error below can explain what is wrong.
version="$(grep -m1 '^## \[[0-9]' "$CHANGELOG" | sed 's/## \[\(.*\)\].*/\1/' || true)"

if [ -z "$version" ]; then
  echo "::error::no versioned entry (## [x.y.z]) found in $CHANGELOG" >&2
  exit 1
fi

# SemVer 2.0.0: x.y.z with optional -prerelease and +build metadata.
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'; then
  echo "::error::'$version' in $CHANGELOG is not a valid SemVer version" >&2
  exit 1
fi

printf '%s\n' "$version"
