-- Rollback is an operator/quiesced-service operation. An `INPUT_INVALID` action, a live process that lists
-- `run-input-validation-v1`, or a recorded requirement of it is a value the restored constraints do not know: that is refused,
-- and the operator removes the requirement and stops the 0.3.0 processes first; an `INPUT_INVALID` action stays as evidence of
-- what the schedule decided, so a rollback waits for the retention of the actions (or keeps a compatible reader).
LOCK TABLE schedule_actions, serving_instances, platform_requirements, platform_requirement_history IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM schedule_actions WHERE outcome = 'INPUT_INVALID') THEN
        RAISE EXCEPTION 'schedule actions recorded INPUT_INVALID are retained; keep a compatible reader until they are purged'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'run-input-validation-v1') THEN
        RAISE EXCEPTION 'run-input-validation-v1 is a recorded requirement; remove it before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM platform_requirement_history WHERE capability = 'run-input-validation-v1') THEN
        RAISE EXCEPTION 'the requirement history names run-input-validation-v1; it is evidence and is kept: keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM serving_instances WHERE 'run-input-validation-v1' = ANY (capabilities)) THEN
        RAISE EXCEPTION 'a registered process lists run-input-validation-v1; stop the 0.3.0 processes before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE platform_requirement_history
    ADD CONSTRAINT platform_requirement_history_capability_valid CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1'));
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_known;

ALTER TABLE platform_requirements
    ADD CONSTRAINT platform_requirements_capability_valid CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1'));
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_known;

ALTER TABLE serving_instances
    ADD CONSTRAINT serving_instances_capabilities_closed CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1']::text[]
        AND cardinality(capabilities) <= 32);
ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_known;

ALTER TABLE schedule_actions
    ADD CONSTRAINT schedule_actions_outcome_valid CHECK (outcome IN
        ('STARTED', 'BUFFERED', 'SKIPPED_OVERLAP', 'DROPPED_BUFFER_FULL', 'DROPPED_DELETED', 'COALESCED',
         'MISSED_CATCHUP_WINDOW', 'ALREADY_ACTED', 'AUTHORITY_REVOKED', 'BUSINESS_ID_CONFLICT'));
ALTER TABLE schedule_actions DROP CONSTRAINT schedule_actions_outcome_known;
