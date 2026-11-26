# Security review, 2026-11-21

The `security-reviewer` agent (read only), given the diff of `bugfix/withdraw-undelivered-leases` against `develop` at commit `bc4ad87d` (the merge of develop into `32aab110`) and the documents of the feature. Its final message, verbatim:

---

I found one Blocker, two Major, three Minor and a few Notes. The Blocker and both Major findings come from reading the code; no test was run (I'm read-only), so the proofs below are tests still to be written. Two smaller points are marked as suspicions.

## Blocker

**B1. Handler code, not only the SDK, can send `TASK_NOT_STARTED`. The server then ignores the retry policy, the backoff and the non-retryable flag.**
- **Where:** `crates/application/src/worker.rs:2430-2464` (root) and `crates/application/src/worker/scoped_reports.rs:376-408` (scoped).
- **Why it reaches the server:**
  - `sdks/SPEC.md:440-448` lets a handler raise `ActivityFailure(code, …)` with any code matching `[A-Z0-9_]{1,64}`. `TASK_NOT_STARTED` matches, and no SDK reserves or rewrites it: the string appears nowhere under `sdks/` except `SPEC.md`.
  - The diff changes no SDK code. So today the only way this code reaches the server is a handler (or a hostile client).
- **Why LEASED proves nothing:** the server's only check is that the step is still `LEASED`. But the first heartbeat comes at least 500 ms after dispatch (research.md D006-001). A handler that finishes faster never leaves `LEASED`.
- **Scenario:** an Activity with `maxAttempts=1` charges a card. The handler then raises `permanent("TASK_NOT_STARTED", "declined")` within 500 ms. What happens:
  - The fence matches, so the server writes `step.lease_withdrawn` and gives the attempt back.
  - `next_attempt_at = None` (`crates/domain/src/execution/state.rs:1478`), so the step is `READY` at once.
  - D006-014 decides on the code alone, so `retryable=false` is ignored.
  - The task is leased again and the handler charges again, with no limit, no backoff and no `step.failed`.
  - For a compensation under `ManualReview`, the review after the retries run out is never reached.
- **What it adds over today:** a lying host can already send a retryable failure after an effect. What is new is the bypass of the budget, the backoff and the non-retryable flag. Reusing the attempt number keeps the idempotency key the same, which helps idempotent providers and does nothing for the others. D006-006's argument ("neither repeats without new polling") does not hold: the loop runs at the pace the host chooses.
- **Proof:** a contract test with a worker that, on every lease and with no heartbeat, sends `FailStep(TASK_NOT_STARTED, retryable=false)` to a one-attempt Activity. Assert that after N leases the journal holds N `step.lease_withdrawn` and no `step.failed`.
- **Smallest fix (server side, because SDKs already in the field forward any code):**
  1. Honour `TASK_NOT_STARTED` as a withdrawal only from a worker session whose accepted protocol minor declares the code reserved for the SDK. Every other session gets the ordinary failure path, with its own `retryable` flag.
  2. Every SDK rewrites a handler-raised `TASK_NOT_STARTED` to `WORKER_ERROR`.
  3. Add the cap in M1.
- **Defence in depth:** refuse the withdrawal when the server already has evidence that this lease was used: secrets resolved, a blob written, or a `script_executions` row.

## Major

**M1. Withdrawals have no cap and no backoff, for either cause. A worker of the tenant can loop a step forever and grow its journal without bound.**
- **Where:** `crates/application/src/worker/withdrawal.rs:137-192` and `crates/domain/src/execution/state.rs:1474-1479`.
- **`NOT_SENT` is in the client's hands:** a client opens `StreamTasks`, lets the claim commit, never reads and drops the stream. The server withdraws, the step is `READY` at once, and the client polls again.
- **`NOT_STARTED`:** the same loop through B1.
- **Cost per cycle:** a claim transaction, a withdrawal transaction, two journal events, an outbox hint, and an idempotency row on the `FailStep` path. Only the tenant's poll rate limit bounds it.
- **Compared with before:** the loop was bounded by the retry budget, or stopped at a review that an operator could see. Now it is silent apart from a counter, and the database is shared by every tenant.
- **Suspicion, not checked:** I found no per-run journal ceiling, and I did not check whether `run_events` counts against `TenantContentBytes`.
- **Proof:** loop the drop-after-commit test 1,000 times with the probe. Expect 1,000 `step.lease_withdrawn` events and attempt 1 throughout.
- **Smallest fix:** count withdrawals per step and attempt, either in a `steps` column or with a bounded count of `step.lease_withdrawn` for this attempt inside the run transaction.
  - After K (say 3), `NOT_STARTED` takes the ordinary failure path and `NOT_SENT` is left to lapse.
  - Log a WARN with the run and node.
  - Revisit D006-006.

**M2. A cancelled unary `PollTasks` releases the tenant's concurrency slot while its claim keeps running.**
- **Where:** `crates/api-grpc/src/service.rs:579-616`.
- **Cause:** `auth` holds `slot: TenantPermit` (`crates/api-http/src/extract.rs:38-43`; the comment at `service.rs:469` says it "holds the tenant's concurrency slot until dropped"). It stays in the handler. The spawned task gets only `auth.session.principal.clone()`.
- **Scenario:** a client sends `PollTasks` and cancels at once (RST_STREAM). The handler is dropped and the slot is freed. The detached claim still holds a pool connection and run locks, until the statement and lock timeouts.
  - Repeating this at the per-subject request rate gives as many concurrent claims as that rate times the claim time, above the tenant's cap.
  - Pool exhaustion then hits other tenants.
  - Before the change, the claim was cancelled with the handler.
- **Proof:** hold claims with the probe, cancel more unary polls than the cap allows, and assert that a further authenticated call still gets a slot. With the fix it gets `RESOURCE_EXHAUSTED`.
- **Fix:** move the slot into the task: `let slot = auth.slot; tokio::spawn(async move { let _slot = slot; … })`.

## Minor

**m1. `TASK_NOT_STARTED` is accepted on an expired lease that the reaper has not processed yet.**
- **Where:** the root branch relies on `verify` (`worker.rs:2888-2904`), which uses `lease_matches_instance` (`crates/infra-postgres/src/execution.rs:4999-5019`). That query has no `lease_expires_at` condition.
- **Why it matters:** ADR-0098 decision 6 says a lapse keeps its unknown outcome. A late hand-back now turns a lapse that should be reviewed into a free re-run.
- **Context:** the ordinary `FailStep` also accepts an expired lease today, so the extra harm is the budget bypass.
- **Fix:** in both `NOT_STARTED` branches also require `lease_unexpired_instance(task.instance, &hash)` (it exists, `execution.rs:4846-4867`), or its path equivalent. Add the missing test.

**m2. Reusing the attempt number collides with records keyed by attempt without generation.**
- **Where:** `script_executions` uses first write wins on `(tenant, run, node, attempt)` (`crates/infra-postgres/src/scripts.rs:1784-1797`). Payload disclosure lists blobs by `attempt` only (`crates/infra-postgres/src/payload_disclosure.rs:667`). The secret audit reasons say "node X attempt N" (`crates/application/src/worker/secrets.rs:117-121`, and the pool refusal at `worker.rs:2748`).
- **Scenario:** after a B1-style withdrawal, the real execution of attempt N is silently not recorded (`Ok(false)`), and the audit cannot tell the two leases apart.
- **Fix:** add the generation to the key and to the audit reason, or refuse the withdrawal when such a row exists.

**m3. The withdrawal tracker has no concurrency bound.**
- **Where:** `crates/api-grpc/src/undelivered.rs:60-72`.
- **Problem:** every dropped frame or poll spawns a task that takes a pool connection with `Lock::Wait`. A flood of dropped streams competes with request traffic for the shared pool.
- **Fix:** a semaphore (for example a quarter of the pool). When it is saturated, count `outcome="error"` and let the leases lapse.

## Note

- **Stale gate (suspicion):** `withdraws_leases()` (`withdrawal.rs:121-126`) ignores `gate.is_open()` and whether the kept epoch is the current one. `refresh_kept` keeps the old set when a read fails (`crates/application/src/instances.rs:626-638`). So after a removal, an engine whose reads fail keeps withdrawing longer than the "within one heartbeat" the runbook promises. Fix: also require the gate open and `kept_epoch() == epoch()`.
- **Untracked claims at shutdown (suspicion):** detached unary claims use `tokio::spawn`, not the tracker, so `drain` (`undelivered.rs:186-193`) does not wait for them. A claim that commits after the drain falls under G006-002.
- **`with_delivery_probe`** (`service.rs:104-110`): it is public, but no composition root under `apps/` calls it (only `tests/support/api.rs:646`). It needs a recompile, receives no lease data and cannot be reached remotely. Put it behind `cfg(feature = "test-seams")` or add a check in `scripts/check-architecture.py`.
- **No finding: secrets.** `UndeliveredLease` redacts its token in Debug (`withdrawal.rs:46-55`), and so does `Token` (`crates/application/src/entropy.rs:42-46`). `Frame`, `PendingLeases` and `WithdrawalSink` have no Debug. The labels of `worker_leases_undelivered_total` are closed sets. The WARN prints only the error and the run id.
- **No finding: tenant boundary.** The group key and `begin` carry the claim principal's tenant. The fence is the step id, the token hash, the attempt and generation, and `LEASED`. A mismatch is `FenceMissed`, and the caller sees nothing of it. The `FailStep` path runs `verify` and `ensure_bound` first.
- **No finding: least privilege.** The migration changes only CHECK constraints and grants nothing. The engine uses existing transaction paths. `platform_requirement_activate` is revoked from PUBLIC (`20270113000000_instance_registry.up.sql:391`).
- **Suspicion: down migration.** It scans `run_events` by `event_type` while holding the registry tables in ACCESS EXCLUSIVE mode, which may block every heartbeat for the length of the scan. Run `\d run_events` to check for an index on `event_type`.
- **No finding: operator command.** The name is validated by `unprintable` and `Subject::parse` and printed through `shown`. The transaction is bounded by `OperatorBounds`, and the fleet is counted again under the epoch lock.
- **No finding: `NOT_SENT` against feature 037.** It rests on the server's own proof, and it never withdraws a frame that was handed over.

Files relevant to this review:
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/scoped_reports.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/service.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/undelivered.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-http/src/extract.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/state.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/scripts.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/payload_disclosure.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/secrets.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/instances.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/tests/contract/durable-execution/grpc/lease_withdrawal.rs
