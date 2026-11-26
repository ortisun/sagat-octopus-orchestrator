# Reviews of the batch `c6cf8200..a99bb05b` (tasks T022-071, T022-082, T022-105 and the earlier rulings), and the rulings (2026-11-25)

The repository's read-only `correctness-reviewer` and `security-reviewer` read the code at `a99bb05b`. No Blocker.
Correctness: 2 Majors, 8 minors, 4 test-honesty findings, 3 nits. Security: 1 Major, 2 minors, 3 nits. Every ruling is
delegate (OD-2026-11-24-4), built with a test seen failing first (or a hand mutation where the code is already right).

## Majors

| Finding | Ruling |
|---|---|
| Correctness Major 1: a run-start grant deletes its waiter row (which locks the `flow_control_counts` slot row, 8b) before `persist` locks the run's `run_active_counts` slot row (6) when the grant's settle ends the run; an admission takes 6 then 8b; two runs of one namespace sharing a slot can deadlock (`gate_queue.rs:364-399`, `passes.rs:5868-5879`, `execution.rs:5514-5520`); the migration's own claim (`20270212000000_flow_control_counts.up.sql:12-17`) is false for this path | A run-start grant locks the run's active-run counter row before it takes its keys, restoring run, slot 6, keys (7), slot 8b; the migration's comment is corrected; `concurrency_locks` gains slot 8b and a case against the reference order (run, slot 6, key, slot 8b) for a queued start whose entry is `end`; the stale header of `concurrency_locks.rs` is updated |
| Correctness Major 2, Security Major 1: the saturated and stuck alert candidates are cut at 5,000 rows deployment-wide before each namespace's condition is applied (`operations.rs:141-171`, `:269-275`, `:303-305`, `:316`); a cut sets `Short::Everything`, so nothing resolves for anyone; one tenant's volume hides another's saturated key | The condition is applied in SQL, joining each namespace's effective ceilings, before any limit; candidates are ranked per tenant and namespace (`row_number() OVER (PARTITION BY tenant_id, namespace_id …)` ordered by severity, age first for stuck), as the generic alert path does; "short" is recorded per tenant and namespace (`Short::Tenant`/`Short::Namespace`), never globally; the stuck query has an `ORDER BY`. Tests: more than 5,000 one-waiter windows of tenant A beside one saturated key of tenant B (B fires; A's drained alerts resolve) |

## minors and test honesty

| Finding | Ruling |
|---|---|
| Correctness minor 1: a child left for another replica (`BINARY_CAPABILITY_MISSING`) stays at the head of its shard's window and is retried by the owner every tick, logged as an error | The pass moves the reservation to the back (touches its `updated_at`), counts it `deferred`, never `failed`, and logs at `warn`, as the grant pass does; a test with one process that holds every shard and cannot serve the gate: the other children of the shard still start |
| Correctness minor 2: `CHECK (queued_runs >= 0)` can wedge a run after drift; a missing row hides drift; no recount | The trigger saturates at zero (as `run_active_counts` does) and logs a drift; the monitor compares counts with rows, bounded per namespace, and raises the existing consistency alert kind or a new `FLOW_CONTROL_COUNT_DRIFT` (decide by what the alert catalog allows); the runbook gains the recount statement |
| Correctness minor 3: two alerts resolve on the first look that misses the condition and flap on a busy key | An alert of these kinds resolves only after two consecutive looks that miss its condition (deterministic in tests: two monitor rounds) |
| Correctness minor 4: the orphan check joins every waiter to `runs` on each look; ceilings are read once per namespace in sequence | Both bounded: the orphan check limited and ranked per tenant like the others; the ceilings read in the same statement (the join of the Major ruling) |
| Correctness minor 5, security minor 2: the reset's advisory read opens a second pool connection while the request holds the source's row | The advisory read runs on the request's own transaction; it also counts the eligible waiters the completion's reservation rule counts |
| Correctness minor 6: the counts migration locks `concurrency_waiters` then `rate_limit_waiters`; a dual-gated admission inserts rate first | The migration locks rate first, as admission does |
| Correctness minor 7: FR-022-058 still requires slot-8b live-key counters; D022-084 departs | A dated amendment line under FR-022-058 naming D022-084 |
| Correctness minor 8: the counts and alert-kind migrations sort before the renumbered pinned-limit migrations | They are renumbered after the pinned-limit migrations, so the rollback order follows the dependencies; the `BEFORE_COUNTS` test's target is checked |
| Test honesty 1: the differential test fixes the deadline at the maximum, runs alive, no ceiling, no unleased grant, never a cause that may not queue, never compares place and estimate with `RATE_PLACE_SQL` | All of these enter the generated sequences, and place and estimate are compared with the SQL |
| Test honesty 2: the estimate property uses identical waiters only | A property over mixed bounds for `place` against `admits_queued` |
| Test honesty 3: the reset-retry test polls `pg_stat_activity` for any lock wait and discards a timeout | `pg_blocking_pids` on the two sessions; the timeout asserted |
| Security minor 1: two derived groups sharing their first 120 characters collapse into one alert subject | A group longer than the subject bound is written as its first characters and a fixed-length digest suffix, so subjects are unique and bounded; grouping uses that subject |
| Security nit 1: no test that the runtime role cannot attach the trigger function to a table of its own | The probe, expecting `42501` |
| Security nit 2: the runbook's orphan repair omits `tenant_id`, and does not say an orphan holds a unit of `queued_runs` | Both corrected |
| Security nit 3: the privilege audit's fallback keeps a macro's prefix and drops what follows a `$` | The fallback records the macro as unresolved instead |
| Correctness nit: the trigger runs for every step waiter row | Two triggers with `WHEN (NEW.node = '')` / `WHEN (OLD.node = '')` |
