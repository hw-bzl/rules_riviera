#!/usr/bin/env bash
# Build the rules_riviera release archive and emit release notes.
# Invoked by bazel-contrib/.github/.github/workflows/release_ruleset.yaml
# at the hardcoded path `.github/workflows/release_prep.sh`.
#
# Args:
#   $1: tag name (e.g. 0.1.0). Must match VERSION in version.bzl.
#
# Side effects:
#   Writes rules_riviera-${TAG}.tar.gz to the current directory.
#
# Output:
#   Release notes to stdout (redirected by the caller into
#   release_notes.txt). All other noise goes to stderr.

set -euo pipefail

exec 3>&1 1>&2

TAG="${1:?tag_name required}"
WORKSPACE="${GITHUB_WORKSPACE:-$(pwd)}"

ON_DISK_VERSION="$(grep 'VERSION =' "${WORKSPACE}/version.bzl" | sed 's/VERSION = "//' | sed 's/"//')"
if [[ "${ON_DISK_VERSION}" != "${TAG}" ]]; then
    echo "ERROR: tag ${TAG} does not match version.bzl VERSION=${ON_DISK_VERSION}"
    exit 1
fi

# Stage the archive outside the workspace to avoid `tar: file changed
# as we read it` from GNU tar noticing its own mtime bump on cwd.
ARCHIVE="rules_riviera-${TAG}.tar.gz"
STAGING="$(mktemp -d)/${ARCHIVE}"
trap 'rm -rf "$(dirname "${STAGING}")"' EXIT
tar -czf "${STAGING}" \
    --exclude=".git" \
    --exclude=".github" \
    -C "${WORKSPACE}" .
mv "${STAGING}" "${ARCHIVE}"

NOTES="$(mktemp)"
sed "s#{version}#${TAG}#g" \
    "${WORKSPACE}/.github/release_notes.template" > "${NOTES}"

cat "${NOTES}" >&3
