#!/usr/bin/env bash
# The hot-row diagnosis of design 041's spike T041-35 (D041-105): one invocation of run.sh on a fresh database running
# `step_log_carry_diag` (three repetitions of nine rows at 1 x 16), with WAL I/O timing on. Not a gate.
#   OUT=<dir> tests/performance/scalability-ha/carry_diag_rounds.sh
set -uo pipefail
cd "$(dirname "$0")/../../.."
OUT="${OUT:?set OUT to the directory for the diagnosis}"
mkdir -p "$OUT"
export SAGAT_PERF_STEP_LOG_SPIKE=1 SAGAT_PERF_STEP_LOG_CARRY=1 SAGAT_PERF_STEP_LOG_CARRY_RUN=2 SAGAT_PERF_STEP_LOG_DIAG=1
export SAGAT_PERF_SCENARIOS=step_log_carry_diag SAGAT_PERF_CARGO_FEATURES=perf-spike
export SAGAT_BENCH_PG_EXTRA="-c track_wal_io_timing=on"
echo "diagnosis: begin, nproc $(nproc 2>/dev/null || sysctl -n hw.ncpu), uptime $(uptime)" | tee -a "$OUT/rounds.log"
SAGAT_PERF_REPORT="$OUT/diag.md" tests/performance/scalability-ha/run.sh > "$OUT/diag.log" 2>&1
echo "diagnosis: end exit $?, uptime $(uptime)" | tee -a "$OUT/rounds.log"
