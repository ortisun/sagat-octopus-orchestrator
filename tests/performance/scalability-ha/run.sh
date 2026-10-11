#!/usr/bin/env bash
# Runs the load harness of feature 010 (tests/performance/scalability-ha/main.rs) against a
# PostgreSQL of its own, started here with the configuration the report states, and writes
# tests/performance/reports/scalability-ha.md.
#
#   tests/performance/scalability-ha/run.sh                 # the full run: up to 1 000 000 waits
#   SAGAT_PERF_MAX_WAITS=100000 tests/performance/scalability-ha/run.sh
#
# Needs: PostgreSQL's server binaries (initdb, pg_ctl) on the PATH or in PGBIN, and a Rust toolchain.
# It takes a few minutes per hundred thousand waits and about 2.5 GiB of disk per million.
# Every knob of the harness is an environment variable named in main.rs (SAGAT_PERF_*).
set -euo pipefail
cd "$(dirname "$0")/../../.."

# The directory of the server binaries: the one that has both initdb and postgres.
find_pgbin() {
  local dir
  for dir in "${PGBIN:-}" "$(pg_config --bindir 2>/dev/null || true)" \
             /opt/homebrew/opt/postgresql@*/bin /usr/lib/postgresql/*/bin /usr/local/opt/postgresql@*/bin; do
    if [ -n "$dir" ] && [ -x "$dir/initdb" ] && [ -x "$dir/postgres" ]; then echo "$dir"; return 0; fi
  done
  echo "no PostgreSQL server binaries found: set PGBIN" >&2; return 1
}
PGBIN="$(find_pgbin)"
PORT="${SAGAT_BENCH_PGPORT:-55433}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/sagat-bench-XXXXXX")"
DATA="$WORK/pgdata"
REPORT="${SAGAT_PERF_REPORT:-tests/performance/reports/scalability-ha.md}"

# Whatever happens from here on, the server is stopped and the directory removed.
cleanup() {
  [ -f "$DATA/postmaster.pid" ] && "$PGBIN/pg_ctl" -D "$DATA" -m fast stop >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# The server listens on the loopback of a machine others may use: it asks for a password, a random one
# that exists only in this process and in a file only this user reads.
PASSWORD="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
(umask 077; printf '%s' "$PASSWORD" > "$WORK/password")
"$PGBIN/initdb" -D "$DATA" -U postgres --auth=scram-sha-256 --pwfile="$WORK/password" \
  --encoding=UTF8 --locale=C >/dev/null
"$PGBIN/pg_ctl" -D "$DATA" -l "$WORK/postgres.log" -w \
  -o "-p $PORT -c listen_addresses=127.0.0.1 \
      -c shared_buffers=${SAGAT_BENCH_SHARED_BUFFERS:-2GB} -c effective_cache_size=${SAGAT_BENCH_CACHE:-16GB} \
      -c work_mem=16MB -c max_connections=400 -c max_wal_size=16GB -c checkpoint_timeout=15min \
      -c wal_level=replica -c fsync=on -c synchronous_commit=on -c full_page_writes=on" start

export SAGAT_TEST_DATABASE_URL="postgres://postgres:$PASSWORD@127.0.0.1:$PORT/postgres"
export SAGAT_PERF_REPORT="$REPORT"
cat >"$REPORT" <<HEAD
# Performance and scalability report: feature 010 (scalability and high availability)

Measured by \`tests/performance/scalability-ha/main.rs\`, run by \`tests/performance/scalability-ha/run.sh\`
(\`SAGAT_PERF_MAX_WAITS\` = ${SAGAT_PERF_MAX_WAITS:-1000000}). **These are measurements of one machine on one day, not promises** (\`NFR-010-PERF\`):
each section says what it measured and on what. The machine was shared with other work, which the load
average of each section shows; absolute figures vary from run to run, and the claims that matter are the
structural ones (what grows with what).

The three things the numbers must not be confused with: the **capacity to wait** (how many suspended runs
the database holds, and what each costs), the **active throughput** (step transitions per second), and the
**latency under backlog** (how late a timer fires when many fall due together).
HEAD

for scenario in waits_at_scale firing_scales_with_concurrency_and_replicas a_noisy_tenant_and_a_quiet_one step_transitions_per_second; do
  echo "== $scenario"
  cargo test -p sagat-tests --test perf_scalability_ha --release -- --ignored --nocapture --exact "$scenario"
done
echo "report: $REPORT"
