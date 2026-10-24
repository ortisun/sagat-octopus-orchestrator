#!/usr/bin/env bash
# Builds a test-only old writer from immutable 6c44a0e4, then runs the current Q5 reader twice.
set -euo pipefail
umask 077
case "${SAGAT_Q5_CASE-output-view}" in
  output-view) fixture_case=output-view ;;
  nonfinal-return) fixture_case=nonfinal-return ;;
  nonfinal-return-red) fixture_case=nonfinal-return-red ;;
  free-admission) fixture_case=free-admission ;;
  free-admission-red) fixture_case=free-admission-red ;;
  free-admission-equal-limit) fixture_case=free-admission-equal-limit ;;
  failed-member) fixture_case=failed-member ;;
  *) echo "unknown SAGAT_Q5_CASE" >&2; exit 2 ;;
esac
# Bound the entire new fixture, including builds and cleanup, in a separate process group.
if [[ "${fixture_case}" != output-view && "${SAGAT_Q5_SUPERVISED:-}" != 1 ]]; then
  exec python3 - "$0" "$@" <<'SUPERVISOR'
import os,select,signal,subprocess,sys,time
child=None
pending=None
def interrupted(signum,_frame):
    global pending
    pending=signum
def stop(signum):
    # The leader is still unreaped: retain ownership of its PGID through KILL.
    try:os.killpg(child.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    deadline=time.monotonic()+5
    while time.monotonic()<deadline:
        select.select([],[],[],max(0,deadline-time.monotonic()))
    try:os.killpg(child.pid,signal.SIGKILL)
    except ProcessLookupError:pass
    child.wait()
    raise SystemExit(128+signum)
signal.signal(signal.SIGTERM,interrupted);signal.signal(signal.SIGINT,interrupted)
env=dict(os.environ,SAGAT_Q5_SUPERVISED='1')
child=subprocess.Popen(['bash',*sys.argv[1:]],env=env,start_new_session=True)
deadline=time.monotonic()+2400
while True:
    if pending is not None:stop(pending)
    if time.monotonic()>=deadline:
        print('whole fixture exceeded 2400s; inspect retained owned artifacts',file=sys.stderr)
        stop(signal.SIGTERM)
    # WNOWAIT observes completion without releasing the owned group leader.
    finished=os.waitid(os.P_PID,child.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
    if finished is not None:
        if pending is not None:continue
        result=child.wait()
        break
    select.select([],[],[],min(0.1,max(0,deadline-time.monotonic())))
raise SystemExit(result)
SUPERVISOR
fi

readonly old_commit="6c44a0e49670cf19660792cd1518d241e6c99b3b"
old_test="legacy_collection_output_producer::frozen_old_collection_output_producer"
current_test="legacy_collection_output_view::a_current_full_run_transaction_reads_frozen_legacy_collection_outputs"
phase_seconds=1200
expected_red_marker=""
case "${fixture_case}" in
  output-view) ;;
  nonfinal-return|nonfinal-return-red)
    old_test="legacy_collection_output_producer::frozen_old_nonfinal_return_frontier_producer"
    current_test="legacy_collection_output_view::current_frontier_drains_frozen_old_nonfinal_return_before_checkpoint"
    phase_seconds=300
    if [[ "${fixture_case}" == nonfinal-return-red ]]; then
      old_test="legacy_collection_output_producer::frozen_old_nonfinal_return_frontier_guard_red"
      expected_red_marker='observed_legacy_nonfinal_return_checkpointed=true'
    fi ;;
  failed-member)
    old_test="legacy_collection_output_producer::frozen_old_failed_member_frontier_producer"
    current_test="legacy_collection_output_view::current_frontier_preserves_frozen_old_member_failure_over_excess"
    phase_seconds=300 ;;
  free-admission-equal-limit)
    old_test="legacy_collection_output_producer::frozen_old_free_admission_equal_limit_producer"
    current_test="legacy_collection_output_view::current_frontier_admits_frozen_old_free_admission_at_exact_limit"
    phase_seconds=300 ;;
  free-admission|free-admission-red)
    old_test="legacy_collection_output_producer::frozen_old_free_admission_frontier_producer"
    current_test="legacy_collection_output_view::current_frontier_drains_frozen_old_free_admission_before_first_member"
    phase_seconds=300
    if [[ "${fixture_case}" == free-admission-red ]]; then
      old_test="legacy_collection_output_producer::frozen_old_free_admission_frontier_guard_red"
      expected_red_marker='observed_legacy_free_admission_admitted_zero=true'
    fi ;;
esac
readonly fixture_case old_test current_test phase_seconds expected_red_marker
repo_root="$(git rev-parse --show-toplevel)"
producer_overlay="${repo_root}/tests/fixtures/legacy-collection-output/producer.rs"
consumer_source="${repo_root}/tests/integration/workflow-primitives/legacy_collection_output_view.rs"
coordinator_source="${repo_root}/scripts/test-legacy-collection-output-view.sh"

if [[ "$(git -C "${repo_root}" rev-parse "${old_commit}^{commit}")" != "${old_commit}" ]]; then
  echo "the reviewed old commit is unavailable" >&2
  exit 1
fi
if [[ -z "${CARGO_TARGET_DIR:-}" || "${CARGO_TARGET_DIR}" != /* ]]; then
  echo "CARGO_TARGET_DIR must be the absolute coordinated workspace target" >&2
  exit 2
fi
if [[ -z "${SAGAT_TEST_DATABASE_URL:-}" || "${SAGAT_Q5_OWNED_POSTGRES:-}" != "1" ]]; then
  echo "set SAGAT_TEST_DATABASE_URL to the owned external admin URL and SAGAT_Q5_OWNED_POSTGRES=1" >&2
  exit 2
fi
for command in cargo git python3 psql tar; do
  command -v "${command}" >/dev/null || { echo "missing command: ${command}" >&2; exit 2; }
done
[[ -f "${producer_overlay}" ]] || { echo "missing Q5 old-producer overlay" >&2; exit 2; }
[[ -f "${consumer_source}" ]] || { echo "apply the focused Q5 consumer patch first" >&2; exit 2; }

df -h "${CARGO_TARGET_DIR}"
available_kib="$(df -Pk "${CARGO_TARGET_DIR}" | awk 'NR==2 {print $4}')"
if (( available_kib < 8 * 1024 * 1024 )); then
  echo "less than 8 GiB is available for the coordinated target" >&2
  exit 2
fi

artifact_root="${SAGAT_Q5_ARTIFACT_ROOT:-${TMPDIR:-/tmp}/sagat-q5-legacy-output}"
mkdir -p "${artifact_root}"
run_root="$(mktemp -d "${artifact_root}/run.XXXXXX")"
old_source="${run_root}/old-source"
artifacts="${run_root}/artifacts"
logs="${run_root}/logs"
mkdir -p "${old_source}" "${artifacts}" "${logs}"
manifest="${artifacts}/MANIFEST"
active_before=""
producer_started_marker=""
active_phase_pid=""
phase_launching=0
cleanup_receipt_database=""
cleanup_receipt_sha=""
receipt_established=0
receipt_cache_failed=0

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}';
  else shasum -a 256 "$1" | awk '{print $1}'; fi
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

bounded_psql() {
  # The DSN stays in the environment and is never printed into evidence.
  python3 - "$@" <<'PSQL'
import os,select,signal,subprocess,sys,time
child=None
pending=None
stopping=False
shared=os.environ.get('SAGAT_Q5_SUPERVISED')=='1'
def terminate_child():
    global stopping
    if child is None or stopping:return
    stopping=True
    try:
        if shared:child.terminate()
        else:os.killpg(child.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    try:child.wait(timeout=1)
    except subprocess.TimeoutExpired:
        try:
            if shared:child.kill()
            else:os.killpg(child.pid,signal.SIGKILL)
        except ProcessLookupError:pass
        child.wait()
def interrupted(signum,_frame):
    global pending
    pending=signum
signal.signal(signal.SIGTERM,interrupted);signal.signal(signal.SIGINT,interrupted)
env=dict(os.environ,PGCONNECT_TIMEOUT='5',PGOPTIONS='-c statement_timeout=5000 -c lock_timeout=4000')
child=subprocess.Popen(['psql',os.environ['SAGAT_TEST_DATABASE_URL'],'-X','-v','ON_ERROR_STOP=1',*sys.argv[1:]],env=env,start_new_session=not shared)
deadline=time.monotonic()+12
while True:
    if pending is not None:
        terminate_child();raise SystemExit(128+pending)
    if time.monotonic()>=deadline:
        terminate_child();raise SystemExit('bounded PostgreSQL administration timed out')
    finished=os.waitid(os.P_PID,child.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
    if finished is not None:
        if pending is not None:continue
        result=child.wait();break
    select.select([],[],[],min(0.1,max(0,deadline-time.monotonic())))
raise SystemExit(result)
PSQL
}

list_test_databases() {
  bounded_psql -Atq -c "SELECT datname FROM pg_database WHERE datname ~ '^t_[0-9a-f]{32}$' ORDER BY datname"
}

drop_database() {
  local name="$1"
  [[ "${name}" =~ ^t_[0-9a-f]{32}$ ]] || { echo "refusing unsafe database name" >&2; return 1; }
  bounded_psql -q -c "DROP DATABASE IF EXISTS \"${name}\" WITH (FORCE)" >/dev/null
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
  list_test_databases > "${after}" || return 1
  validate_database_snapshot "${after}" || return 1
  local additions="${run_root}/databases.cleanup-owned"
  comm -13 "${active_before}" "${after}" > "${additions}"
  local count
  count="$(wc -l < "${additions}" | tr -d ' ')"
  if [[ "${count}" != 1 ]]; then
    echo "cleanup cannot prove one phase-owned database; retained for inspection" >&2
    return 1
  fi
  local database
  IFS= read -r database < "${additions}"
  [[ "${receipt_cache_failed}" == 0 ]] || return 1
  if [[ -n "${receipt:-}" && -e "${receipt}" ]]; then
    cache_private_receipt || return 1
  fi
  local receipt_database="${cleanup_receipt_database}"
  if [[ "${receipt_established}" == 1 ]]; then
    [[ -n "${receipt_database}" && "${cleanup_receipt_sha}" =~ ^[0-9a-f]{64}$ ]] || {
      echo "established receipt has no valid cached identity; refusing fallback deletion" >&2; return 1;
    }
  fi
  if [[ -n "${receipt_database}" ]]; then
    if [[ "${receipt_database}" != "${database}" ]]; then
      echo "receipt disagrees with sole new database; refusing deletion" >&2
      return 1
    fi
  fi
  drop_database "${database}" || return 1
  active_before=""
  producer_started_marker=""
}

cache_private_receipt() {
  receipt_established=1
  receipt_cache_failed=1
  local checked_database checked_sha
  checked_database="$(receipt_database_name "${receipt}")" || return 1
  checked_sha="$(sha256 "${receipt}")" || return 1
  [[ "${checked_sha}" =~ ^[0-9a-f]{64}$ ]] || return 1
  printf '%s-receipt-sha256 %s\n%s-database %s\n' \
    "${role}" "${checked_sha}" "${role}" "${checked_database}" >> "${manifest}" || return 1
  cleanup_receipt_database="${checked_database}"
  cleanup_receipt_sha="${checked_sha}"
  receipt_cache_failed=0
}

remove_private_receipts() {
  python3 - "${run_root}" <<'PRIVATE'
from pathlib import Path
import sys
root=Path(sys.argv[1])
for role in ('owner','runtime'):
    for name in ('receipt.json','receipt.json.tmp','receipt.tmp'):
        (root/role/name).unlink(missing_ok=True)
PRIVATE
}

cleanup() {
  local status=$?
  trap - INT TERM
  terminate_active_phase
  local receipt_valid=1
  [[ "${receipt_cache_failed}" == 0 ]] || receipt_valid=0
  if [[ -n "${receipt:-}" && -e "${receipt}" ]]; then
    cache_private_receipt || receipt_valid=0
  fi
  # Remove all exact owned credential paths before database administration or
  # archive removal: the outer termination deadline can interrupt either.
  remove_private_receipts
  if [[ "${receipt_valid}" != 1 ]] || ! cleanup_new_databases; then
    echo "owned database cleanup failed; inspect retained artifacts" >&2; status=1
  fi
  python3 - "${old_source}" <<'SOURCE'
from pathlib import Path
import shutil,sys
source=Path(sys.argv[1])
if source.exists():shutil.rmtree(source)
SOURCE
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
import select
import time
import subprocess
import sys

timeout = int(sys.argv[1])
marker = sys.argv[2]
child = None
pending_signal = None
stopping = False
# These native fixtures use embedded Tokio servers and external PostgreSQL;
# they spawn no OS children. The outer supervisor owns their collective group.
shared = os.environ.get('SAGAT_Q5_SUPERVISED') == '1'

def terminate_child():
    global stopping
    if child is None or stopping:return
    stopping=True
    try:
        if shared:child.terminate()
        else:os.killpg(child.pid,signal.SIGTERM)
    except ProcessLookupError:pass
    try:child.wait(timeout=1 if shared else 5)
    except subprocess.TimeoutExpired:
        try:
            if shared:child.kill()
            else:os.killpg(child.pid,signal.SIGKILL)
        except ProcessLookupError:pass
        child.wait()

def interrupted(signum, _frame):
    global pending_signal
    # Never reap inside a signal handler: Popen may hold its waitpid lock.
    pending_signal = signum

signal.signal(signal.SIGINT, interrupted)
signal.signal(signal.SIGTERM, interrupted)
child = subprocess.Popen(sys.argv[3:], start_new_session=not shared)
try:
    if marker != '-':
        temporary = Path(marker + '.tmp')
        temporary.write_text(str(child.pid), encoding='ascii')
        os.replace(temporary, marker)
except BaseException:
    terminate_child()
    raise
deadline=time.monotonic()+timeout
while True:
    if pending_signal is not None:
        terminate_child();raise SystemExit(128+pending_signal)
    if time.monotonic()>=deadline:
        terminate_child();raise SystemExit(f'timed out after {timeout}s')
    finished=os.waitid(os.P_PID,child.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
    if finished is not None:
        if pending_signal is not None:continue
        result=child.wait();break
    select.select([],[],[],min(0.1,max(0,deadline-time.monotonic())))
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

receipt_database_name() {
  python3 - "$1" "${fixture_case}" "${role}" <<'RECEIPT'
import json,re,stat,sys
from pathlib import Path
p=Path(sys.argv[1])
if stat.S_IMODE(p.stat().st_mode)!=0o600:raise SystemExit('receipt is not private')
v=json.loads(p.read_text())
case=sys.argv[2]
if case!='output-view':
    fields='format source_commit case role database_name database_url runtime_database_url tenant other_tenant namespace run workflow_version scope_id scheduler_owner handoff_run_version closed_source_cutoff producer_witness_sha256 closed_source_sha256'.split()
    if case in ('nonfinal-return','nonfinal-return-red'):
        fields+=['forward_step_id','forward_target_key'];fmt='legacy-nonfinal-return-frontier-v1';kind='nonfinal-return'
    elif case in ('free-admission','free-admission-red'):
        fmt='legacy-free-admission-frontier-v1';kind='free-admission'
    elif case=='free-admission-equal-limit':
        fmt='legacy-free-admission-equal-limit-v1';kind='free-admission-equal-limit'
    elif case=='failed-member':
        fmt='legacy-failed-member-frontier-v1';kind='failed-member'
    else:raise SystemExit('unknown private receipt case')
    if set(v)!=set(fields) or v['format']!=fmt or v['source_commit']!='6c44a0e49670cf19660792cd1518d241e6c99b3b' or v['case']!=kind or v['role']!=sys.argv[3]:raise SystemExit('invalid private frontier receipt envelope')
name=v.get('database_name')
if not isinstance(name,str) or re.fullmatch(r't_[0-9a-f]{32}',name) is None:raise SystemExit('unsafe receipt database')
print(name)
RECEIPT
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

# The archive hash and complete extracted-file manifest establish the immutable production base.
git -C "${repo_root}" archive --format=tar "${old_commit}" > "${artifacts}/old-source.tar"
tar -xf "${artifacts}/old-source.tar" -C "${old_source}"
source_manifest "${old_source}" > "${artifacts}/source.before"
source_manifest "${old_source}/crates/infra-postgres/migrations" > "${artifacts}/old-migrations.before"
cp "${old_source}/tests/integration/workflow-primitives/main.rs" "${artifacts}/old-main.original.rs"
cp "${producer_overlay}" \
  "${old_source}/tests/integration/workflow-primitives/legacy_collection_output_producer.rs"
python3 - "${old_source}/tests/integration/workflow-primitives/main.rs" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
anchor = 'mod collection_pipeline;\n'
if text.count(anchor) != 1 or 'mod legacy_collection_output_producer;' in text:
    raise SystemExit('unexpected frozen integration main shape')
path.write_text(text.replace(anchor, 'mod legacy_collection_output_producer;\n' + anchor))
PY
diff -u "${artifacts}/old-main.original.rs" \
  "${old_source}/tests/integration/workflow-primitives/main.rs" \
  > "${artifacts}/test-overlay-main.diff" || true

# The old test binary is the only build from the overlaid archive. Builds remain sequential.
# Archive timestamps can predate current same-version package fingerprints in the shared target.
# Invalidate source freshness in both directions without changing any archived bytes.
find "${old_source}/crates" "${old_source}/apps" "${old_source}/tests" \
  -type f -name '*.rs' -exec touch {} +
old_build_json="${artifacts}/old-build.jsonl"
(
  cd "${old_source}"
  SAGAT_Q5_SOURCE_COMMIT="${old_commit}" CARGO_INCREMENTAL=0 \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
    cargo test --locked --no-run --message-format=json \
      --manifest-path tests/Cargo.toml --test integration_workflow_primitives
) > "${old_build_json}"
old_built="$(find_test_binary "${old_build_json}" integration_workflow_primitives)"
old_binary="${artifacts}/integration-workflow-primitives-old"
install -m 0755 "${old_built}" "${old_binary}"

# Remove the test overlay and prove every archived byte is restored after the build.
cp "${artifacts}/old-main.original.rs" \
  "${old_source}/tests/integration/workflow-primitives/main.rs"
python3 - "${old_source}/tests/integration/workflow-primitives/legacy_collection_output_producer.rs" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).unlink()
PY
source_manifest "${old_source}" > "${artifacts}/source.after"
source_manifest "${old_source}/crates/infra-postgres/migrations" \
  > "${artifacts}/old-migrations.after"
cmp "${artifacts}/source.before" "${artifacts}/source.after"
cmp "${artifacts}/old-migrations.before" "${artifacts}/old-migrations.after"

# Shared-target package fingerprints may otherwise point at the same-version archived sources.
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

# The source hashes cover the two untracked test files that git diff HEAD cannot contain.
{
  echo "fixture-case ${fixture_case}"
  echo "old-exact-test ${old_test}"
  echo "current-exact-test ${current_test}"
  echo "old-commit ${old_commit}"
  echo "current-commit $(git -C "${repo_root}" rev-parse HEAD)"
  echo "current-diff-sha256 $(sha256 "${artifacts}/current.diff")"
  echo "current-consumer-source-sha256 $(sha256 "${consumer_source}")"
  echo "coordinator-source-sha256 $(sha256 "${coordinator_source}")"
  echo "old-archive-sha256 $(sha256 "${artifacts}/old-source.tar")"
  echo "old-source-manifest-sha256 $(sha256 "${artifacts}/source.before")"
  echo "restored-source-manifest-sha256 $(sha256 "${artifacts}/source.after")"
  echo "old-migrations-manifest-sha256 $(sha256 "${artifacts}/old-migrations.before")"
  echo "restored-old-migrations-manifest-sha256 $(sha256 "${artifacts}/old-migrations.after")"
  echo "test-overlay-producer-sha256 $(sha256 "${producer_overlay}")"
  echo "test-overlay-main-diff-sha256 $(sha256 "${artifacts}/test-overlay-main.diff")"
  echo "old-rust-toolchain-sha256 $(sha256 "${old_source}/rust-toolchain.toml")"
  echo "old-cargo-lock-sha256 $(sha256 "${old_source}/Cargo.lock")"
  echo "current-rust-toolchain-sha256 $(sha256 "${repo_root}/rust-toolchain.toml")"
  echo "current-cargo-lock-sha256 $(sha256 "${repo_root}/Cargo.lock")"
  echo "old-rustc-version $(cd "${old_source}" && rustc --version --verbose | awk '/release:/{print $2}')"
  echo "old-rustc-commit $(cd "${old_source}" && rustc --version --verbose | awk '/commit-hash:/{print $2}')"
  echo "old-test-binary-sha256 $(sha256 "${old_binary}")"
  echo "current-test-binary-sha256 $(sha256 "${current_binary}")"
} > "${manifest}"

old_list="${artifacts}/old-tests.list"
current_list="${artifacts}/current-tests.list"
"${old_binary}" --list > "${old_list}"
"${current_binary}" --list > "${current_list}"
grep -Fxq "${old_test}: test" "${old_list}" || { echo "old producer test is not registered" >&2; exit 1; }
grep -Fxq "${current_test}: test" "${current_list}" || { echo "current consumer test is not registered" >&2; exit 1; }

for role in owner runtime; do
  receipt_established=0
  receipt_cache_failed=0
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
  export SAGAT_Q5_PHASE_RECEIPT="${receipt}"

  producer_log="${logs}/${role}-producer.log"
  producer_status=0
  run_bounded "${phase_seconds}" "${producer_started_marker}" \
    "${old_binary}" --exact "${old_test}" --nocapture > "${producer_log}" 2>&1 || producer_status=$?
  [[ -f "${producer_started_marker}" ]] || {
    echo "old producer was not launched; cleanup remains disarmed" >&2; exit 1;
  }
  if [[ -n "${expected_red_marker}" ]]; then
    if [[ "${producer_status}" != 101 ]] || ! grep -Fq "${expected_red_marker}" "${producer_log}" || ! grep -Fq 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' "${producer_log}"; then
      cat "${producer_log}" >&2
      echo "not the exact expected old frontier RED" >&2; exit 1
    fi
    echo "${role}-expected-red-exit 101" >> "${manifest}"
    echo "${role}-expected-red-log-sha256 $(sha256 "${producer_log}")" >> "${manifest}"
    cleanup_new_databases
    continue
  fi
  if [[ "${producer_status}" != 0 ]] || ! grep -Fq 'test result: ok. 1 passed; 0 failed; 0 ignored;' "${producer_log}"; then
    cat "${producer_log}" >&2; exit 1
  fi
  receipt_established=1
  [[ -s "${receipt}" ]] || { echo "old producer wrote no receipt" >&2; exit 1; }
  database_name="$(receipt_database_name "${receipt}")"
  after_producer="${role_root}/databases.after-producer"
  list_test_databases > "${after_producer}"
  validate_database_snapshot "${after_producer}"
  comm -13 "${active_before}" "${after_producer}" | grep -Fxq "${database_name}" || {
    echo "receipt database was not created by this phase" >&2; exit 1;
  }
  [[ "$(comm -13 "${active_before}" "${after_producer}" | wc -l | tr -d ' ')" == 1 ]] || {
    echo "old producer created an unexpected number of databases" >&2; exit 1;
  }

  consumer_log="${logs}/${role}-consumer.log"
  if ! run_bounded "${phase_seconds}" - \
    "${current_binary}" --ignored --exact "${current_test}" --nocapture \
    > "${consumer_log}" 2>&1; then
    cat "${consumer_log}" >&2
    exit 1
  fi
  grep -Fq "test result: ok. 1 passed; 0 failed;" "${consumer_log}" || {
    echo "current consumer did not execute exactly one passing matrix test" >&2; exit 1;
  }
  cache_private_receipt
  remove_private_receipts
  completed_marker="${producer_started_marker}"
  cleanup_new_databases
  python3 - "${completed_marker}" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).unlink(missing_ok=True)
PY
  cleanup_receipt_database=""
  cleanup_receipt_sha=""
done
unset SAGAT_TEST_AS_RUNTIME_ROLE SAGAT_Q5_PHASE_RECEIPT || true

echo "Q5 ${fixture_case} evidence: ${run_root}"
