-- Rollback is an operator/quiesced-service operation, not an online caller protocol.
-- SQLx keeps the retained-data guard and the removal in one migration transaction. Schedules,
-- their history and a recorded handover are authority and evidence: rolling back never destroys them.
LOCK TABLE schedules, schedule_revisions, schedule_actions, schedule_backfills, schedule_obligations,
    schedule_trigger_handover IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM schedules)
        OR EXISTS (SELECT 1 FROM schedule_revisions)
        OR EXISTS (SELECT 1 FROM schedule_actions)
        OR EXISTS (SELECT 1 FROM schedule_backfills)
        OR EXISTS (SELECT 1 FROM schedule_obligations) THEN
        RAISE EXCEPTION 'schedules and their history are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM schedule_trigger_handover WHERE state <> 'LEGACY') THEN
        RAISE EXCEPTION 'the cron triggers were handed over to the schedule pass; a rollback would stop them'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TABLE schedule_trigger_handover;
DROP TABLE schedule_obligations;
DROP TABLE schedule_backfills;
DROP TABLE schedule_actions;
DROP FUNCTION schedule_actions_guard();
ALTER TABLE schedules DROP CONSTRAINT schedules_head_revision_fk;
DROP TABLE schedule_revisions;
DROP TABLE schedules;
DROP FUNCTION schedules_identity_guard();
