#!/usr/bin/env bash
# Run the real OS-process crash gate; ordinary service reconstruction is covered separately.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
if [[ -z "${CARGO_TARGET_DIR:-}" || "${CARGO_TARGET_DIR}" != /* ]]; then
  echo "CARGO_TARGET_DIR must name an absolute coordinated workspace target." >&2
  exit 2
fi
artifact_root="${SAGAT_PROCESS_FAULT_ARTIFACT_ROOT:-${repo_root}/target/process-faults}"
mkdir -p "${artifact_root}"
export SAGAT_PROCESS_FAULT_LOG_DIR
SAGAT_PROCESS_FAULT_LOG_DIR="$(mktemp -d "${artifact_root}/run.XXXXXX")"
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}"
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

cargo build --locked --manifest-path "${repo_root}/Cargo.toml" \
  -p sagat-control-plane --bin control-plane
export SAGAT_TEST_CONTROL_PLANE="${CARGO_TARGET_DIR}/debug/control-plane"
{
  git -C "${repo_root}" rev-parse HEAD
  git -C "${repo_root}" status --short
  shasum -a 256 "${SAGAT_TEST_CONTROL_PLANE}" "${repo_root}/Cargo.lock"
} > "${SAGAT_PROCESS_FAULT_LOG_DIR}/manifest.txt"

cargo test --locked --manifest-path "${repo_root}/tests/Cargo.toml" \
  --test fault_payload_classification process:: \
  -- --ignored --test-threads=1 --nocapture
echo "Process fault evidence: ${SAGAT_PROCESS_FAULT_LOG_DIR}"
