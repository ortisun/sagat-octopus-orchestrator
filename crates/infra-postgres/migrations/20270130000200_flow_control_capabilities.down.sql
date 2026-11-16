-- Rollback is an operator/quiesced-service operation. A process that lists a flow-control capability, a requirement of
-- one or its history says what the registry of this version understood: rolling back never destroys any of them, so it
-- refuses while one exists.
LOCK TABLE serving_instances, platform_requirements, platform_requirement_history IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM serving_instances
                WHERE capabilities && ARRAY['flow-control-concurrency-v1', 'flow-control-rate-v1']::text[])
       OR EXISTS (SELECT 1 FROM platform_requirements
                   WHERE capability IN ('flow-control-concurrency-v1', 'flow-control-rate-v1'))
       OR EXISTS (SELECT 1 FROM platform_requirement_history
                   WHERE capability IN ('flow-control-concurrency-v1', 'flow-control-rate-v1')) THEN
        RAISE EXCEPTION 'a flow-control capability is listed, required or recorded in the registry; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_closed;
ALTER TABLE serving_instances ADD CONSTRAINT serving_instances_capabilities_closed CHECK (
    capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1']::text[]
    AND cardinality(capabilities) <= 32);
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_valid;
ALTER TABLE platform_requirements ADD CONSTRAINT platform_requirements_capability_valid CHECK (
    capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1'));
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_valid;
ALTER TABLE platform_requirement_history ADD CONSTRAINT platform_requirement_history_capability_valid CHECK (
    capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1'));
