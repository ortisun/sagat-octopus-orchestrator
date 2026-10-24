#!/usr/bin/env bash
# Builds the immutable v0.1.0 processes and runs the T016-052 compatibility gate.
# This script intentionally uses no Docker image: the source commit, lock files, toolchains and
# produced bytes are recorded directly in an evidence manifest.
set -euo pipefail

readonly RELEASE_TAG="v0.1.0"
readonly RELEASE_COMMIT="6ede1c7849c4ac5a209c08da8ad25294fbdd7795"

repo_root="$(git rev-parse --show-toplevel)"
artifact_root="${SAGAT_COMPAT_ARTIFACT_ROOT:-${repo_root}/target/payload-classification-compat/v0.1.0}"
if [[ "$(git -C "${repo_root}" rev-parse "${RELEASE_TAG}^{commit}")" != "${RELEASE_COMMIT}" ]]; then
  echo "${RELEASE_TAG} does not resolve to the reviewed release commit ${RELEASE_COMMIT}" >&2
  exit 1
fi
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  echo "CARGO_TARGET_DIR must name the coordinated current-workspace target." >&2
  exit 2
fi
if [[ "${CARGO_TARGET_DIR}" != /* ]]; then
  echo "CARGO_TARGET_DIR must be absolute so both source worktrees use the same target." >&2
  exit 2
fi

mkdir -p "${artifact_root}"
run_root="$(mktemp -d "${artifact_root}/run.XXXXXX")"
old_source="${run_root}/source"
studio_source="${run_root}/studio-source"
artifact_dir="${run_root}/artifacts"
manifest="${artifact_dir}/SHA256SUMS"
created_worktree="false"

mkdir -p "${artifact_dir}" "${studio_source}"
if [[ ! -e "${old_source}/.git" ]]; then
  git -C "${repo_root}" worktree add --detach "${old_source}" "${RELEASE_COMMIT}"
  created_worktree="true"
fi
chmod -R a-w "${old_source}"

cleanup() {
  if [[ "${created_worktree}" == "true" && -e "${old_source}/.git" ]]; then
    chmod -R u+w "${old_source}" 2>/dev/null || true
    git -C "${repo_root}" worktree remove "${old_source}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "Building released Rust processes from ${RELEASE_TAG} (${RELEASE_COMMIT}) with Cargo.lock."
(
  cd "${old_source}"
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
    cargo build --locked \
      -p sagat-control-plane --bin control-plane \
      -p sagat-execution-engine --bin execution-engine
)
old_control_plane="${artifact_dir}/control-plane-v0.1.0-${RELEASE_COMMIT}-debug"
old_execution_engine="${artifact_dir}/execution-engine-v0.1.0-${RELEASE_COMMIT}-debug"
install -m 0755 "${CARGO_TARGET_DIR}/debug/control-plane" "${old_control_plane}"
install -m 0755 "${CARGO_TARGET_DIR}/debug/execution-engine" "${old_execution_engine}"

# npm writes node_modules, so the detached release worktree remains read-only: the exact frontend
# tree is copied from the reviewed commit and built in the artifact directory.
mkdir -p "${studio_source}"
git -C "${repo_root}" archive "${RELEASE_COMMIT}" frontend | tar -x -C "${studio_source}"
npm ci --prefix "${studio_source}/frontend" --ignore-scripts --no-audit --no-fund
npm run build --prefix "${studio_source}/frontend" --workspace @sagat/studio
old_studio="${artifact_dir}/studio-v0.1.0-${RELEASE_COMMIT}"
cp -R "${studio_source}/frontend/apps/studio/dist" "${old_studio}"

if [[ -n "$(git -C "${old_source}" status --porcelain)" ]]; then
  echo "the detached release source was modified while producing artifacts" >&2
  git -C "${old_source}" status --short >&2
  exit 1
fi
echo "Building the current control plane in the coordinated target."
# Cargo's local package fingerprints can otherwise reuse metadata from a different worktree with
# the same package version. Refresh only current workspace source mtimes; contents stay unchanged.
find "${repo_root}/crates" "${repo_root}/apps" -type f -name '*.rs' -exec touch {} +
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  cargo build --manifest-path "${repo_root}/Cargo.toml" --locked \
    -p sagat-control-plane --bin control-plane
current_commit="$(git -C "${repo_root}" rev-parse HEAD)"
current_control_plane="${artifact_dir}/control-plane-current-${current_commit}-debug"
install -m 0755 "${CARGO_TARGET_DIR}/debug/control-plane" "${current_control_plane}"

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
  echo "current-commit ${current_commit}"
  echo "cargo-profile debug"
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
  echo "control-plane-sha256 $(sha256 "${old_control_plane}")"
  echo "execution-engine-sha256 $(sha256 "${old_execution_engine}")"
  echo "current-control-plane-sha256 $(sha256 "${current_control_plane}")"
  find "${old_studio}" -type f -print | LC_ALL=C sort | while IFS= read -r file; do
    relative="${file#"${old_studio}/"}"
    echo "studio-sha256 $(sha256 "${file}") ${relative}"
  done
} > "${manifest}"

export SAGAT_COMPAT_RELEASE_TAG="${RELEASE_TAG}"
export SAGAT_COMPAT_RELEASE_COMMIT="${RELEASE_COMMIT}"
export SAGAT_COMPAT_OLD_SOURCE="${old_source}"
export SAGAT_COMPAT_OLD_CONTROL_PLANE="${old_control_plane}"
export SAGAT_COMPAT_OLD_EXECUTION_ENGINE="${old_execution_engine}"
export SAGAT_COMPAT_OLD_STUDIO="${old_studio}"
export SAGAT_COMPAT_CURRENT_CONTROL_PLANE="${current_control_plane}"
export SAGAT_COMPAT_MANIFEST="${manifest}"
export SAGAT_COMPAT_LOG_DIR="${run_root}/logs"
mkdir -p "${SAGAT_COMPAT_LOG_DIR}"

test_name="compat::actual_v0_1_0_processes_survive_the_additive_upgrade"
test_list="$(
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
    cargo test --manifest-path "${repo_root}/tests/Cargo.toml" \
      --test integration_payload_classification \
      -- --ignored --list
)"
if ! grep -Fq "${test_name}: test" <<< "${test_list}"; then
  echo "compatibility test is not registered in the integration target" >&2
  exit 1
fi
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" \
  cargo test --manifest-path "${repo_root}/tests/Cargo.toml" \
    --test integration_payload_classification \
    "${test_name}" \
    -- --ignored --exact --nocapture

echo "Compatibility artifact manifest: ${manifest}"
