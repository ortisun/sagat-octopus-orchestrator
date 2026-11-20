-- Rollback is an operator/quiesced-service operation. A recovery in the middle of its steps would be
-- forgotten, and the older pass would plan the backlog again and act on it past the ceiling of the
-- recovery, so the rollback refuses while any schedule is carrying one out.
LOCK TABLE schedules IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM schedules WHERE recovery_until IS NOT NULL) THEN
        RAISE EXCEPTION 'a schedule is in the middle of a recovery; let it finish before the rollback'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE schedules
    DROP CONSTRAINT schedules_recovery_acts_bounded,
    DROP CONSTRAINT schedules_recovery_whole,
    DROP COLUMN recovery_acts,
    DROP COLUMN recovery_until;
