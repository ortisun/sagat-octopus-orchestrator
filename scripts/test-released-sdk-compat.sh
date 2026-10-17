#!/usr/bin/env bash
# Build all five v0.1.0 client runners and execute the current reviewed lifecycle scenario.
set -euo pipefail
readonly release_commit=6ede1c7849c4ac5a209c08da8ad25294fbdd7795
repo_root="$(git rev-parse --show-toplevel)"
if [[ "$(git rev-parse 'v0.1.0^{commit}')" != "${release_commit}" ]]; then
  echo "v0.1.0 does not match the reviewed immutable release." >&2
  exit 1
fi
if [[ -z "${CARGO_TARGET_DIR:-}" || "${CARGO_TARGET_DIR}" != /* ]]; then
  echo "CARGO_TARGET_DIR must name an absolute coordinated current target." >&2
  exit 2
fi
artifact_root="${SAGAT_OLD_SDK_ARTIFACT_ROOT:-${repo_root}/target/released-sdks}"
mkdir -p "${artifact_root}"
artifact_root="$(cd "${artifact_root}" && pwd)"
run_root="$(mktemp -d "${artifact_root}/run.XXXXXX")"
source_root="${run_root}/source"
mkdir -p "${source_root}"
git archive "${release_commit}" sdks specs/001-platform-foundation/contracts | tar -x -C "${source_root}"
{
  echo "release-commit ${release_commit}"
  echo "server-commit $(git rev-parse HEAD)"
  find "${source_root}" -type f -print | LC_ALL=C sort | while IFS= read -r sdk_file; do
    shasum -a 256 "${sdk_file}"
  done
} > "${run_root}/source.sha256"
env -u SAGAT_SDK_LANGS -u SAGAT_SDK_REUSE -u SAGAT_SDK_BLESS \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}" \
  SAGAT_SDK_SOURCE_ROOT="${source_root}/sdks" SAGAT_SDK_SCENARIO=run-lifecycle \
  cargo test --locked --manifest-path "${repo_root}/tests/Cargo.toml" \
    --test sdk_conformance -- --ignored --nocapture
echo "Released SDK source and build evidence: ${run_root}"
