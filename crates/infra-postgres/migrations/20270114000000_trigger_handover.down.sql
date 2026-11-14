-- A handover that happened is evidence of what the fleet was asked to honour (the requirement `schedules-v1` and the
-- history of its activation stay with it): rolling back never destroys it. Keep a compatible reader, or move the state
-- by hand in a quiesced service after reading `docs/runbooks/schedules.md` (the rollback hazard, G1011).
LOCK TABLE schedule_trigger_handover IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM schedule_trigger_handover WHERE state <> 'LEGACY') THEN
        RAISE EXCEPTION 'the cron triggers were handed over to the schedule pass; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TRIGGER platform_requirements_handover_guard ON platform_requirements;
DROP FUNCTION platform_requirements_handover_guard();
DROP FUNCTION schedule_trigger_handover_census(boolean);
DROP INDEX workflow_triggers_cron_last_fire_idx;
DROP INDEX schedules_trigger_shard_idx;
DROP INDEX workflow_triggers_cron_shard_idx;
DROP FUNCTION schedule_trigger_unconverged();
DROP FUNCTION schedule_trigger_convergence(integer);
DROP VIEW schedule_trigger_drift;
DROP FUNCTION schedule_trigger_handover_complete(text);
DROP FUNCTION schedule_trigger_handover_lock();
DROP TRIGGER schedule_trigger_handover_guard ON schedule_trigger_handover;
DROP FUNCTION schedule_trigger_handover_guard();
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_changed_by_printable;
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_changed_by_nonempty;
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_db_role_bounded;
ALTER TABLE schedule_trigger_handover DROP CONSTRAINT schedule_trigger_handover_db_role_consistent;
ALTER TABLE schedule_trigger_handover DROP COLUMN changed_db_role;
