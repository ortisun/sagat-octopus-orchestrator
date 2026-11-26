-- Rollback is an operator/quiesced-service operation. A queued waiter whose declaration is above its filed limit is
-- bounded by that declaration under the namespace's current ceiling; dropping the column would lose it, and no older
-- binary can recover it (the run's version pins it, the waiter row does not repeat it). Wait for those waiters to be
-- granted, time out or be cancelled, or keep a compatible reader; a row whose two limits agree loses nothing.
LOCK TABLE concurrency_waiters, rate_limit_waiters IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM concurrency_waiters WHERE pinned_limit > declared_limit)
       OR EXISTS (SELECT 1 FROM rate_limit_waiters WHERE pinned_limit > declared_limit) THEN
        RAISE EXCEPTION 'queued waiters are bounded by a declaration above their filed limit; let them drain or keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE rate_limit_waiters DROP CONSTRAINT rate_limit_waiters_pinned_limit_valid;
ALTER TABLE concurrency_waiters DROP CONSTRAINT concurrency_waiters_pinned_limit_valid;
ALTER TABLE rate_limit_waiters DROP COLUMN pinned_limit;
ALTER TABLE concurrency_waiters DROP COLUMN pinned_limit;
