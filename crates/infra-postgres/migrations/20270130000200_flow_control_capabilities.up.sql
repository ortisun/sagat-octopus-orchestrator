-- Feature 022 (T022-094; FR-022-053; ADR-0089 decision 6, ADR-0050 decision 9): the two flow-control capabilities join the
-- closed list of the instance-capability registry (`specs/018-schedules-bulk-operations/data-model.md`, rule 5: a value is
-- owned by the feature that introduces it). `flow-control-concurrency-v1` and `flow-control-rate-v1` are required of the
-- control plane and the engine; a process lists one only when it runs the gate (D022-037), which no build does yet.
--
-- Additive: the three checks are widened, nothing else changes. No row uses the new values until a process lists one.
ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_closed;
ALTER TABLE serving_instances ADD CONSTRAINT serving_instances_capabilities_closed CHECK (
    capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1',
                          'flow-control-concurrency-v1', 'flow-control-rate-v1']::text[]
    AND cardinality(capabilities) <= 32);
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_valid;
ALTER TABLE platform_requirements ADD CONSTRAINT platform_requirements_capability_valid CHECK (
    capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1',
                   'flow-control-concurrency-v1', 'flow-control-rate-v1'));
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_valid;
ALTER TABLE platform_requirement_history ADD CONSTRAINT platform_requirement_history_capability_valid CHECK (
    capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1',
                   'flow-control-concurrency-v1', 'flow-control-rate-v1'));
