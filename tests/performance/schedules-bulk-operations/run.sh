#!/usr/bin/env bash
# Runs the benchmark of feature 018 (tests/performance/schedules-bulk-operations/main.rs) against a PostgreSQL of its own, started
# here with the configuration the report declares, repeats every scenario, records the load of the machine before and after each
# run, and leaves the fragments the report (tests/performance/reports/schedules-bulk-operations.md) is written from.
#
#   tests/performance/schedules-bulk-operations/run.sh                       # every scenario, three repetitions each
#   SAGAT_PERF_REPS=5 SAGAT_PERF_ONLY=claim tests/performance/schedules-bulk-operations/run.sh
#   SAGAT_PERF_OUT=/some/directory ...                                         # where the logs and fragments go
#
# It is never run by CI: a pull request runs this target without `--ignored`, so only the unit tests of its helpers run there.
# Needs: Docker (the PostgreSQL image is pinned by digest; the container is removed when the script ends), a Rust toolchain,
# python3. Set CARGO_TARGET_DIR to a directory outside the repository. A full run takes a few hours on a laptop and about
# 10 GiB of disk (the databases of a repetition are dropped when it ends); check `df -h` first.
set -euo pipefail
cd "$(dirname "$0")/../../.."

# A benchmark that runs in a pull request's CI is a defect: the workflow does not call this script, and it refuses if it is called there.
if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
  echo "run.sh is a benchmark of hours: it is not run by CI" >&2
  exit 1
fi

REPS="${SAGAT_PERF_REPS:-3}"
ONLY="${SAGAT_PERF_ONLY:-}"
IMAGE="${SAGAT_PERF_PG_IMAGE:-postgres@sha256:cf78e76683b9ca8c5733cbbdce6c9262b45b6767934dd0a95e671f9a0fc20685}"
OUT="${SAGAT_PERF_OUT:-$(mktemp -d "${TMPDIR:-/tmp}/sagat-perf-018-XXXXXX")}"
NAME="sagat-perf-018-$$"
mkdir -p "$OUT/logs" "$OUT/fragments"

# The first loopback port from 55618 up that nothing holds.
PORT="$(python3 - <<'PY'
import socket
port = 55618
while True:
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", port))
        s.close()
        print(port)
        break
    except OSError:
        s.close()
        port += 1
PY
)"

cleanup() { docker rm -f -v "$NAME" >/dev/null 2>&1 || true; }
trap cleanup EXIT

# The PostgreSQL the benchmark runs against: the image defaults (fsync on, synchronous_commit on, shared_buffers 128MB) except
# max_connections, which the scenarios need (a pool of 48 for the 32-way claim, two replicas of 16, the owner pool). Loopback only.
docker run -d --name "$NAME" -p "127.0.0.1:$PORT:5432" -e POSTGRES_PASSWORD=postgres --shm-size=1g "$IMAGE" \
  -c max_connections=200 >/dev/null
for _ in $(seq 1 60); do
  if docker exec "$NAME" pg_isready -U postgres >/dev/null 2>&1 \
     && docker exec "$NAME" psql -U postgres -tAc 'SELECT 1' >/dev/null 2>&1; then break; fi
  sleep 1
done
URL="postgres://postgres:postgres@127.0.0.1:$PORT/postgres"

{
  echo "date (UTC): $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "commit: $(git rev-parse HEAD)  status: $(git status --short | wc -l | tr -d ' ') changed paths"
  echo "uname: $(uname -a)"
  echo "hw.ncpu / hw.memsize: $(sysctl -n hw.ncpu hw.memsize 2>/dev/null | tr '\n' ' ')"
  echo "docker: $(docker info --format '{{.NCPU}} CPUs, {{.MemTotal}} bytes, {{.OperatingSystem}}, server {{.ServerVersion}}, {{.Driver}}')"
  echo "image: $IMAGE"
  echo "container: $NAME, port $PORT, --shm-size=1g, -c max_connections=200"
  echo "postgres: $(docker exec "$NAME" psql -U postgres -tAc 'SELECT version()')"
  docker exec "$NAME" psql -U postgres -tAc \
    "SELECT name || ' = ' || setting || coalesce(unit, '') FROM pg_settings WHERE name IN ('shared_buffers','max_connections','work_mem','effective_cache_size','synchronous_commit','fsync','full_page_writes','max_wal_size','wal_level','random_page_cost','checkpoint_timeout','wal_buffers') ORDER BY name"
  echo "rustc: $(rustc --version)   cargo: $(cargo --version)"
  echo "repetitions: $REPS"
} | tee "$OUT/environment.txt"

echo "== build (release)" >&2
cargo test -p sagat-tests --test perf_schedules_bulk_operations --release --locked --no-run 2>&1 | tee "$OUT/build.log" >&2
BIN="$(sed -n 's/.*Executable .*(\(.*\))$/\1/p' "$OUT/build.log" | tail -1)"
test -x "$BIN"
echo "binary: $BIN" | tee -a "$OUT/environment.txt"

snapshot() { # label repetition before|after
  {
    echo "--- $1 repetition $2 $3: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    uptime
    docker stats --no-stream --format 'postgres container: cpu {{.CPUPerc}}  memory {{.MemUsage}}  block io {{.BlockIO}}' "$NAME"
  } >> "$OUT/load.log"
}

drop_databases() {
  for database in $(docker exec "$NAME" psql -U postgres -tAc "SELECT datname FROM pg_database WHERE datname LIKE 't\\_%'"); do
    docker exec "$NAME" psql -U postgres -qc "DROP DATABASE \"$database\" WITH (FORCE)" >/dev/null
  done
}

scenario() { # label test-name [VAR=value ...]
  local label="$1" test="$2"
  shift 2
  # SAGAT_PERF_ONLY=claim runs every scenario whose label contains "claim"; SAGAT_PERF_ONLY==claim (a leading =) only that label
  if [ -n "$ONLY" ]; then
    if [[ "$ONLY" == =* ]]; then
      [ "$label" = "${ONLY#=}" ] || return 0
    else
      [[ "$label" == *"$ONLY"* ]] || return 0
    fi
  fi
  local rep
  for rep in $(seq 1 "$REPS"); do
    echo "== $label, repetition $rep of $REPS" >&2
    snapshot "$label" "$rep" before
    local began=$SECONDS status=0
    env SAGAT_TEST_DATABASE_URL="$URL" SAGAT_TEST_AS_RUNTIME_ROLE=1 SAGAT_PERF_FRAGMENTS="$OUT/fragments" \
        SAGAT_PERF_PG_IMAGE="$IMAGE" "$@" \
        "$BIN" --ignored --nocapture --test-threads=1 --exact "$test" >"$OUT/logs/$label.$rep.log" 2>&1 || status=$?
    snapshot "$label" "$rep" after
    printf '%s\t%s\t%s\t%s\n' "$label" "$rep" "$status" "$((SECONDS - began))" >> "$OUT/status.tsv"
    drop_databases
  done
}

# The sizes of the report. A scenario that breaks a bound exits non-zero after it wrote its fragment: the run goes on, and
# status.tsv says which repetitions failed.
scenario firing-instant        firing_lag_all_due_at_the_same_instant               SAGAT_PERF_SCHEDULES=1000,10000
scenario firing-spread         firing_lag_spread_over_a_minute                      SAGAT_PERF_SCHEDULES=1000,10000
scenario firing-two-replicas   firing_lag_two_replicas_all_due_at_the_same_instant  SAGAT_PERF_SCHEDULES=1000,10000
scenario batch-one-default     batch_one_job_at_the_default_rate                    SAGAT_PERF_BATCH_RUNS=120
scenario batch-one-highest     batch_one_job_at_the_highest_rate                    SAGAT_PERF_BATCH_RUNS=1000
scenario batch-selection       batch_selection_and_drain_of_many_runs               SAGAT_PERF_BATCH_SELECTION=1000,10000
scenario batch-16-default      batch_sixteen_jobs_at_the_default_rate               SAGAT_PERF_BATCH_EACH=300
scenario batch-16-highest      batch_sixteen_jobs_at_the_highest_rate               SAGAT_PERF_BATCH_EACH=500
scenario batch-big-and-small   batch_one_big_job_beside_fifteen_small_ones          SAGAT_PERF_BATCH_EACH=30
scenario claim                 business_id_claim_thirty_two_way                     SAGAT_PERF_CLAIM_ROUNDS=50
scenario reset                 reset_at_the_sizes_of_the_ceiling                    SAGAT_PERF_RESET_EVENTS=1000,5000,20000
scenario triggers-steady       trigger_handover_steady_state                        SAGAT_PERF_TRIGGERS=10000,100000
scenario triggers-backlog      trigger_handover_backlog_after_an_upgrade            SAGAT_PERF_TRIGGERS=10000,100000 SAGAT_PERF_TRIGGER_SECONDS=60

# Between the two sizes of the steady state, where the handover check on a converged fleet was found to depend on the size.
scenario triggers-steady-mid   trigger_handover_steady_state                        SAGAT_PERF_TRIGGERS=20000,50000
scenario triggers-steady-small trigger_handover_steady_state                        SAGAT_PERF_TRIGGERS=5000,15000

# Two variants of the settings, to see what a setting changes: the batch loop polling every 50 ms instead of 1,000 (a lone job at the
# default rate), and the schedule pass with the hard ceilings of its round (1,000 actions, 500 schedules) at 10,000 schedules.
scenario batch-one-default-poll50 batch_one_job_at_the_default_rate             SAGAT_PERF_BATCH_RUNS=120 SAGAT_PERF_BATCH_POLL_MS=50
scenario firing-instant-round1000 firing_lag_all_due_at_the_same_instant        SAGAT_PERF_SCHEDULES=10000 SAGAT_PERF_SCHEDULES_ROUND_ACTIONS=1000 SAGAT_PERF_SCHEDULES_BATCH=500

echo "== summary" >&2
python3 -I "$(dirname "$0")/summarize.py" "$OUT" | tee "$OUT/summary.md"
echo "logs, fragments and the summary are in $OUT" >&2
