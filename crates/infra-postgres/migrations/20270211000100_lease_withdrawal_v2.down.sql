-- Rollback of `20270211000100_lease_withdrawal_v2`, an operator/quiesced-service operation (docs/runbooks/lease-withdrawal.md,
-- "Rollback"), possible only while nothing names the capability: a recorded requirement of `lease-withdrawal-v2`, a history
-- that names it, a registered process that lists it, or a journal that holds `step.lease_withdrawn` with the cause
-- `NOT_ACKNOWLEDGED` (a binary that does not list it cannot replay that cause) is refused. Otherwise the guard is the one of
-- `20270211000000_lease_withdrawal_functions`, the index goes, and the checks list what `20270208000000_lease_withdrawal`
-- listed.
LOCK TABLE serving_instances, platform_requirements, platform_requirement_history IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM platform_requirements WHERE capability = 'lease-withdrawal-v2') THEN
        RAISE EXCEPTION 'lease-withdrawal-v2 is a recorded requirement; remove it before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM platform_requirement_history WHERE capability = 'lease-withdrawal-v2') THEN
        RAISE EXCEPTION 'the requirement history names lease-withdrawal-v2; it is evidence and is kept: keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM serving_instances WHERE 'lease-withdrawal-v2' = ANY (capabilities)) THEN
        RAISE EXCEPTION 'a registered process lists lease-withdrawal-v2; stop the processes of this build before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
    IF EXISTS (SELECT 1 FROM run_events
                WHERE event_type = 'step.lease_withdrawn' AND detail->>'cause' = 'NOT_ACKNOWLEDGED') THEN
        RAISE EXCEPTION 'a journal holds step.lease_withdrawn with the cause NOT_ACKNOWLEDGED, which a binary without lease-withdrawal-v2 cannot replay; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

CREATE OR REPLACE FUNCTION platform_requirements_replay_floor_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF OLD.capability = 'lease-withdrawal-v1' THEN
        IF current_setting('transaction_isolation') <> 'read committed' THEN
            RAISE EXCEPTION 'lease-withdrawal-v1 is removed only under READ COMMITTED (this transaction runs under %): a stricter level keeps a snapshot that cannot see a withdrawal committed meanwhile', current_setting('transaction_isolation')
                USING ERRCODE = 'restrict_violation';
        END IF;
        IF EXISTS (SELECT 1 FROM public.run_events e WHERE e.event_type = 'step.lease_withdrawn') THEN
            RAISE EXCEPTION 'lease-withdrawal-v1 cannot be removed while a journal holds step.lease_withdrawn: a build without it cannot replay the event; set SAGAT_LEASE_WITHDRAWAL=off on the engines to stop new withdrawals of unmarked leases'
                USING ERRCODE = 'restrict_violation';
        END IF;
    END IF;
    RETURN OLD;
END;
$$;
REVOKE ALL ON FUNCTION platform_requirements_replay_floor_guard() FROM PUBLIC;

DROP INDEX IF EXISTS run_events_lease_not_acknowledged_idx;

ALTER TABLE platform_requirement_history
    DROP CONSTRAINT platform_requirement_history_capability_known,
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1'));
ALTER TABLE platform_requirements
    DROP CONSTRAINT platform_requirements_capability_known,
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1'));
ALTER TABLE serving_instances
    DROP CONSTRAINT serving_instances_capabilities_known,
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                              'lease-withdrawal-v1']::text[]
        AND cardinality(capabilities) <= 32);
