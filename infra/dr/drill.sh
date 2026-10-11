#!/usr/bin/env bash
# The disaster-recovery drill of feature 010 (FR-010-007, T010-031): it makes the two disasters a
# single-region deployment has to survive, with real PostgreSQL clusters started here, and measures
# what the plan promises.
#
#   1. Point-in-time recovery. A workload runs while a base backup is taken, a restore point is
#      named, more work is done, and an operator's mistake destroys the schema. A new cluster is
#      restored from the base backup and the archived WAL to the restore point. The journal digest
#      (infra/dr/journal_digest.sql) before and after must be equal, and the journal of every run
#      must replay to its projections. Measured: the time to restore (RTO of this procedure) and what
#      the restore discards (the work after the point).
#   2. Failover to a standby. A primary with a streaming standby is killed with `kill -9` while the
#      platform acknowledges runs as fast as it can. The standby is promoted. Every run the platform
#      acknowledged must be there, if the standby was synchronous. Measured: the time to promote
#      (RTO of the database) and how many acknowledged runs an asynchronous standby loses (RPO).
#
#   infra/dr/drill.sh                      # the report goes to tests/performance/reports/disaster-recovery.md
#   SAGAT_DR_RUNS=20000 infra/dr/drill.sh  # a larger population
#
# It needs PostgreSQL's server binaries (PGBIN), psql, a Rust toolchain and about 1 GiB of disk. The
# clusters, the archive and the backup are made in a temporary directory and removed at the end
# (SAGAT_DR_KEEP=1 keeps them).
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"

find_pgbin() {
  local dir
  for dir in "${PGBIN:-}" "$(pg_config --bindir 2>/dev/null || true)" \
             /opt/homebrew/opt/postgresql@*/bin /usr/lib/postgresql/*/bin /usr/local/opt/postgresql@*/bin; do
    if [ -n "$dir" ] && [ -x "$dir/initdb" ] && [ -x "$dir/postgres" ]; then echo "$dir"; return 0; fi
  done
  echo "no PostgreSQL server binaries found: set PGBIN" >&2; return 1
}
PGBIN="$(find_pgbin)"
export PATH="$PGBIN:$PATH"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/sagat-dr-XXXXXX")"
REPORT="${SAGAT_DR_REPORT:-tests/performance/reports/disaster-recovery.md}"
RUNS="${SAGAT_DR_RUNS:-3000}"
BASE_PORT="${SAGAT_DR_BASE_PORT:-55460}"
case "$RUNS$BASE_PORT" in *[!0-9]*) echo "SAGAT_DR_RUNS and SAGAT_DR_BASE_PORT are numbers" >&2; exit 2;; esac
PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do pkill -P "$pid" 2>/dev/null || true; kill "$pid" 2>/dev/null || true; done
  for data in "$WORK"/*/data; do
    [ -f "$data/postmaster.pid" ] && pg_ctl -D "$data" -m immediate stop >/dev/null 2>&1 || true
  done
  [ "${SAGAT_DR_KEEP:-}" = 1 ] && echo "kept: $WORK" || rm -rf "$WORK"
}
trap cleanup EXIT

# The clusters listen on the loopback of a machine others may use, so they ask for a password: a random
# one that exists in this process, in the environment of what it starts, and in a file only this user
# reads (the standby's walreceiver reads the same file).
PASSWORD="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
(umask 077; printf '%s' "$PASSWORD" > "$WORK/password"; printf '127.0.0.1:*:*:postgres:%s\n' "$PASSWORD" > "$WORK/pgpass")
export PGPASSWORD="$PASSWORD" PGPASSFILE="$WORK/pgpass"

now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time'; }
log() { echo "[$(date +%H:%M:%S)] $*" >&2; }
psqlc() { psql -X -At -h 127.0.0.1 -U postgres "$@"; }

# A cluster with the settings every case shares. $1 name, $2 port, the rest extra settings.
make_cluster() {
  local name="$1" port="$2"; shift 2
  local data="$WORK/$name/data"
  mkdir -p "$WORK/$name"
  initdb -D "$data" -U postgres --auth=scram-sha-256 --pwfile="$WORK/password" --encoding=UTF8 --locale=C >/dev/null
  {
    echo "port = $port"
    echo "listen_addresses = '127.0.0.1'"
    echo "unix_socket_directories = '$WORK/$name'"
    echo "max_connections = 200"
    echo "shared_buffers = 512MB"
    echo "wal_level = replica"
    echo "max_wal_senders = 5"
    echo "hot_standby = on"
    echo "fsync = on"
    echo "synchronous_commit = on"
    for setting in "$@"; do echo "$setting"; done
  } >> "$data/postgresql.conf"
  echo "$data"
}
start() { pg_ctl -D "$1" -l "$1/../log" -w start >/dev/null; }

run_test() { # $1 test name; the environment carries the rest
  cargo test -p sagat-tests --test perf_scalability_ha --release -- --ignored --nocapture --exact "dr::$1"
}

SUMMARY=""
add() { SUMMARY+="$1"$'\n'; }

# ----------------------------------------------------------------------------- 1. point in time
log "1/2 point-in-time recovery"
P1=$BASE_PORT
ARCH="$WORK/archive"; mkdir -p "$ARCH"
D1=$(make_cluster primary "$P1" "archive_mode = on" \
  "archive_command = 'test ! -f $ARCH/%f && cp %p $ARCH/%f'" "archive_timeout = 30s" "max_wal_size = 2GB")
start "$D1"
export SAGAT_TEST_DATABASE_URL="postgres://postgres:$PASSWORD@127.0.0.1:$P1/postgres"
export SAGAT_DR_DIR="$WORK/signals"; mkdir -p "$SAGAT_DR_DIR"
export SAGAT_DR_RUNS="$RUNS"
run_test dr_workload > "$WORK/workload.log" 2>&1 &
PIDS+=($!)
workload=$!
wait_for() { local f="$SAGAT_DR_DIR/$1" n=0; until [ -e "$f" ]; do sleep 0.2; n=$((n+1)); [ $n -lt 9000 ] || { echo "timeout waiting for $1" >&2; tail -20 "$WORK/workload.log" >&2; return 1; }; done; }
wait_for a.done
DB=$(cat "$SAGAT_DR_DIR/db")
log "phase A done: database $DB; taking the base backup while phase B runs"
t_backup=$(now)
touch "$SAGAT_DR_DIR/backup.started"
pg_basebackup -h 127.0.0.1 -p "$P1" -U postgres -D "$WORK/base" -Fp -X stream -c fast >/dev/null
t_backup_done=$(now)
wait_for b.done
log "phase B done; naming the restore point"
psqlc -p "$P1" -d "$DB" -c "SELECT pg_create_restore_point('drill_target')" >/dev/null
digest_before=$(psqlc -p "$P1" -d "$DB" -f infra/dr/journal_digest.sql)
runs_at_target=$(echo "$digest_before" | sed -n 's/^runs=//p')
events_at_target=$(echo "$digest_before" | sed -n 's/^journal_events=//p')
touch "$SAGAT_DR_DIR/target.set"
wait_for c.done
runs_after=$(psqlc -p "$P1" -d "$DB" -c "SELECT count(*) FROM runs")
log "phase C done ($((runs_after - runs_at_target)) runs after the point); the operator's mistake"
PGOPTIONS="-c client_min_messages=warning" psqlc -p "$P1" -d "$DB" -c "DROP SCHEMA public CASCADE" >/dev/null
last_wal=$(psqlc -p "$P1" -c "SELECT pg_walfile_name(pg_current_wal_lsn())")
psqlc -p "$P1" -c "SELECT pg_switch_wal()" >/dev/null
for _ in $(seq 1 120); do
  archived=$(psqlc -p "$P1" -c "SELECT coalesce(last_archived_wal, '') FROM pg_stat_archiver")
  [ -n "$archived" ] && [[ ! "$archived" < "$last_wal" ]] && break
  sleep 1
done
touch "$SAGAT_DR_DIR/finish"; wait "$workload" 2>/dev/null || true
pg_ctl -D "$D1" -m immediate stop >/dev/null
archived_segments=$(ls "$ARCH" | grep -vc '\.history$\|\.backup$' || true)

log "restoring to drill_target"
DR="$WORK/restored/data"; mkdir -p "$WORK/restored"
t0=$(now)
cp -R "$WORK/base" "$DR"; chmod 700 "$DR"
rm -f "$DR/pg_wal/"0000* 2>/dev/null || true
P2=$((BASE_PORT + 1))
{
  echo "port = $P2"
  echo "listen_addresses = '127.0.0.1'"
  echo "unix_socket_directories = '$WORK/restored'"
  echo "restore_command = 'cp $ARCH/%f \"%p\"'"
  echo "recovery_target_name = 'drill_target'"
  echo "recovery_target_action = 'promote'"
} >> "$DR/postgresql.conf"
touch "$DR/recovery.signal"
t_copied=$(now)
pg_ctl -D "$DR" -l "$WORK/restored/log" -w start >/dev/null
for _ in $(seq 1 600); do
  [ "$(psqlc -p "$P2" -d "$DB" -c 'SELECT pg_is_in_recovery()' 2>/dev/null || echo t)" = f ] && break
  sleep 0.1
done
t_up=$(now)
digest_after=$(psqlc -p "$P2" -d "$DB" -f infra/dr/journal_digest.sql)
t_digest=$(now)
if [ "$digest_before" = "$digest_after" ]; then identical=yes; else identical=NO; fi
log "journal digest identical: $identical"
verify=$(SAGAT_DR_URL="postgres://postgres:$PASSWORD@127.0.0.1:$P2/$DB" run_test dr_verify 2>&1 | sed -n 's/^verified=/verified=/p' | head -1)
t_verified=$(now)
pg_ctl -D "$DR" -m fast stop >/dev/null

secs() { perl -e "printf '%.1f', $2 - $1"; }
add "### 1. Point-in-time recovery"
add ""
add "| Step | Result |"
add "|---|---|"
add "| Population | $RUNS runs before the base backup, $RUNS during it, $((RUNS / 2)) after the restore point (a third of the first two phases carried through their activity) |"
add "| Base backup, taken while phase B ran | $(secs "$t_backup" "$t_backup_done") s (\`pg_basebackup -X stream\`) |"
add "| WAL segments archived at the disaster | $archived_segments (16 MiB each; \`archive_timeout = 30s\`) |"
add "| The mistake | \`DROP SCHEMA public CASCADE\` after the restore point, on the primary |"
add "| Restored to | the named restore point \`drill_target\`: $runs_at_target runs, $events_at_target journal events |"
add "| Copy of the base backup | $(secs "$t0" "$t_copied") s |"
add "| WAL replay and promotion, until the database answers | $(secs "$t_copied" "$t_up") s |"
add "| **Time to restore** (copy, replay, promote) | **$(secs "$t0" "$t_up") s** |"
add "| Journal digest before and after | **$identical** (counts and sha256 of the journal and of the audit trail) |"
add "| Replay of every run against its projections | \`${verify:-not run}\` |"
add "| Discarded by the restore | the $((runs_after - runs_at_target)) runs of phase C: what is written after the point is gone, by definition |"
add ""
add "Digest before the disaster:"
add ""
add '```'
add "$digest_before"
add '```'
add ""
add "Digest of the restored database:"
add ""
add '```'
add "$digest_after"
add '```'
[ "$identical" = yes ] || { echo "THE RESTORED JOURNAL DIFFERS" >&2; echo "$SUMMARY"; exit 1; }
case "$verify" in *"failed=0"*) ;; *) echo "THE RESTORED RUNS DO NOT REPLAY: $verify" >&2; echo "$SUMMARY"; exit 1;; esac

# ----------------------------------------------------------------------------- 2. failover
failover() { # $1 label, $2 synchronous_standby_names ('' = asynchronous), $3 port offset
  local label="$1" names="$2" off="$3"
  local pp=$((BASE_PORT + off)) sp=$((BASE_PORT + off + 1))
  local sync_conf=""
  [ -n "$names" ] && sync_conf="synchronous_standby_names = '$names'"
  local dp; dp=$(make_cluster "primary-$label" "$pp" "${sync_conf:-synchronous_standby_names = ''}")
  start "$dp"
  local ds="$WORK/standby-$label/data"; mkdir -p "$WORK/standby-$label"
  PGAPPNAME=standby1 pg_basebackup -h 127.0.0.1 -p "$pp" -U postgres -D "$ds" -R -X stream -c fast >/dev/null
  {
    echo "port = $sp"
    echo "unix_socket_directories = '$WORK/standby-$label'"
  } >> "$ds/postgresql.conf"
  chmod 700 "$ds"
  start "$ds"
  for _ in $(seq 1 100); do
    [ -n "$(psqlc -p "$pp" -c "SELECT 1 FROM pg_stat_replication WHERE state = 'streaming'")" ] && break
    sleep 0.2
  done
  local mode; mode=$(psqlc -p "$pp" -c "SELECT sync_state FROM pg_stat_replication LIMIT 1")
  log "2/2 failover, standby $label (the primary reports it as: $mode)"
  export SAGAT_TEST_DATABASE_URL="postgres://postgres:$PASSWORD@127.0.0.1:$pp/postgres"
  export SAGAT_DR_DIR="$WORK/signals-$label"; mkdir -p "$SAGAT_DR_DIR"
  run_test dr_acked_workload > "$WORK/acked-$label.log" 2>&1 &
  local load=$!; PIDS+=($!)
  wait_for workload.ready
  sleep 8
  local pm; pm=$(head -1 "$dp/postmaster.pid")
  local children; children=$(pgrep -P "$pm" || true)
  local t_kill; t_kill=$(now)
  # shellcheck disable=SC2086
  kill -9 "$pm" $children 2>/dev/null || true
  pg_ctl -D "$ds" -w promote >/dev/null
  for _ in $(seq 1 600); do
    [ "$(psqlc -p "$sp" -c 'SELECT pg_is_in_recovery()' 2>/dev/null || echo t)" = f ] && break
    sleep 0.05
  done
  local t_promoted; t_promoted=$(now)
  touch "$SAGAT_DR_DIR/stop"; wait "$load" 2>/dev/null || true
  local db; db=$(cat "$SAGAT_DR_DIR/db")
  local out
  out=$(SAGAT_DR_URL="postgres://postgres:$PASSWORD@127.0.0.1:$sp/$db" SAGAT_DR_ACKED="$SAGAT_DR_DIR/acked" \
        run_test dr_verify 2>&1 | grep -E '^(verified|acknowledged)=' | tr '\n' ' ')
  pg_ctl -D "$ds" -m fast stop >/dev/null
  # (This runs in a subshell, so the row goes to a file the main script reads.)
  echo "| $label | $mode | $(secs "$t_kill" "$t_promoted") s | ${out:-not run} |" >> "$WORK/failover-rows"
  echo "$out"
}
add ""
add "### 2. Failover to a standby (\`kill -9\` of the primary while the platform acknowledges runs)"
add ""
add "| Standby | State reported by the primary | Time to promote (RTO of the database) | After the failover |"
add "|---|---|---:|---|"
sync_out=$(failover synchronous "FIRST 1 (standby1)" 10)
async_out=$(failover asynchronous "" 20)
SUMMARY+="$(cat "$WORK/failover-rows")"$'\n'
add ""
add "What these rows say, and what they do not:"
add ""
add "- The synchronous standby loses nothing, as it must: the primary does not acknowledge a commit before the standby has it, so what the platform acknowledged is there (RPO zero for acknowledged work). The drill fails if that ever stops being true."
add "- The asynchronous standby lost nothing *here*, on a loopback where it is never behind. That does not show that an asynchronous standby is safe: its RPO is the replication lag at the moment of the loss, which a loopback does not have. Measure the lag on your network (\`pg_stat_replication\`) and treat the loss of the last seconds of acknowledged runs as possible."
add "- The time to promote is the database's (the kill until the promoted standby answers). The platform's own recovery after it is the keeper's: another replica takes the dead replica's shards when its lease ends (15 s by default) and the passes are idempotent (docs/runbooks/scaling.md); the engine reconnects by itself."
add ""
case "$sync_out" in *"lost=0"*) ;; *) echo "A SYNCHRONOUS STANDBY LOST ACKNOWLEDGED RUNS: $sync_out" >&2; echo "$SUMMARY" >> "$REPORT"; exit 1;; esac
case "$sync_out" in *"failed=0"*) ;; *) echo "THE PROMOTED RUNS DO NOT REPLAY: $sync_out" >&2; exit 1;; esac

{
  echo "# Disaster recovery drill (feature 010)"
  echo
  echo "Run by \`infra/dr/drill.sh\` on $(date -u +%Y-%m-%d) with $("$PGBIN/postgres" --version), commit $(git rev-parse --short HEAD), on $(uname -srm)."
  echo "The clusters, the archive and the standbys ran on one machine; the numbers are of this procedure on this"
  echo "machine, not an objective for your deployment (see docs/runbooks/disaster-recovery.md for how to run it"
  echo "against yours). A real restore adds the time to fetch the backup from where it is kept."
  echo
  echo "$SUMMARY"
} > "$REPORT"
log "report: $REPORT"
