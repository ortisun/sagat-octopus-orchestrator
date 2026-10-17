#!/usr/bin/env bash
# Compare actual HTTP bodies from the integrated platform immediately before feature 016.
set -euo pipefail

readonly baseline_commit=3637ee8c4b98d2e5809f1527a9b2bb1fcaf2406a
repo_root="$(git rev-parse --show-toplevel)"
if [[ -z "${CARGO_TARGET_DIR:-}" || "${CARGO_TARGET_DIR}" != /* ]]; then
  echo "CARGO_TARGET_DIR must name an absolute coordinated current target." >&2
  exit 2
fi
artifact_root="${SAGAT_SURFACE_ARTIFACT_ROOT:-${repo_root}/target/payload-classification-surfaces}"
mkdir -p "${artifact_root}"
artifact_root="$(cd "${artifact_root}" && pwd)"
run_root="$(mktemp -d "${artifact_root}/run.XXXXXX")"
baseline_source="${run_root}/source"
artifact_dir="${run_root}/artifacts"
log_dir="${run_root}/logs"
mkdir -p "${artifact_dir}" "${log_dir}"
git -C "${repo_root}" worktree add --detach "${baseline_source}" "${baseline_commit}"
cleanup() {
  chmod -R u+w "${baseline_source}" 2>/dev/null || true
  git -C "${repo_root}" worktree remove "${baseline_source}" 2>/dev/null || true
}
trap cleanup EXIT
chmod -R a-w "${baseline_source}"

echo "Building pre-classification control plane: ${baseline_commit}"
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  cargo build --locked --manifest-path "${baseline_source}/Cargo.toml" \
    -p sagat-control-plane --bin control-plane
baseline_cp="${artifact_dir}/control-plane-before-016"
install -m 0755 "${CARGO_TARGET_DIR}/debug/control-plane" "${baseline_cp}"
if [[ -n "$(git -C "${baseline_source}" status --porcelain)" ]]; then
  echo "The frozen baseline source changed." >&2
  exit 1
fi

# Same-version local packages otherwise can reuse metadata from the other worktree.
rg --files -0 "${repo_root}/crates" "${repo_root}/apps" -g '*.rs' | xargs -0 touch
echo "Building current control plane."
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  cargo build --locked --manifest-path "${repo_root}/Cargo.toml" \
    -p sagat-control-plane --bin control-plane
current_cp="${artifact_dir}/control-plane-current"
install -m 0755 "${CARGO_TARGET_DIR}/debug/control-plane" "${current_cp}"
manifest="${artifact_dir}/SHA256SUMS"
{
  echo "baseline-commit ${baseline_commit}"
  echo "current-commit $(git -C "${repo_root}" rev-parse HEAD)"
  echo "current-diff-sha256 $(git -C "${repo_root}" diff --binary HEAD | shasum -a 256 | cut -d ' ' -f 1)"
  echo "baseline-control-plane-sha256 $(shasum -a 256 "${baseline_cp}" | cut -d ' ' -f 1)"
  echo "current-control-plane-sha256 $(shasum -a 256 "${current_cp}" | cut -d ' ' -f 1)"
  echo "baseline-lock-sha256 $(shasum -a 256 "${baseline_source}/Cargo.lock" | cut -d ' ' -f 1)"
  echo "current-lock-sha256 $(shasum -a 256 "${repo_root}/Cargo.lock" | cut -d ' ' -f 1)"
} > "${manifest}"
SAGAT_SURFACE_BASELINE_CP="${baseline_cp}" SAGAT_SURFACE_CURRENT_CP="${current_cp}" \
  SAGAT_SURFACE_BASELINE_SOURCE="${baseline_source}" SAGAT_SURFACE_ARTIFACT_DIR="${artifact_dir}" \
  SAGAT_SURFACE_LOG_DIR="${log_dir}" SAGAT_SURFACE_MANIFEST="${manifest}" \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}" \
  cargo test --locked --manifest-path "${repo_root}/tests/Cargo.toml" \
    --test integration_payload_classification -- --ignored --exact \
    compat::unchanged_surfaces_match_actual_pre_classification_response_bytes --nocapture
echo "Surface compatibility evidence: ${run_root}"
