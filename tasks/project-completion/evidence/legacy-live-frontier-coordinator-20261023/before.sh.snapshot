#!/usr/bin/env bash
# Closed immutable-old/current cases for ordering ordinary known successes, stop and drain.
set -euo pipefail
umask 077

readonly old_commit="6c44a0e49670cf19660792cd1518d241e6c99b3b"
case_name="report-before-drain"
if (( $# != 0 )); then
  if (( $# != 2 )) || [[ "$1" != "--case" ]]; then
    echo "usage: $0 [--case report-before-drain|drain-before-report|cancellation-before-report|under-limit-increment-before-drain]" >&2
    exit 2
  fi
  case_name="$2"
fi
case "${case_name}" in
  report-before-drain)
    old_test="legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_success_before_drain_producer"
    current_test="legacy_collection_inflight_report_v1::old_ordinary_success_before_drain_preserves_the_report_and_compensation"
    artifact_namespace="sagat-q10c-ordinary-before-drain-v1"
    log_label="ordinary-before-drain"
    ;;
  drain-before-report)
    old_test="legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_success_before_drain_producer"
    current_test="legacy_collection_inflight_report_v1::old_ordinary_success_after_drain_preserves_the_report_and_compensation"
    artifact_namespace="sagat-q10c-ordinary-drain-before-report-v1"
    log_label="ordinary-drain-before-report"
    ;;
  cancellation-before-report)
    old_test="legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_success_before_drain_producer"
    current_test="legacy_collection_inflight_report_v1::old_ordinary_success_after_cancellation_drain_preserves_the_original_stop"
    artifact_namespace="sagat-q10c-ordinary-cancellation-before-report-v1"
    log_label="ordinary-cancellation-before-report"
    ;;
  under-limit-increment-before-drain)
    old_test="legacy_collection_inflight_report_producer_v1::frozen_old_ordinary_under_limit_increment_producer"
    current_test="legacy_collection_inflight_report_v1::old_under_limit_increment_before_drain_preserves_the_report_and_compensation"
    artifact_namespace="sagat-q10c-ordinary-under-limit-increment-v1"
    log_label="ordinary-under-limit-increment-before-drain"
    ;;
  *)
    echo "unknown case: ${case_name}" >&2
    exit 2
    ;;
esac
readonly case_name old_test current_test artifact_namespace log_label
repo_root="$(git rev-parse --show-toplevel)"
producer_overlay="${repo_root}/tests/fixtures/legacy-collection-inflight-report-v1/producer.rs"
consumer_source="${repo_root}/tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs"
coordinator_source="${repo_root}/scripts/test-legacy-collection-inflight-report-v1.sh"
fixture_readme="${repo_root}/tests/fixtures/legacy-collection-inflight-report-v1/README.md"
current_main="${repo_root}/tests/integration/workflow-primitives/main.rs"

if [[ "$(git -C "${repo_root}" rev-parse "${old_commit}^{commit}")" != "${old_commit}" ]]; then
  echo "the reviewed old commit is unavailable" >&2
  exit 1
fi
if [[ -z "${CARGO_TARGET_DIR:-}" || "${CARGO_TARGET_DIR}" != /* ]]; then
  echo "CARGO_TARGET_DIR must be the absolute coordinated workspace target" >&2
  exit 2
fi
if [[ -z "${SAGAT_TEST_DATABASE_URL:-}" || "${SAGAT_Q10C_OWNED_POSTGRES:-}" != "1" ]]; then
  echo "set SAGAT_TEST_DATABASE_URL to the owned external admin URL and SAGAT_Q10C_OWNED_POSTGRES=1" >&2
  exit 2
fi
for command in cargo git python3 psql tar; do
  command -v "${command}" >/dev/null || { echo "missing command: ${command}" >&2; exit 2; }
done
[[ -f "${producer_overlay}" ]] || { echo "missing immutable-old producer overlay" >&2; exit 2; }
[[ -f "${consumer_source}" ]] || { echo "missing current legacy in-flight consumer" >&2; exit 2; }
[[ -f "${fixture_readme}" ]] || { echo "missing fixture scope documentation" >&2; exit 2; }

df -h "${CARGO_TARGET_DIR}"
available_kib="$(df -Pk "${CARGO_TARGET_DIR}" | awk 'NR==2 {print $4}')"
if (( available_kib < 8 * 1024 * 1024 )); then
  echo "less than 8 GiB is available for the coordinated target" >&2
  exit 2
fi

artifact_root="${SAGAT_Q10C_ARTIFACT_ROOT:-${TMPDIR:-/tmp}/${artifact_namespace}}"
mkdir -p -m 0700 "${artifact_root}"
run_root="$(mktemp -d "${artifact_root}/run.XXXXXX")"
chmod 0700 "${run_root}"
old_source="${run_root}/old-source"
artifacts="${run_root}/artifacts"
logs="${run_root}/logs"
mkdir -m 0700 "${old_source}" "${artifacts}" "${logs}"
manifest="${artifacts}/MANIFEST"
active_before=""
producer_started_marker=""
active_phase_pid=""
phase_launching=0

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}';
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

file_mode() {
  if stat -f '%Lp' "$1" >/dev/null 2>&1; then stat -f '%Lp' "$1";
  else stat -c '%a' "$1"; fi
}

source_manifest() {
  python3 - "$1" <<'PY'
from hashlib import sha256
from pathlib import Path
import sys
root = Path(sys.argv[1])
for path in sorted(p for p in root.rglob('*') if p.is_file()):
    print(sha256(path.read_bytes()).hexdigest(), path.relative_to(root).as_posix())
PY
}

list_test_databases() {
  psql "${SAGAT_TEST_DATABASE_URL}" -XAtq -v ON_ERROR_STOP=1 \
    -c "SELECT datname FROM pg_database WHERE datname ~ '^t_[0-9a-f]{32}$' ORDER BY datname"
}

drop_database() {
  local name="$1"
  [[ "${name}" =~ ^t_[0-9a-f]{32}$ ]] || { echo "refusing unsafe database name" >&2; return 1; }
  psql "${SAGAT_TEST_DATABASE_URL}" -Xq -v ON_ERROR_STOP=1 \
    -c "DROP DATABASE IF EXISTS \"${name}\" WITH (FORCE)" >/dev/null
}

terminate_active_phase() {
  local pid="${active_phase_pid}"
  if [[ -z "${pid}" && "${phase_launching}" == 1 ]]; then
    pid="$(jobs -pr | tail -n 1)"
  fi
  [[ -n "${pid}" ]] || return 0
  active_phase_pid=""
  phase_launching=0
  kill -TERM "${pid}" 2>/dev/null || true
  set +e
  wait "${pid}" 2>/dev/null
  set -e
}

cleanup_new_databases() {
  [[ -n "${active_before}" && -f "${active_before}" ]] || return 0
  [[ -n "${producer_started_marker}" && -f "${producer_started_marker}" ]] || return 0
  local after="${run_root}/databases.after.cleanup"
  list_test_databases > "${after}" || return 0
  while IFS= read -r database; do
    [[ -n "${database}" ]] && drop_database "${database}" || true
  done < <(comm -13 "${active_before}" "${after}")
  active_before=""
  producer_started_marker=""
}

cleanup() {
  local status=$?
  trap - INT TERM
  terminate_active_phase
  cleanup_new_databases
  python3 - "${old_source}" "${run_root}" <<'PY'
from pathlib import Path
import shutil, sys
source, run = map(Path, sys.argv[1:])
if source.exists():
    shutil.rmtree(source)
for receipt in run.glob('*/receipt.json*'):
    receipt.unlink(missing_ok=True)
for receipt in run.glob('*/.receipt.*.tmp'):
    receipt.unlink(missing_ok=True)
PY
  exit "${status}"
}

signal_exit() {
  local status="$1"
  trap - INT TERM
  terminate_active_phase
  exit "${status}"
}
trap cleanup EXIT
trap 'signal_exit 130' INT
trap 'signal_exit 143' TERM

run_bounded() {
  local seconds="$1"
  local started_marker="$2"
  shift 2
  phase_launching=1
  python3 - "${seconds}" "${started_marker}" "$@" <<'PY' &
import os
from pathlib import Path
import signal
import subprocess
import sys

timeout = int(sys.argv[1])
marker = sys.argv[2]
child = None
pending_signal = None

def terminate_child():
    global child
    if child is None or child.poll() is not None:
        return
    os.killpg(child.pid, signal.SIGTERM)
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()

def interrupted(signum, _frame):
    global pending_signal
    if child is None:
        pending_signal = signum
        return
    terminate_child()
    raise SystemExit(128 + signum)

signal.signal(signal.SIGINT, interrupted)
signal.signal(signal.SIGTERM, interrupted)
child = subprocess.Popen(sys.argv[3:], start_new_session=True)
if pending_signal is not None:
    terminate_child()
    raise SystemExit(128 + pending_signal)
try:
    if marker != '-':
        temporary = Path(marker + '.tmp')
        temporary.write_text(str(child.pid), encoding='ascii')
        os.replace(temporary, marker)
except BaseException:
    terminate_child()
    raise
try:
    result = child.wait(timeout=timeout)
except subprocess.TimeoutExpired:
    terminate_child()
    raise SystemExit(f"timed out after {timeout}s")
raise SystemExit(result)
PY
  active_phase_pid=$!
  phase_launching=0
  local status=0
  set +e
  wait "${active_phase_pid}"
  status=$?
  set -e
  active_phase_pid=""
  return "${status}"
}

validate_database_snapshot() {
  python3 - "$1" <<'PY'
from pathlib import Path
import re, sys
lines = Path(sys.argv[1]).read_text(encoding='utf-8').splitlines()
if lines != sorted(set(lines)):
    raise SystemExit('database snapshot is not sorted and unique')
if any(re.fullmatch(r't_[0-9a-f]{32}', line) is None for line in lines):
    raise SystemExit('database snapshot contains an unsafe name')
PY
}

find_test_binary() {
  python3 - "$1" "$2" <<'PY'
import json, sys
wanted = sys.argv[2]
found = []
for line in open(sys.argv[1], encoding='utf-8'):
    try: value = json.loads(line)
    except json.JSONDecodeError: continue
    target = value.get('target', {})
    executable = value.get('executable')
    if value.get('reason') == 'compiler-artifact' and target.get('name') == wanted and executable:
        found.append(executable)
if not found:
    raise SystemExit(f"no executable for {wanted}")
print(found[-1])
PY
}

# Build the immutable-old overlay and the current consumer before any retained lease is created.
git -C "${repo_root}" archive --format=tar "${old_commit}" > "${artifacts}/old-source.tar"
tar -xf "${artifacts}/old-source.tar" -C "${old_source}"
source_manifest "${old_source}" > "${artifacts}/source.before"
source_manifest "${old_source}/crates/infra-postgres/migrations" > "${artifacts}/old-migrations.before"
cp "${old_source}/tests/integration/workflow-primitives/main.rs" "${artifacts}/old-main.original.rs"
cp "${producer_overlay}" \
  "${old_source}/tests/integration/workflow-primitives/legacy_collection_inflight_report_producer_v1.rs"
python3 - "${old_source}/tests/integration/workflow-primitives/main.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
anchor = 'mod collection_pipeline;\n'
module = 'mod legacy_collection_inflight_report_producer_v1;\n'
if text.count(anchor) != 1 or module in text:
    raise SystemExit('unexpected frozen integration main shape')
path.write_text(text.replace(anchor, module + anchor))
PY
diff -u "${artifacts}/old-main.original.rs" \
  "${old_source}/tests/integration/workflow-primitives/main.rs" \
  > "${artifacts}/test-overlay-main.diff" || true

find "${old_source}/crates" "${old_source}/apps" "${old_source}/tests" \
  -type f -name '*.rs' -exec touch {} +
old_build_json="${artifacts}/old-build.jsonl"
(
  cd "${old_source}"
  SAGAT_Q10C_SOURCE_COMMIT="${old_commit}" CARGO_INCREMENTAL=0 \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
    cargo test --locked --no-run --message-format=json \
      --manifest-path tests/Cargo.toml --test integration_workflow_primitives
) > "${old_build_json}"
old_built="$(find_test_binary "${old_build_json}" integration_workflow_primitives)"
old_binary="${artifacts}/integration-workflow-primitives-old"
install -m 0755 "${old_built}" "${old_binary}"

cp "${artifacts}/old-main.original.rs" \
  "${old_source}/tests/integration/workflow-primitives/main.rs"
python3 - "${old_source}/tests/integration/workflow-primitives/legacy_collection_inflight_report_producer_v1.rs" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).unlink()
PY
source_manifest "${old_source}" > "${artifacts}/source.after"
source_manifest "${old_source}/crates/infra-postgres/migrations" > "${artifacts}/old-migrations.after"
cmp "${artifacts}/source.before" "${artifacts}/source.after"
cmp "${artifacts}/old-migrations.before" "${artifacts}/old-migrations.after"

find "${repo_root}/crates" "${repo_root}/apps" "${repo_root}/tests" \
  -type f -name '*.rs' -exec touch {} +
current_build_json="${artifacts}/current-build.jsonl"
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
  cargo test --locked --no-run --message-format=json \
    --manifest-path "${repo_root}/tests/Cargo.toml" \
    --test integration_workflow_primitives > "${current_build_json}"
current_built="$(find_test_binary "${current_build_json}" integration_workflow_primitives)"
current_binary="${artifacts}/integration-workflow-primitives-current"
install -m 0755 "${current_built}" "${current_binary}"

git -C "${repo_root}" diff --binary HEAD > "${artifacts}/current.diff"
{
  echo "case ${case_name}"
  echo "log-label ${log_label}"
  echo "old-test ${old_test}"
  echo "current-test ${current_test}"
  echo "old-commit ${old_commit}"
  echo "current-commit $(git -C "${repo_root}" rev-parse HEAD)"
  echo "current-diff-sha256 $(sha256 "${artifacts}/current.diff")"
  echo "producer-overlay-sha256 $(sha256 "${producer_overlay}")"
  echo "current-consumer-source-sha256 $(sha256 "${consumer_source}")"
  echo "current-main-source-sha256 $(sha256 "${current_main}")"
  echo "coordinator-source-sha256 $(sha256 "${coordinator_source}")"
  echo "fixture-readme-source-sha256 $(sha256 "${fixture_readme}")"
  echo "old-archive-sha256 $(sha256 "${artifacts}/old-source.tar")"
  echo "old-source-manifest-sha256 $(sha256 "${artifacts}/source.before")"
  echo "restored-source-manifest-sha256 $(sha256 "${artifacts}/source.after")"
  echo "old-migrations-manifest-sha256 $(sha256 "${artifacts}/old-migrations.before")"
  echo "restored-old-migrations-manifest-sha256 $(sha256 "${artifacts}/old-migrations.after")"
  echo "test-overlay-main-diff-sha256 $(sha256 "${artifacts}/test-overlay-main.diff")"
  echo "old-rust-toolchain-sha256 $(sha256 "${old_source}/rust-toolchain.toml")"
  echo "old-cargo-lock-sha256 $(sha256 "${old_source}/Cargo.lock")"
  echo "current-rust-toolchain-sha256 $(sha256 "${repo_root}/rust-toolchain.toml")"
  echo "current-cargo-lock-sha256 $(sha256 "${repo_root}/Cargo.lock")"
  echo "old-test-binary-sha256 $(sha256 "${old_binary}")"
  echo "current-test-binary-sha256 $(sha256 "${current_binary}")"
  echo "q5-producer-source-sha256 $(sha256 "${repo_root}/tests/fixtures/legacy-collection-output/producer.rs")"
  echo "q5-consumer-source-sha256 $(sha256 "${repo_root}/tests/integration/workflow-primitives/legacy_collection_output_view.rs")"
  echo "q5-coordinator-source-sha256 $(sha256 "${repo_root}/scripts/test-legacy-collection-output-view.sh")"
} > "${manifest}"

old_list="${artifacts}/old-tests.list"
current_list="${artifacts}/current-tests.list"
"${old_binary}" --list > "${old_list}"
"${current_binary}" --list > "${current_list}"
[[ "$(grep -Fxc "${old_test}: test" "${old_list}")" == 1 ]] || {
  echo "old producer test is not registered exactly once" >&2; exit 1;
}
[[ "$(grep -Fxc "${current_test}: test" "${current_list}")" == 1 ]] || {
  echo "current consumer test is not registered exactly once" >&2; exit 1;
}
echo "old-registered-test-count 1" >> "${manifest}"
echo "current-registered-test-count 1" >> "${manifest}"

for role in owner runtime; do
  role_root="${run_root}/${role}"
  mkdir -m 0700 "${role_root}"
  receipt="${role_root}/receipt.json"
  snapshot_tmp="${role_root}/databases.before.tmp"
  snapshot="${role_root}/databases.before"
  if ! list_test_databases > "${snapshot_tmp}"; then
    echo "could not capture the pre-producer database set; cleanup remains disarmed" >&2
    exit 1
  fi
  validate_database_snapshot "${snapshot_tmp}"
  mv "${snapshot_tmp}" "${snapshot}"
  active_before="${snapshot}"
  producer_started_marker="${role_root}/producer.started"

  if [[ "${role}" == runtime ]]; then export SAGAT_TEST_AS_RUNTIME_ROLE=1;
  else unset SAGAT_TEST_AS_RUNTIME_ROLE || true; fi
  export SAGAT_Q10C_ORDINARY_PHASE_RECEIPT="${receipt}"

  producer_log="${logs}/${log_label}-${role}-producer.log"
  if ! run_bounded 1200 "${producer_started_marker}" \
    "${old_binary}" --exact "${old_test}" --nocapture > "${producer_log}" 2>&1; then
    cat "${producer_log}" >&2
    exit 1
  fi
  [[ -f "${producer_started_marker}" ]] || {
    echo "old producer was not launched; cleanup remains disarmed" >&2; exit 1;
  }
  grep -Fq "test result: ok. 1 passed; 0 failed;" "${producer_log}" || {
    echo "old producer did not execute exactly one passing test" >&2; exit 1;
  }
  [[ -s "${receipt}" ]] || { echo "old producer wrote no receipt" >&2; exit 1; }
  [[ "$(file_mode "${role_root}")" == 700 ]] || { echo "receipt directory is not mode 0700" >&2; exit 1; }
  [[ "$(file_mode "${receipt}")" == 600 ]] || { echo "receipt is not mode 0600" >&2; exit 1; }
  echo "${role}-producer-exit-status 0" >> "${manifest}"
  echo "${role}-producer-reaped yes" >> "${manifest}"

  database_name="$(python3 - "${receipt}" <<'PY'
import json, re, sys
value = json.load(open(sys.argv[1], encoding='utf-8'))['database_name']
if not re.fullmatch(r't_[0-9a-f]{32}', value): raise SystemExit('unsafe receipt database')
print(value)
PY
)"
  after_producer="${role_root}/databases.after-producer"
  list_test_databases > "${after_producer}"
  validate_database_snapshot "${after_producer}"
  comm -13 "${active_before}" "${after_producer}" | grep -Fxq "${database_name}" || {
    echo "receipt database was not created by this phase" >&2; exit 1;
  }
  [[ "$(comm -13 "${active_before}" "${after_producer}" | wc -l | tr -d ' ')" == 1 ]] || {
    echo "old producer created an unexpected number of databases" >&2; exit 1;
  }

  consumer_log="${logs}/${log_label}-${role}-consumer.log"
  if ! run_bounded 1200 - \
    "${current_binary}" --ignored --exact "${current_test}" --nocapture \
    > "${consumer_log}" 2>&1; then
    cat "${consumer_log}" >&2
    exit 1
  fi
  grep -Fq "test result: ok. 1 passed; 0 failed;" "${consumer_log}" || {
    echo "current consumer did not execute exactly one passing test" >&2; exit 1;
  }
  echo "${role}-consumer-exit-status 0" >> "${manifest}"
  echo "${role}-receipt-sha256 $(sha256 "${receipt}")" >> "${manifest}"
  echo "${role}-database ${database_name}" >> "${manifest}"
  drop_database "${database_name}"
  python3 - "${receipt}" "${producer_started_marker}" <<'PY'
from pathlib import Path
import sys
for name in sys.argv[1:]:
    Path(name).unlink(missing_ok=True)
PY
  active_before=""
  producer_started_marker=""
done
unset SAGAT_TEST_AS_RUNTIME_ROLE SAGAT_Q10C_ORDINARY_PHASE_RECEIPT || true

echo "Q10c ${log_label} evidence: ${run_root}"
