-- A loop that still needs another body the run's lifetime instance budget cannot admit stops with
-- the durable failure INSTANCE_LIMIT, as one that spent its iteration budget stops with
-- ITERATION_LIMIT. The checkpoint records which budget ran out.

ALTER TABLE loop_checkpoints
    DROP CONSTRAINT loop_checkpoints_continuation,
    ADD CONSTRAINT loop_checkpoints_continuation CHECK (
        continuation IN ('CONTINUE', 'DELAY', 'COMPLETE', 'ITERATION_LIMIT', 'INSTANCE_LIMIT'));
