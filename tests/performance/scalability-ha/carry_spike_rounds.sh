#!/usr/bin/env bash
# Rounds of design 041's spike T041-35, second run (one-call carry, OD-2026-11-20-1), each one invocation of run.sh on a
# fresh database. On a dedicated runner (SAGAT_SPIKE_DEDICATED=1, the declared environment of OD-2026-11-20-6) nothing
# else runs, so a round begins at once and counts as quiet; elsewhere it waits at most 45 minutes for a 1-minute load
# under 3 and is marked not quiet otherwise. Stops once three rounds are valid (carry_spike_judge.py) or after ROUNDS.
#   OUT=<dir> tests/performance/scalability-ha/carry_spike_rounds.sh
set -uo pipefail
cd "$(dirname "$0")/../../.."
OUT="${OUT:?set OUT to the directory for the rounds}"
mkdir -p "$OUT"
export SAGAT_PERF_STEP_LOG_SPIKE=1 SAGAT_PERF_STEP_LOG_CARRY=1 SAGAT_PERF_STEP_LOG_CARRY_RUN=2
export SAGAT_PERF_SCENARIOS=step_log_carry_spike SAGAT_PERF_CARGO_FEATURES=perf-spike
load1() { awk '{print $1}' /proc/loadavg 2>/dev/null || sysctl -n vm.loadavg | awk '{print $2}'; }
cpu() { (lscpu 2>/dev/null | sed -n 's/^Model name: *//p') || sysctl -n machdep.cpu.brand_string; }
for round in $(seq "${START:-1}" "${ROUNDS:-6}"); do
  waited=0; quiet=1
  if [ "${SAGAT_SPIKE_DEDICATED:-0}" != 1 ]; then
    while awk "BEGIN{exit !($(load1) >= 3)}"; do
      if [ $waited -ge 2700 ]; then quiet=0; break; fi
      sleep 15; waited=$((waited+15))
    done
  fi
  echo "$quiet" > "$OUT/second-round-$round.quiet"
  echo "round $round: begin after ${waited}s, quiet=$quiet, nproc $(nproc 2>/dev/null || sysctl -n hw.ncpu), cpu $(cpu), uptime $(uptime)" | tee -a "$OUT/rounds.log"
  SAGAT_PERF_PGLOG="$OUT/second-round-$round.pglog" SAGAT_PERF_REPORT="$OUT/second-round-$round.md" tests/performance/scalability-ha/run.sh > "$OUT/second-round-$round.log" 2>&1
  echo "round $round: end exit $?, uptime $(uptime)" | tee -a "$OUT/rounds.log"
  valid=$(python3 -I tests/performance/scalability-ha/carry_spike_judge.py "$OUT" --count-valid 2>/dev/null || echo 0)
  echo "round $round: valid rounds so far $valid" | tee -a "$OUT/rounds.log"
  [ "$valid" -ge 3 ] && break
done
echo "rounds done" | tee -a "$OUT/rounds.log"
