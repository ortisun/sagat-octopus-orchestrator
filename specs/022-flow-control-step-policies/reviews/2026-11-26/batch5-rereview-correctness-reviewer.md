# Correctness re-read of the batch-5 fixes (2026-11-26)

Short re-read of the fixes alone (OD-2026-11-25-2 step 4), commits `ac107fcb..223ab80d` of `feature/022-throughput-control`,
against `reviews/2026-11-25/batch5-reviews-and-rulings.md` and the batch-3 and batch-4 minors that `bb8a35d2` closes. Read by
the correctness reviewer; nothing was run. Decisions D022-090 to D022-092 of that batch are now D022-121 to D022-123.

Verdict: the fixes for Major 2 and most minors are correct; Major 1 is fixed only when a key's limit is 2 or more; five batch-5
rulings were not built in the range.

## Batch 5 rulings

| Ruling | Verdict | Evidence |
|---|---|---|
| Major 1: only a busy run or a transient failure ends a key's pass; other failures skipped, counted, logged and alerted; a held signal whose delivery fails is dropped | partly | `passes.rs:594` (`of_failure`), `:6065`, `:6078`; second try that drops the signal `:6116`. Test `run_admission::a_waiter_whose_grant_always_fails_does_not_hold_back_the_younger_waiters` fails without the change. Gaps: the failing waiter still counts ahead of younger ones (new finding 1); a commit-time delivery failure is not retried (2); no test for the transient branch (3); no alert: `CONCURRENCY_QUEUE_STUCK` does not cover rate-limit keys and has no test for this case. |
| Major 2: every return that does not commit rolls back and awaits it | met | `passes.rs:6142-6170` (`try_grant_waiter`), `PgRunTx::rollback`. Test `run_admission::a_refused_waiter_frees_its_run_before_the_next_waiter_of_the_same_run_is_tried` (relies on a sleep, finding 5). |
| Correctness minor 1: refuse at creation as the delivery would (no entry-path target, output bound) | not met | Not in the range; the output-bound check exists only at the grant (`application/src/execution.rs:2229`). |
| Correctness minor 2: lock the active-run counter only when the entry can end the run | not met | Not in the range. |
| Correctness minor 3: `verify_run` and `assert_consistent` check held rows; drop on any end of a run whose start never ran | partly | Check `infra-postgres/src/execution.rs:790`, test `run_creators::verify_run_refuses_a_held_signal_kept_for_a_run_whose_start_does_not_wait`; the wider drop (`:5597`) has no test that fails without it (finding 4). |
| Correctness minor 4: ADR-0089 decision 4 amendment | met | ADR-0089, dated paragraph under decision 4. |
| Correctness minor 5: `spansPages` | met | `flow_control.rs:1366`, DTOs, both OpenAPI files, `schema.d.ts`; test `rest::a_group_larger_than_a_page_is_summed_across_pages_and_the_next_group_is_reached`. |
| Security minor 1: FR-022-060 for signals; test revoking `run:signal` | met (weak test, finding 6) | `spec.md:764`; `run_creators::a_held_signal_is_delivered_without_reauthorization_and_keeps_its_sender`. |
| Security minor 2: the drop's audit names the sender | met | `held_signals.rs:109`. |
| Security minor 3: two doc comments | met | `routes/execution.rs:794`, `runs.rs:1034`. |
| Security suspicion 3: cursor expires after fifteen minutes | met | `flow_control_keys.rs:472`; tests `flow_control_keys::tests::a_cursor_expires_fifteen_minutes_after_its_page`, `rest::an_expired_cursor_is_refused_with_its_own_message`. |
| Security suspicion 4 | not met | Depends on correctness minor 1. |
| Security nit: held, delivered and dropped counter | not met | No such counter. |
| Security nit: alert detection run as `sagat_app` | not met | Not in the range. |
| Correctness nit: fault tests (commit failure after delivery; timeout racing the grant) | not met | Not in the range. |
| Correctness nit: `effective_sql` returns a `Result` | met | `quotas.rs`; test `quotas::effective_sql_tests::iteration_concurrency_has_no_effective_expression_in_any_build`. |

## Batch 3 and batch 4 minors closed by `bb8a35d2`

All met: batch 3 correctness minor 6 (rate-limit waiters locked first), batch 3 correctness nit (triggers guarded by `node = ''`),
batch 4 correctness minor 1 (`migration_head()`), batch 4 correctness minor 4 (rollback measured from its own schema), batch 4
correctness minor 6 (label test on non-empty lists), batch 4 correctness nit 1 (`OTHER_VIEW`, concurrency listing only, finding 7).

## New findings

1. **Major: a waiter whose grant always fails still holds back every younger waiter when the key's limit is 1.**
   `crates/infra-postgres/src/execution/gate_queue.rs:234-245, 311`; `lib.rs:45` (`eligible_gate_waiter`). On a serial key the
   older waiter's holder insert always fails, so it is skipped and counted `failed`; the younger waiter's reservation check reads
   `taken = 0`, `ahead = 1` (the eligible-waiter rule counts the older waiter), `0 + 1 < 1` is false, so the younger waiter is
   refused on every pass until the older one's deadline, and every new arrival queues behind it (D022-073). The test uses
   `serial_start("start-poisoned", 2)` on purpose. Fix: a waiter skipped for a lasting failure stops counting ahead (a marker
   `eligible_gate_waiter` excludes until its next successful try, or end its wait through the timeout routine with a failure
   reason). Test: the same test with limit 1. Confirmed in code.
2. **minor: a delivery that fails at commit is never retried without the signal.** `passes.rs:6165`: the commit's error is
   `GrantFailure::Other`, so the retry that drops the signal never runs (for example the deferred content-quota flush raising at
   commit, `20261024000000_durable_quotas.up.sql:353-354`). Fix: record that the try delivered a held signal and class a commit
   failure of such a try as a delivery failure. Plausible.
3. **minor: no test pins that a transient failure ends the pass.** `passes.rs:6065`. Test: a trigger raising `40001` on the
   older waiter's holder insert; assert `gates_granted == 0`, `failed == 1`. Confirmed in code.
4. **minor: the wider drop of correctness minor 3 has no test that fails without it.** `infra-postgres/src/execution.rs:5597`.
   Either a test that ends a queued start by a path writing no dequeue or timeout event, or a statement in D022-123 that no such
   path exists.
5. **nit: the Major 2 test relies on a 500 ms sleep** (`run_admission.rs`, `after_release`). Check instead that a second
   connection's `SELECT ... FOR UPDATE NOWAIT` on the run succeeds after a refused try.
6. **nit: the re-authorization test cannot tell whether the demotion took effect** (`run_creators.rs`): assert one membership
   row changed and that a direct signal from carol is refused.
7. **nit: the rate-limit listing's `OTHER_VIEW` message is untested.**

The migration commits (D022-089, waiting-kind validation, held-signal quota triggers) and the boxing commit (D022-102) introduce
no defect found; their checksum changes require recreating a database that applied the branch, as D022-089 states.
