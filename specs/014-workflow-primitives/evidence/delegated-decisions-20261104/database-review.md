# PostgreSQL and transaction review of ADR-0075 — 2026-11-04

Recorded by the assistant from the reply of the `correctness-reviewer` agent, briefed as a
PostgreSQL and transaction-correctness expert (read-only; its session could not write files, so this
is the reply, not a file it produced). Paths are relative to the repository at `54570349`. The
reviewer read the code; nothing was run. It was given the question and the files, not the
assistant's leaning, and it did not see the architecture review.

## Verdict: ACCEPT WITH AMENDMENTS

The lock order and deadlock freedom hold in code. The group lock as written lets non-holders starve
native Acquire and Vacate, and the creator and mixed-version rules are open.

| # | Result | Evidence |
|---|---|---|
| 1 Lock order is the documented one in every path | HOLDS | Every path that touches a group takes chain, then run, then group, then fence. Worker: `execution.rs:388-410` (chain), `:244-253` (run), `worker_capacity.rs:111-113` (group). Natives: `capacity_transfer.rs:210,217,220,225`, `capacity_finish.rs:178,188,191,196`, `scoped_leaf_finish.rs:302,311,325,345`. `capacity_discovery.rs` takes no locks. No group-first path was found. Every other `runs FOR UPDATE` site takes the chain first (`lock_member_chain`); the exceptions are `secret_cleanup.rs:30` (SKIP LOCKED), the `FOR SHARE` readers and `chain_progress.rs:916` (a run created in the same transaction). Caveat: the worker also takes blocking `worker_sessions` and quota/breaker locks after the group (`execution.rs:588`). The ADR does not document them, and G721 says the graph is not accepted. |
| 2 Deadlock freedom and starvation | Deadlock HOLDS; starvation FAILS | The group is never a blocking wait: natives use NOWAIT and the worker uses SKIP LOCKED. The worker also never waits on chain or run (`execution.rs:404-407`, `:705`). A non-holder poller locks the group `FOR UPDATE` (`worker_capacity.rs:113`) before it learns it is not the holder (`:141`). When the group is VACANT every member is a non-holder, so Acquire NOWAIT races all member pollers and can get Busy repeatedly (starvation is plausible, not forever). G718 says fairness is not claimed. |
| 3 Lease commit and unknown COMMIT | HOLDS (safety); lease-path test missing | The group lock and the lease share one transaction, committed at `worker.rs:1184`. A lease never changes the group row. An ambiguous COMMIT leaves an orphan LEASED step with a token nobody holds. Vacate refuses it as NotQuiescent (`capacity_transfer.rs:331`) until the reaper clears it. Two live holders are not possible. No test injects an unknown COMMIT on the lease path. |
| 4 Re-read and invariants | HOLDS for the re-read; invariant not DB-enforced | Transactions are READ COMMITTED (`pool.rs:123`). The membership is re-read after the group lock (`worker_capacity.rs:124-140`). The uniques are per group (`20261210000000_capacity_finish_storage.up.sql:41-44`). There is no unique index on (tenant, namespace, chain) where CHILD_ROOT and ACTIVE, and the worker index `20261231000000` is non-unique. "Current head" is the chain lock plus a read of `current_run_id`. Today only the chain lock and the fail-closed refusal enforce this. It is harmless until a creator exists, because only owner fixtures write memberships. |
| 5 Indexes | HOLDS | The scoped, ACTIVE and any-history probes use `capacity_members_*_worker_idx` (`20261231000000`). The chain lookup uses `chain_segments_run_unique` and the primary key. G714 is resolved by `steps_run_capacity_lease_idx` (`20261226000000...up.sql:52`). The probe costs about 6 index lookups per ready instance, bounded by batch size. Natural planner choice is unproven. That index is built without CONCURRENTLY on the hot `steps` table, which blocks writes during the build. |
| 6 Rollout with an old worker | Breaks, the ADR admits it | The guard is an application flag (`worker.rs:536`, default false). An old or unflagged replica leases a non-holder's step without looking at the group. After A vacates and B acquires, an old replica can lease A. Two chains then run in the group, which breaks the capacity invariant. The data stays consistent. The old claim takes no group lock, so there is no deadlock. |
| 7 Runtime-role privileges | HOLDS | `sagat_app` has only SELECT on `capacity_groups` and `capacity_group_members`. It has column UPDATE only (`20261210000000_capacity_finish_storage.up.sql:169-180`), plus column-restricted INSERT on the receipts. It has no INSERT on the group or member tables. `FOR UPDATE` works with column UPDATE. A creator needs a new grant migration that ships before the creator code. Enabling the guard before any creator exists changes nothing. |

## Amendments, verbatim

1. "A worker claim MUST NOT take a row lock on G unless an unlocked header read shows HELD by the
   claiming member. Vacant, Retired and non-holder refusals are decided without locking G. The grant
   path re-reads the header and the member under `FOR UPDATE SKIP LOCKED`."
2. "Membership creation MUST lock chains in ascending (tenant, chain_id) order, then runs, and MUST
   issue `SELECT ... FROM capacity_groups FOR UPDATE NOWAIT` before the first INSERT into
   `capacity_group_members`." The reason is that the insert's foreign-key check waits
   unconditionally behind a worker's lock on the group.
3. "Before the creator ships, add `UNIQUE (tenant_id, namespace_id, chain_id) WHERE
   member_kind='CHILD_ROOT' AND state='ACTIVE'`, or record that only the creator and the fail-closed
   guard enforce it." Adding the index means the duplicate-active fixture in
   `tests/integration/workflow-primitives/capacity_worker_leases.rs` (around line 249) can no longer
   seed duplicates.
4. "The creator migration grants column-scoped INSERT on both tables to `sagat_app` and extends the
   runtime-privilege test. Activation is the first membership creation, gated by a database-recorded
   flag set only after every replica runs the guarded binary."
5. Optional: cache the possession verdict per transaction. Build the new `steps` index concurrently.

## The single test the reviewer demands before activation

A real-PostgreSQL barrier test, run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`:

- holder and non-holder pollers run against sweeper Vacate and Acquire;
- it injects an unknown COMMIT by terminating the backend after COMMIT is sent;
- it asserts that the group never has more than one chain with a live LEASED or RUNNING step;
- it asserts that Acquire eventually succeeds, which exercises amendment 1.
