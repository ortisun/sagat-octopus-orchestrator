-- Rollback is an operator/quiesced-service operation. A run that a schedule started keeps saying so:
-- dropping the columns would lose that evidence, so the rollback refuses while any run carries it.
LOCK TABLE runs IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM runs WHERE schedule_id IS NOT NULL) THEN
        RAISE EXCEPTION 'runs started by schedules are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TRIGGER runs_schedule_provenance ON runs;
DROP FUNCTION runs_schedule_provenance_guard();
DROP INDEX runs_schedule_idx;
ALTER TABLE runs
    DROP CONSTRAINT runs_schedule_revision_positive,
    DROP CONSTRAINT runs_schedule_action_kind_valid,
    DROP CONSTRAINT runs_schedule_provenance_whole,
    DROP COLUMN schedule_revision,
    DROP COLUMN schedule_action_kind,
    DROP COLUMN schedule_nominal_at,
    DROP COLUMN schedule_id;
