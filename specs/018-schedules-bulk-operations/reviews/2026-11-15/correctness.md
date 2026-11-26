# Review: feature 018, the automatic handover (ADR-0091) — correctness

- **Doc:** `docs/adr/ADR-0091-a-one-shot-job-hands-the-cron-triggers-over-after-a-deployment.md` (the spec)
- **Reviewed:** the diff of `crates apps infra tests` against the merge base with `develop` (`3268f621`), the working tree of `feature/018-schedules-default-on` on 2026-11-15 after head `19ad4a51` (before the fixes below)
- **Reviewer:** `correctness-reviewer` (independent read-only agent, given the diff and the ADR, not the author's conclusions)
- **Recorded by:** the author of the slice, from the reviewer's full report (summarised by finding, every finding kept)
- **Answered in:** the commit that adds this file (D2307, D2311, G2309 to G2312)

## Findings

| Finding | Severity | How it is answered |
|---|---|---|
| M1 — the wrapper checks convergence under the lock and the move checks it again, inside the activation's 3 s: a large fleet could never hand over (plausible, depends on volume) | Major | **Measured (D2307).** At 100,000 converged triggers the check takes p50 267 to 300 ms, slowest 349 ms (three runs), so the two are about 0.7 s of 3 s, and the whole move at most 369 ms. Not changed |
| M2 — the tenant audit is one unbatched statement under the job's 5 s timeout: a large tenant set never completes (plausible) | Major | **Measured (D2307)**: 1,000 tenants in 43.4 to 43.6 ms. Beyond that, G2309 |
| m1 — with the shipped bounds (step 3 s, statement 3 s) the client's step bound fires first, so an activation that is too slow is `BUSY` for ever, never `CHECK_TOO_SLOW`; the test of it widened the step to 60 s | Minor | **Fixed.** A step bound counts with the statement timeouts of the activation (`handover_auto.rs`, `drive`). Test, red first (the job never said `CHECK_TOO_SLOW` and was interrupted at its last wait): `trigger_auto_handover::three_step_bounds_of_the_activation_in_a_row_are_check_too_slow` |
| m2 — `TOO_MANY_RUNS` is checked before the state, so after the handover a seventeenth restart within two hours exits 2 and skips the audit completion | Minor | **Fixed.** Once handed over the cap is not looked at (`schedule_trigger_handover_attempt_open`). Test, red first (`54000`): `trigger_auto_handover::after_the_handover_a_seventeenth_run_is_answered_handed_over_and_not_too_many_runs` |
| m3 — the gauges of the handover exist only inside the schedule pass; decision 5 says every engine exports them | Minor | **Fixed in part.** `trigger_schedule_auto_handover_expected` is exported by every engine (`apps/execution-engine/src/lib.rs`); test, red first: `trigger_auto_handover::an_engine_without_the_schedule_pass_still_exports_whether_the_job_is_expected` (the built engine with `SAGAT_SCHEDULES_ENABLED=false`). The reason, mode, age and audit-pending gauges need the view read outside the pass: G2310 |
| m4 — a failed read of the view freezes the age gauge, and one frozen engine holds the alert's `min` down | Minor | **Open (G2311)** |
| m5 — a crashed 0.3 engine's membership outlives its registry row's horizon by up to about 30 s and is told `ENGINE_UNREGISTERED` | Minor | **Open (G2312)**: a streak starts over, nothing is handed over wrongly |
| Nit — the wrapper's two 2 s lock waits inside a 3 s statement show contention as `57014` | Nit | Recorded (G2312) |
| Nit — a refusal under the lock is not written to the attempt row | Nit | Recorded (G2312) |
| Nit — the chart fails the Job at once on exit 3 as well as 2 | Nit | Recorded (G2312) |

## Checked by the reviewer and found sound

The rounds and the stable count (spacing, the 30 s gap, a changed live set); the first horizon and the deadline kept per run; the flag and the variable of the run id agreeing; the controller UID label (`kubeVersion >= 1.27`); the backoff and the clamp to the deadline; the departure under the lock; the wrapper's `session_user`, freshness and actor; the lock order (epoch, the triggers' table, the handover row) in the wrapper, the operator's path and `instance_register`; nothing held across a wait; the signals (handlers installed synchronously, no lost wake-up, exit 4 at every check point); the audit completion (serialised, idempotent, the handover row's actor, mode and time, `AUDIT_CONFLICT` by SQLSTATE); the retirement (refused while the audit is pending, `migrate` cannot log the role back in); the migration's backfill and `down`; the tests (races held by locks and `pg_stat_activity`, sleeps only in polling loops, the real-signal test, the smoke step's exit codes under `set -euo pipefail`).
