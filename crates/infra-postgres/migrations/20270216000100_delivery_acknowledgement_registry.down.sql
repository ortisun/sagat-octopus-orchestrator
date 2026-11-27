-- Rollback of `20270216000100_delivery_acknowledgement_registry`, an operator/quiesced-service operation, possible only while
-- nothing names the capability: a recorded requirement or a history that names `delivery-acknowledgement-v1`, or a
-- registered process, live or not, that lists it, is refused. Otherwise the trigger, its function and the requirements read
-- go (by full signature), and the checks list what `20270211000100_lease_withdrawal_v2` listed, `lease-withdrawal-v2` kept.
LOCK TABLE serving_instances, platform_requirements, platform_requirement_history IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'delivery-acknowledgement-v1') THEN
        RAISE EXCEPTION 'delivery-acknowledgement-v1 is a recorded requirement; remove it before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM platform_requirement_history WHERE capability = 'delivery-acknowledgement-v1') THEN
        RAISE EXCEPTION 'the requirement history names delivery-acknowledgement-v1; it is evidence and is kept: keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM serving_instances WHERE 'delivery-acknowledgement-v1' = ANY (capabilities)) THEN
        RAISE EXCEPTION 'a registered process lists delivery-acknowledgement-v1; stop the processes of this build (and remove the rows of those confirmed stopped) before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TRIGGER platform_requirements_delivery_acknowledgement_guard ON platform_requirements;
DROP FUNCTION platform_requirements_delivery_acknowledgement_guard();
DROP FUNCTION delivery_acknowledgement_required();

ALTER TABLE platform_requirement_history
    DROP CONSTRAINT platform_requirement_history_capability_known,
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2'));
ALTER TABLE platform_requirements
    DROP CONSTRAINT platform_requirements_capability_known,
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2'));
ALTER TABLE serving_instances
    DROP CONSTRAINT serving_instances_capabilities_known,
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                              'lease-withdrawal-v1', 'lease-withdrawal-v2']::text[]
        AND cardinality(capabilities) <= 32);
