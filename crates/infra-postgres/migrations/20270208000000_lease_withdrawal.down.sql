-- Rollback is an operator/quiesced-service operation (ADR-0098, "Rollback"): remove the requirement first, then stop the
-- processes of this build. A recorded requirement of `lease-withdrawal-v1`, a history that names it, a live process that lists
-- it, or a journal that holds `step.lease_withdrawn` (an older binary cannot replay it) is refused.
LOCK TABLE serving_instances, platform_requirements, platform_requirement_history IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'lease-withdrawal-v1') THEN
        RAISE EXCEPTION 'lease-withdrawal-v1 is a recorded requirement; remove it before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM platform_requirement_history WHERE capability = 'lease-withdrawal-v1') THEN
        RAISE EXCEPTION 'the requirement history names lease-withdrawal-v1; it is evidence and is kept: keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM serving_instances WHERE 'lease-withdrawal-v1' = ANY (capabilities)) THEN
        RAISE EXCEPTION 'a registered process lists lease-withdrawal-v1; stop the processes of this build before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM run_events WHERE event_type = 'step.lease_withdrawn') THEN
        RAISE EXCEPTION 'a journal holds step.lease_withdrawn, which an older binary cannot replay; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP FUNCTION lease_withdrawal_required();
DROP TRIGGER platform_requirements_replay_floor_guard ON platform_requirements;
DROP FUNCTION platform_requirements_replay_floor_guard();
DROP FUNCTION platform_capability_census(text, text[]);

ALTER TABLE platform_requirement_history
    DROP CONSTRAINT platform_requirement_history_capability_known,
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1'));
ALTER TABLE platform_requirements
    DROP CONSTRAINT platform_requirements_capability_known,
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1'));
ALTER TABLE serving_instances
    DROP CONSTRAINT serving_instances_capabilities_known,
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1']::text[]
        AND cardinality(capabilities) <= 32);
