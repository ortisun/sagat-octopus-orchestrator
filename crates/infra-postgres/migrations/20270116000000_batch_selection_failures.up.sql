-- Feature 018, US05 (G2093, D2197): a selection round that always fails no longer retries for ever.
--
-- A batch job in `SELECTING` is advanced by rounds. A round that fails (a statement over its ten-second timeout, a deadlock
-- victim, an error that will come again) rolls back its own work and was tried again at the next poll, for ever, for every
-- selector: nothing counted the failures and `SELECTION_FAILED`, in the CHECK of `failure_reason` since the first migration of
-- the feature, was raised nowhere. A bound that survives a restart and a move of the shard lease has to be stored, which is
-- why this is a column and not a counter in the memory of the executor.
--
-- `selection_failures` counts the selection rounds of the job that have failed in a row. The executor adds one, in a
-- transaction of its own under the fence of the shard, to a round that failed, and a round that picks sets it back to zero in
-- its own transaction; the eleventh in a row ends the job `FAILED` with `SELECTION_FAILED` (`sagat_domain::batch::selection_retry`).
-- The backoff between two rounds is the job's `next_slot_at`, which the due-scan already honours (`batch_jobs_due_idx` is
-- `(shard, next_slot_at)` and a job whose slot is ahead is not read at all): no second column, no change to the scan.
--
-- Additive: a new `NOT NULL` column with a constant default is a catalog change and rewrites nothing (PostgreSQL 11 and later),
-- an older engine never writes it (it stays 0, and an older engine goes on retrying as it did), and an older reader never
-- selects it (the reads name their columns). The guard trigger `batch_jobs_guard` lists the columns that never change; this one
-- is progress, like the counters, so it needs no change. The fence of the shard is checked by the store in the same transaction
-- as every other write of the executor, which a trigger cannot see. No grant changes: the runtime role holds
-- `SELECT, INSERT, UPDATE` on the table, and so on the new column. The column is a bounded integer, not variable-size, so
-- nothing is classified in `storage-classification.json`.
--
-- Rollback: the `down` refuses while a job that is still selecting has failed rounds on record, and only then (a job that ended
-- `SELECTION_FAILED` keeps its count as evidence and its reason says the rest: it does not block a rollback).
--
-- Operational impact: the statement takes the table's `ACCESS EXCLUSIVE` lock. Adding the column rewrites nothing; the CHECK is
-- verified by one pass over the rows of `batch_jobs`, a table a job's retention keeps small (a few thousand rows is
-- milliseconds), while jobs wait for the lock for that time.
--
-- ROLLOUT ORDER: apply the migration before the engine that counts. An engine that runs against a schema without the column
-- cannot select at all (a round that picks writes the count, and so does the record of a failure): every round fails, none can be
-- recorded, and the jobs wait, as in an outage, until the migration is applied; no job is ended by it. An older engine against the
-- new schema is not affected.

-- No CHECK ties the count to the state of the job (a count only while selecting would be a tidy rule): an older engine that takes
-- the shard over while a job waits picks its page without knowing the column, and a CHECK would refuse its write and stop it, which
-- breaks "an older engine keeps working against the new schema". A running job with a stale count is harmless (nothing reads it
-- once the selection is over); that a pick resets the count is held by the tests, not by the table.
--
-- MIXED VERSIONS: an older engine that picks a page of a job does not reset the count, so after a rolling deploy a job that is
-- still selecting can carry a count that a pick should have reset, and fail up to ten rounds earlier than it would have. Accepted
-- (D2201): the deploy window is short, the job is one that was failing already, and the end is the same `SELECTION_FAILED`.
ALTER TABLE batch_jobs
    ADD COLUMN selection_failures smallint NOT NULL DEFAULT 0
        CONSTRAINT batch_jobs_selection_failures_valid CHECK (selection_failures >= 0);

-- `next_slot_at` means two things from here on, and the catalog says both (D2198): the scan, the cancel and the monitor read it as
-- "the instant this job is next due", which is what both are.
COMMENT ON COLUMN batch_jobs.next_slot_at IS
    'The earliest instant the next step of the job may be taken, by the database clock: the rate slot of a RUNNING job (the next item may be applied from it) and, for a SELECTING job, the end of the wait after a failed selection round (the backoff of 5 s doubling to 5 min). A cancel brings it back to now.';
