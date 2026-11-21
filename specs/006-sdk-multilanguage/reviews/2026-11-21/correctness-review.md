# Correctness review, 2026-11-21

The `correctness-reviewer` agent (read only), given the diff of `bugfix/withdraw-undelivered-leases` against `develop` at commit `bc4ad87d` (the merge of develop into `32aab110`) and the documents of the feature. Its final message, verbatim:

---

No Blocker. On the server I found no path where a withdrawal re-runs a handler that may have started. A withdrawal happens only before `poll_next` or the unary handler returns, it is fenced on step id, token hash, attempt, generation and `LEASED`, and it commits in one transaction under the run lock, with its outbox hint, as constitution I requires. There are four Major findings: a claim that keeps leasing after its consumer is gone, two contract tests that cannot fail for the decisions they are named for, a graceful shutdown that does not wait for claims in progress, and a host rule that no SDK implements. Nothing was run; each finding comes with the test that would prove it.

## Major

**M1. A claim whose consumer is gone keeps leasing more runs (confirmed in code).**
- **Where:** `crates/api-grpc/src/service.rs:829` (the stream producer awaits `poll_bounded` unconditionally) and `:595` (the unary claim runs in a detached task). The loop that leases one run after another is `WorkerService::claim`, `crates/application/src/worker.rs:1055-1092`, and it never checks a stop signal.
- **Before:** an abort or `timeout_at(producer_end, …)` cut the claim at its next await.
- **Now:** a client that goes away, or a window that ends, during the first run's transaction lets the claim go on through up to 64 candidate runs, leasing up to `max_tasks` steps that nobody will receive.
- **Without the requirement recorded:** every one of those leases lapses and, for a declared `Activity`, becomes `step.needs_review`. G006-003 says nothing changes before activation; this makes it worse than before.
- **With it recorded:** each one costs a `step.leased` plus `step.lease_withdrawn` pair and a run lock, for work nobody asked for.
- **Scenario:** a stream with credit 64, ten ready runs of one-attempt activities, the client cancels after the first run's commit. Ten steps end `LEASED`, then are reviewed, where before at most one was.
- **Fix:** pass the `stopped`/`abandoned` token into `poll_bounded` and `claim`, and check it before each `claim_run`. Between run transactions nothing is half-committed, so stopping there is safe; return what was already committed.
- **Proof:** a contract test with two ready runs, the consumer dropped at a barrier after the first commit, asserting that the second run's step was never leased.

**M2. The contract tests for D006-002 and D006-017 cannot fail for the reason they are named for (confirmed by reading).**
- **Where:** `tests/contract/durable-execution/grpc/lease_withdrawal.rs:234` (`a_claim_that_ends_after_the_window_withdraws_its_lease`) and `:277` (`a_unary_poll_dropped_in_the_middle_of_its_claim_withdraws_the_lease`).
- **Why they cannot fail:** the probe runs after `poll_bounded` returns and after the `PendingLeases` guard exists (`service.rs:899` and `:609`). The real case (1) of the finding is a claim cut off between two run commits, before any guard exists; neither test reaches that point.
- **Mutation 1:** restore `timeout_at(producer_end, poll_bounded(..))`, or `producer.abort()`. Both tests stay green: the abort drops the future at the probe, the guard drops with it, and the lease is withdrawn anyway.
- **Mutation 2:** remove the `tokio::spawn` from unary `poll_tasks`. Still green: the dropped handler drops the inline guard.
- **Fix:** add the probe inside `WorkerService::claim` after each run's commit, use two runs, and drop the consumer after the first. Both mutations above must then turn the tests red; show them failing first (rule 18).

**M3. A graceful shutdown does not wait for a claim in progress (the spawns are confirmed in code; the loss depends on timing).**
- **Where:** `apps/execution-engine/src/lib.rs:809` calls `drain` only on the `Withdrawals` tracker. The stream producer (`service.rs:707`) and the unary claim (`:595`) are plain `tokio::spawn` tasks, not on that tracker.
- **Scenario:** on SIGTERM, `serve_grpc` returns once the connections close. A producer still inside `poll_bounded` is not tracked, so the empty tracker drains at once. The serve task then ends and `pool.close()` runs (`lib.rs:834`). The claim commits a lease, then its withdrawal fails on the closing pool or is spawned onto a runtime that is shutting down. The lease lapses into a review.
- This contradicts D006-016 and the text of G006-002 ("a graceful shutdown waits for it"). It happens on every rolling deploy with open streams.
- A second problem: `drain(SHUTDOWN_GRACE)` starts a fresh 30 s inside the 30 s grace that `supervise` gives, so `supervise` can abort the drain.
- **Fix:** spawn the producer and the unary claim through `withdrawals.tracker.spawn(..)`, and bound `drain` by the grace that remains.
- **Proof:** an engine-level test that holds a claim at a probe, triggers shutdown, releases the probe after `serve_grpc` returned, and asserts `step.lease_withdrawn` (it fails today).

**M4. The host hand-back is a MUST that no SDK implements (confirmed by grep).**
- **Where:** D006-005 ("All six SDKs"), ADR-0098 decision 5 and `sdks/SPEC.md` §6.2 make `FailStep` with `TASK_NOT_STARTED` mandatory for hosts.
- No file under `sdks/` other than `SPEC.md` mentions `TASK_NOT_STARTED`, there is no conformance scenario, and nothing is marked pending.
- This breaks AGENTS rules 8, 10 and 11 and constitution VIII ("a placeholder is marked explicitly"). Go and Python still drop leases when stopping, unreported, which is the reported finding.
- **Fix:** mark §6.2 and D006-005 as pending, with a tracked task per SDK and a shared `SAGAT_SDK_SCENARIO`, or deliver the SDKs.

## Minor

1. **An ambiguous commit loses leases the server still holds** (`crates/application/src/worker.rs:1430`; plausible). When `tx.commit().await?` returns an error after the COMMIT reached the server, `claim_run` returns `Err` and its `ClaimedTask`s are dropped without a guard. The withdrawal is fenced on the token hash, so handing them to it is harmless if the commit did not happen. Fix: on a commit error, build `UndeliveredLease`s and send them to the withdrawal. Proof: a store double whose commit succeeds and then reports an I/O error.

2. **A handed-back lease still answers heartbeats with OK** (`crates/application/src/worker.rs:2397`, `effects.reported` kept in the withdrawal branch, read at `crates/infra-postgres/src/execution.rs:5028-5031`). After a `TASK_NOT_STARTED` withdrawal the step is `READY` with `reported_lease_token_hash` set to the handed-back token. Until the next lease, a heartbeat with that token gets OK with cancel=false instead of `STALE_ATTEMPT`. A host that broke "never after start" keeps its handler running while the step is re-leased, so the handler runs twice and nothing shows it. Fix: clear `effects.reported` in the withdrawal branch (root and scoped). Proof: hand back, heartbeat with the same token, expect `STALE_ATTEMPT`.

3. **Withdrawn tasks are counted as streamed** (`crates/api-grpc/src/service.rs:909`). `worker_task_stream_tasks_total` and `worker_task_stream_claims_total{outcome="ready"}` are incremented before the `stopped.is_cancelled()` check at `:911`. They count tasks that went to the withdrawal, so these counters disagree with the journal. Fix: increment only after `place.send` (or when the frame is handed).

4. **The upgrade-path evidence in D006-004 does not exist.** D006-004 (`specs/006-sdk-multilanguage/research.md`, around line 237) says "the released-v0.2 upgrade path proves that no `step.lease_withdrawn` is written before the activation and that the current build replays a journal that holds one". The diff only bumps the expected migration number in `release_upgrade*.rs`. Either write those assertions or remove the claim.

5. **`stream-completion.md` contradicts itself.** Lines 91-93 still say that at token expiry "already committed leases still recover by expiry; they are never requeued immediately", while the new rule 1 withdraws "a frame drained at token expiry". Update the older text.

6. **The runbook's rollback section understates the cost** (`docs/runbooks/lease-withdrawal.md`, "Rollback").
   - Withdrawals already in flight, and engines that have not yet beaten (up to about 10 s), still write the event after the requirement is removed. `withdraws_leases()` is read in memory, not under the run lock.
   - Once any `step.lease_withdrawn` exists, an older binary cannot replay that run, and the down migration refuses permanently because the history names the capability.
   - Say plainly that a binary rollback after activation is safe only if `SELECT count(*) FROM run_events WHERE event_type='step.lease_withdrawn'` is 0 after waiting one heartbeat interval plus the grace.

7. **Scoped withdrawals are barely tested.**
   - There is no contract or integration test of a scoped withdrawal (`withdraw_path`, `begin_current_path`) or of the scoped `TASK_NOT_STARTED` branch (`crates/application/src/worker/scoped_reports.rs:376`).
   - D006-011 is tested only at the root. The scoped domain case 1 (`crates/domain/tests/lease_withdrawal.rs:405`, `:415`) applies a bare `StepCancelRequested` and then asserts `READY` and a successful re-lease. That blesses a lost cancellation instead of using a real drain (collection fail-fast, loop timeout).
   - There is no end-to-end test of token expiry, and the window-deadline discard of a queued frame is tested only at unit level.

8. **`NOT_STARTED` reports that are not withdrawn are not counted** (`crates/application/src/worker.rs:2430`). With the requirement absent or the step `RUNNING`, nothing increments `worker_leases_undelivered_total{cause="NOT_STARTED", outcome=lapsed|fence_missed}`, so D006-018's counter undercounts host hand-backs.

## Note

- **A withdrawn step goes to the back of the queue.** The step-row update sets `ready_at = now` on `LEASED`→`READY` (`crates/infra-postgres/src/execution.rs:7043-7045`), so the step loses its place in `steps_ready_idx`. A step withdrawn repeatedly (D006-006 sets no cap) can be starved. The spec only says telemetry keeps the first ready time.
- **Stream admission bounds can be exceeded briefly.** The stream admission ticket is released when the body drops, while the producer may still be claiming. New streams can then open, and the process, tenant and subject bounds can be exceeded for a moment.
- **Some fence misses are miscounted.** `is_fence_miss` includes `IllegalTransition`, which can also come from the settle step after the withdrawal event was applied in memory. The transaction stays consistent (persist detects the divergence and rolls back), but the metric can say `fence_missed` for a planner fault.
- **A guard dropped during runtime shutdown loses its leases silently.** `WithdrawalSink` spawns while the runtime is shutting down, so the task is dropped. It is counted as handed over, with no outcome.

**Checked and found sound:**
- **Root fence:** checks the step id, `lease_matches_instance` (token hash), then the reducer's `fenced_step(.., &[Leased], ..)`; a stale fence and a `RUNNING` step leave the state unchanged.
- **Attempt and generation:** the attempt is given back and the generation kept; the retry budget is not spent.
- **Codec:** an exact inverse, and an unknown cause is refused.
- **Steps row:** the lease columns are cleared, so `steps_lease_consistent` holds.
- **Session in-flight count:** derived from the steps rows, so it follows the withdrawal.
- **Pins:** the lake blob and database credential pins are keyed by generation.
- **Idempotency:** the report scope contains the token hash.
- **Lock order:** the withdrawal takes only the run lock, and claims take the run lock before the session row, so there is no cycle.
- **Late hints:** the late-hint frame carries no leases.
- **tonic:** stops polling the body after an `Err` (checked in `tonic-0.14.9/src/codec/encode.rs`), so the token-expiry path cannot expose a frame queued late.

Diff reviewed: `/private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/withdrawal.diff`
