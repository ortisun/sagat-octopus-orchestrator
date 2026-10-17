#!/usr/bin/env bash
# Builds the immutable v0.1.0 processes and runs the T016-052 compatibility gate.
# This script intentionally uses no Docker image: the source commit, lock files, toolchains and
# produced bytes are recorded directly in an evidence manifest.
set -euo pipefail

readonly RELEASE_TAG="v0.1.0"
readonly RELEASE_COMMIT="e06532dd0e288273f63966e0d950bb4d7c743459"

repo_root="$(git rev-parse --show-toplevel)"
artifact_root="${SAGAT_COMPAT_ARTIFACT_ROOT:-${repo_root}/target/payload-classification-compat/v0.1.0}"
old_source="${artifact_root}/source"
old_target="${artifact_root}/cargo-target"
old_dist="${artifact_root}/dist"
studio_source="${artifact_root}/studio-source"
manifest="${artifact_root}/SHA256SUMS"

if [[ "$(git -C "${repo_root}" rev-parse "${RELEASE_TAG}^{commit}")" != "${RELEASE_COMMIT}" ]]; then
  echo "${RELEASE_TAG} does not resolve to the reviewed release commit ${RELEASE_COMMIT}" >&2
  exit 1
fi
if [[ "${SAGAT_COMPAT_BUILD_WINDOW:-}" != "granted" ]]; then
  echo "Set SAGAT_COMPAT_BUILD_WINDOW=granted only during an explicitly allocated build window." >&2
  echo "The first build can add more than 500 MiB; no build was started." >&2
  exit 2
fi
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  echo "CARGO_TARGET_DIR must name the coordinated current-workspace target." >&2
  exit 2
fi

mkdir -p "${artifact_root}" "${old_dist}"
if [[ -e "${old_source}/.git" ]]; then
  git worktree remove --force "${old_source}"
fi
rm -rf "${old_source}" "${studio_source}"
git worktree add --detach "${old_source}" "${RELEASE_COMMIT}"

cleanup() {
  chmod -R u+w "${old_source}" 2>/dev/null || true
  git worktree remove --force "${old_source}" 2>/dev/null || true
}
trap cleanup EXIT

echo "Building released Rust processes from ${RELEASE_TAG} (${RELEASE_COMMIT}) with Cargo.lock."
(
  cd "${old_source}"
  CARGO_TARGET_DIR="${old_target}" CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
    cargo build --release --locked \
      -p sagat-control-plane --bin control-plane \
      -p sagat-execution-engine --bin execution-engine
)
install -m 0755 "${old_target}/release/control-plane" "${old_dist}/control-plane"
install -m 0755 "${old_target}/release/execution-engine" "${old_dist}/execution-engine"

# npm writes node_modules, so the detached release worktree remains read-only: the exact frontend
# tree is copied from the reviewed commit and built in the artifact directory.
mkdir -p "${studio_source}"
git archive "${RELEASE_COMMIT}" frontend | tar -x -C "${studio_source}"
npm ci --prefix "${studio_source}/frontend" --ignore-scripts --no-audit --no-fund
npm run build --prefix "${studio_source}/frontend" --workspace @sagat/studio
rm -rf "${old_dist}/studio"
cp -R "${studio_source}/frontend/apps/studio/dist" "${old_dist}/studio"

if [[ -n "$(git -C "${old_source}" status --porcelain)" ]]; then
  echo "the detached release source was modified while producing artifacts" >&2
  git -C "${old_source}" status --short >&2
  exit 1
fi
chmod -R a-w "${old_source}"

echo "Building the current control plane in the coordinated target."
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  cargo build --locked -p sagat-control-plane --bin control-plane

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

{
  echo "release-tag ${RELEASE_TAG}"
  echo "release-commit ${RELEASE_COMMIT}"
  echo "current-commit $(git -C "${repo_root}" rev-parse HEAD)"
  echo "rustc-version $(cd "${old_source}" && rustc --version | awk '{print $2}')"
  echo "rustc-commit $(cd "${old_source}" && rustc --version --verbose | awk '/commit-hash/{print $2}')"
  echo "cargo-version $(cd "${old_source}" && cargo --version | awk '{print $2}')"
  echo "node-version $(node --version)"
  echo "npm-version $(npm --version)"
  echo "rust-toolchain-sha256 $(sha256 "${old_source}/rust-toolchain.toml")"
  echo "cargo-lock-sha256 $(sha256 "${old_source}/Cargo.lock")"
  echo "frontend-lock-sha256 $(sha256 "${old_source}/frontend/package-lock.json")"
  echo "current-rust-toolchain-sha256 $(sha256 "${repo_root}/rust-toolchain.toml")"
  echo "current-cargo-lock-sha256 $(sha256 "${repo_root}/Cargo.lock")"
  echo "control-plane-sha256 $(sha256 "${old_dist}/control-plane")"
  echo "execution-engine-sha256 $(sha256 "${old_dist}/execution-engine")"
  echo "current-control-plane-sha256 $(sha256 "${CARGO_TARGET_DIR}/debug/control-plane")"
  find "${old_dist}/studio" -type f -print | LC_ALL=C sort | while IFS= read -r file; do
    relative="${file#"${old_dist}/studio/"}"
    echo "studio-sha256 $(sha256 "${file}") ${relative}"
  done
} > "${manifest}"

export SAGAT_COMPAT_RELEASE_TAG="${RELEASE_TAG}"
export SAGAT_COMPAT_RELEASE_COMMIT="${RELEASE_COMMIT}"
export SAGAT_COMPAT_OLD_SOURCE="${old_source}"
export SAGAT_COMPAT_OLD_CONTROL_PLANE="${old_dist}/control-plane"
export SAGAT_COMPAT_OLD_EXECUTION_ENGINE="${old_dist}/execution-engine"
export SAGAT_COMPAT_OLD_STUDIO="${old_dist}/studio"
export SAGAT_COMPAT_CURRENT_CONTROL_PLANE="${CARGO_TARGET_DIR}/debug/control-plane"
export SAGAT_COMPAT_MANIFEST="${manifest}"

CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  cargo test --manifest-path tests/Cargo.toml --test integration_payload_classification \
    'compat::actual_v0_1_0_processes_survive_the_additive_upgrade' \
    -- --ignored --exact --nocapture

echo "Compatibility artifact manifest: ${manifest}"
