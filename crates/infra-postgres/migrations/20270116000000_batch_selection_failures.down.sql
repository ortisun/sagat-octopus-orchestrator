-- Rollback is an operator/quiesced-service operation. A job that is still selecting and has failed rounds on record would lose its
-- progress towards the bound (the schema this restores has no place for the count, and an older engine would take the job up as if
-- nothing had failed): that is refused, and the operator lets the job end (about 25 minutes from its first failed round at the
-- most) or cancels it, or keeps a compatible reader. A job that ended `SELECTION_FAILED` keeps its count as evidence of how many
-- rounds failed and its `failure_reason` says the rest, so it does not stand in the way of a rollback (it would otherwise block
-- every rollback for the retention of every job that ever failed); the count goes with the column, the reason stays. A job that is
-- cancelling or cancelled is not selecting any more and does not either.
LOCK TABLE batch_jobs IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM batch_jobs WHERE state = 'SELECTING' AND selection_failures > 0) THEN
        RAISE EXCEPTION 'failed selection rounds of a batch job that is still selecting are retained; let it end or cancel it, or keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

COMMENT ON COLUMN batch_jobs.next_slot_at IS NULL;
ALTER TABLE batch_jobs DROP COLUMN selection_failures;
