#!/usr/bin/env bash
# Prints the product version and fails unless every place that states it agrees: the Cargo
# workspace, the SDK packaging script and the Studio package. The release workflow compares it with
# the tag `v<version>` (see docs/runbooks/release.md).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

workspace="$(awk '/^\[workspace.package\]/ { inside = 1; next }
  /^\[/ { inside = 0 }
  inside && /^version *=/ { gsub(/"/, "", $3); print $3; exit }' "${repo_root}/Cargo.toml")"
sdk="$(sed -n 's/^readonly SDK_VERSION="\(.*\)"$/\1/p' "${repo_root}/scripts/package-sdks.sh")"
studio="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "${repo_root}/frontend/apps/studio/package.json" | head -n 1)"

if [[ -z "${workspace}" ]]; then
  echo "no [workspace.package] version in Cargo.toml" >&2
  exit 1
fi
if [[ ! "${workspace}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "workspace version ${workspace} is not semver" >&2
  exit 1
fi
for pair in "scripts/package-sdks.sh:${sdk}" "frontend/apps/studio/package.json:${studio}"; do
  if [[ "${pair#*:}" != "${workspace}" ]]; then
    echo "${pair%%:*} states ${pair#*:}, the workspace states ${workspace}" >&2
    exit 1
  fi
done
echo "${workspace}"
