# Correctness re-read of design 049 (2026-11-24)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 3d09d92388c49d565826a1bb21e7901840638f6d
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The second read, of the
revision that answered `correctness.md` and `architecture.md`; the reviewer read files only and ran nothing.

## Blockers

1. The new primary key puts `journal_bucket` second, so every lookup by `(tenant_id, run_id)` of the binaries that do
   not pass the bucket (all of today's) reads all of the tenant's entries. Fix: `(tenant_id, run_id, seq,
   journal_bucket)` and `(tenant_id, run_id, digest, journal_bucket)`.
2. Starting a run becomes impossible once dated runs exist: the input payload is written before the run row, so the
   bucket trigger on `run_payloads` and the reference trigger on `runs.input_digest` contradict each other. Fix: one of
   them deferred to commit, counted in the budget.

## Majors

1. Retirement counts the bytes before the detach; a per-run purge of the same bucket committed in between releases bytes
   twice. Fix: count from the detached table; exclude purges of the bucket during a retirement.
2. No accounting function can release bytes without a row change (`sagat_quota_add_delta` only queues; the flush is a
   trigger function). Fix: an owner-only definer function with an underflow check.
3. The quota switch at the contract step blocks payload writes for a scan or double-charges until the owner-gated
   cleanup.
4. The cleanup of legacy payloads is not defined and would fail on the legacy partition's foreign key to `payloads`;
   shared digests and tombstones must be excluded.
5. Step 4 leaves out the parent's three checks and `run_events_instance_idx`, keeps the legacy table's grants, and has
   trigger and index name collisions.
6. A 2 s `lock_timeout` cancels the concurrent index build and the concurrent detach, which wait for old snapshots
   through lock waits.
7. Run-purpose blobs are not handled.
8. Rows that refer to content across runs (`operation_update_receipts` of a chain, `chain_segments.transfer_input_digest`)
   have no named owning run.

## Nits

1. No supported PostgreSQL release skips locking pruned partitions in a generic plan, and PL/pgSQL triggers cache
   generic plans.
2. The static check rejects correlated joins; callers without the run row need a statement.
3. `RUN_PURGED` and `JOURNAL_BUCKET_MISSING` both arrive as SQLSTATE 23514 with no constraint name.
4. Reference triggers must be enabled always and fire only on the columns they check.
5. The registry misses `leaf_collection_success_certifications`' two digests; `leaf_source_no_delete` must consult the
   purge authorization.
6. The index on `runs (journal_bucket)` must be built concurrently; `transaction_timeout` must be lifted for owner
   sessions.
7. A resume after a reattach restarts at step 1 and discards the recorded bytes.
8. Legacy runs can keep appearing after the switch from a stale process.
9. The payload move starts on a one-time census.

Counts: Blocker 2, Major 8, Nit 9
