-- Feature 022 (T022-094; FR-022-053; ADR-0089 decision 6, ADR-0050 decision 9): the two flow-control capabilities join the
-- closed list of the instance-capability registry (`specs/018-schedules-bulk-operations/data-model.md`, rule 5: a value is
-- owned by the feature that introduces it). `flow-control-concurrency-v1` and `flow-control-rate-v1` are required of the
-- control plane and the engine; a process lists one only when it runs the gate (D022-037).
--
-- Stamped after feature 006's newest migration, `20270211000300_instance_heartbeat_reporting_change` (decision D006-033 of
-- `specs/006-sdk-multilanguage/research.md`): feature 022 merges after feature 006, which redefines the same three checks
-- (`20270208000000_lease_withdrawal` and `20270211000100_lease_withdrawal_v2`), so this migration and its validation were
-- restamped from `20270130000200` and `20270130000700` (and again from `20270208000300` and `20270208000400` when
-- `lease-withdrawal-v2` merged) to follow the other's newest. An older stamp would be applied by sqlx after the newer ones on
-- a database of `develop` and narrow the lists again. The lists are the union of both features' values, the tokens of
-- `Capability::ALL` in its order; the down restores feature 006's list of `20270211000100_lease_withdrawal_v2`, the newest
-- before this one (D006-083).
--
-- Additive: the three checks are widened, nothing else changes. No row uses the new values until a process lists one.
-- Each check is replaced under the same name with the eight values, added `NOT VALID` (a superset of the old one, so every
-- stored row satisfies it) and validated by `20270211000500_flow_control_capabilities_validate` in a transaction of its own
-- (D043-145).
ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE serving_instances ADD CONSTRAINT serving_instances_capabilities_known CHECK (
    capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                          'lease-withdrawal-v1', 'lease-withdrawal-v2',
                          'flow-control-concurrency-v1', 'flow-control-rate-v1']::text[]
    AND cardinality(capabilities) <= 32) NOT VALID;
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirements ADD CONSTRAINT platform_requirements_capability_known CHECK (
    capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                   'lease-withdrawal-v1', 'lease-withdrawal-v2',
                   'flow-control-concurrency-v1', 'flow-control-rate-v1')) NOT VALID;
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_known;
ALTER TABLE platform_requirement_history ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
    capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                   'lease-withdrawal-v1', 'lease-withdrawal-v2',
                   'flow-control-concurrency-v1', 'flow-control-rate-v1')) NOT VALID;

-- The feature step of a flow-control activation (T022-094): how many of the required kinds have no live process (seen
-- within 30 seconds) that lists the capability. The registry records a requirement whenever no live process lacks it, an
-- empty fleet included; the step refuses unless this is zero. Operator only, like the activation functions.
CREATE FUNCTION flow_control_kinds_without_capable_process(p_capability text, p_kinds text[]) RETURNS bigint
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT count(*) FROM unnest(p_kinds) AS k(kind)
     WHERE NOT EXISTS (SELECT 1 FROM public.serving_instances s
                        WHERE s.kind = k.kind AND s.last_seen > clock_timestamp() - interval '30 seconds'
                          AND p_capability = ANY (s.capabilities));
$$;
REVOKE ALL ON FUNCTION flow_control_kinds_without_capable_process(text, text[]) FROM PUBLIC;
