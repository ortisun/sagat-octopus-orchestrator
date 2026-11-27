# Correctness review of design 053, full round of 2026-11-26

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 7771688e6ab65562395c6c2faa8c548e66a50bdd
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Blocker

1. Phase 2 can pass while a use is in flight: `PgInstanceStore::remove` runs `verify` before the delete, and a fence
   that reads the row without a lock lets a write whose snapshot predates phase 2 through. Scenario: a start-run
   handler checks `requires(JournalBucketsV1)` before phase 1, waits for a connection, and inserts a dated root after
   phase 2 removed the row; N-1 cannot read it. Fix: the gated write takes the requirement row `FOR KEY SHARE` and
   refuses when absent or draining (decision D006-032's pattern, `lease_withdrawal_required()`); phase 2 locks the epoch,
   then the row `FOR UPDATE`, then runs the applied-epoch and drain checks; a barrier test of a use held open.

## Major

1. `requires()` ignores the capability window and the full-table state: an engine whose beats time out keeps serving
   past 30 s with `requires()` true for any length of time; a process the full table did not take answers true; design
   049's claim that a process unaware of a drain creates legacy roots is wrong (it creates dated ones). Fix: one gate
   method true only inside the capability window and when counted; correct 049 for the drain direction.
2. A failed concurrent build is recorded as applied before the validity check (`IF NOT EXISTS` skips the invalid
   index, sqlx records the version), and a crash between record and check is never caught. Fix: the loop runs the
   statement, checks `indisvalid`, and records only after; check the applied migrations' indexes at start.
3. A 2 s `lock_timeout` makes concurrent builds fail on any 3 s transaction and protects nothing (they never block
   inserts). Fix: no lock timeout there; an overall deadline naming the oldest transaction.
4. Only the last statement is bounded: an earlier `ALTER TABLE` on a table the hot path writes but not listed queues
   behind a long transaction with every insert behind it, or deadlocks with a run transaction. Fix: `lock_timeout` as
   the first statement; any lock stronger than `SHARE UPDATE EXCLUSIVE` on a table the hot path writes is blocking.
5. The partitioned-index procedure can loop for ever (an invalid child skipped by `IF NOT EXISTS`, a partition
   detached by a retirement). Fix: per-partition validity, at most 3 rebuilds, bounded rounds, a vanished partition
   done, an exit naming the partition.
6. A migration with an old timestamp escapes the boundaries and is applied out of order. Fix: refuse a new migration
   whose version is not above the merge base's newest; apply the boundaries by "file absent at the merge base".
7. The capability mechanism is refused by its own rules (three closed checks on registry tables, one blocking lock per
   migration); widening in separate migrations lets registration pass and activation fail. Fix: a
   `registry_capabilities` table with keys in 0.3.0.
8. Phase 2's refusal has no name (every outcome but `REMOVED` is `StoreError::Internal`) and its wait is two beats
   (about 24 s) plus up to 30 s for a crashed row. Fix: `NOT_DRAINING`, `NOT_APPLIED{count, ids}`,
   `DRAIN_CHECK_REFUSED`, exit codes, a command that polls up to 54 s.
9. One-run commands are invisible to both phases. Fix: the row fence in each writing transaction, or register.
10. A draining row reads as active in `lease_withdrawal_required()` (so phase 1 does not stop withdrawals) and in
    `schedule_trigger_handover_complete`. Fix: `state = 'active'`; `requirements()` returns the state; a test per
    reader.
11. Two tests can pass while the property fails: the overlap test cannot tell the sent epoch from the read one; the
    mixed-writer test may remove nothing, roll back a process whose gate stays closed, and have no migration in its
    latency window. Fix: a paused process between beat and refresh; assert a capability removed, the rolled-back
    process serving, and a fixture migration on the hot tables.

## Nit

1. The two guards `RETURN OLD`, which on update silently keeps the old row; the replay floor on draining refuses phase 1
   of lease withdrawal. Fix: return `NEW` on update; the handover check on `active` to `draining` only; the floor on
   delete only.
2. Store `draining_epoch`, set by the drain and cleared by the undrain; count rows of the requirement's kinds.
3. The history check needs `DRAINED` and `UNDRAINED`.
4. Over 40 suites call `MIGRATOR.run` directly; make `migrate()` the loop and refuse direct calls.
5. "2 s plus one commit" is per attempt; Success criterion 053-003's 12 s omits the beat's call and the second
   registration; `57014` is also an operator's cancel; the second `migrate` waits for the advisory lock without bound.

Counts: Blocker 1, Major 11, Nit 5
