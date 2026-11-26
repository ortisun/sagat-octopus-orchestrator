-- Feature 006, delivery acknowledgement (protocol 1.8, ADR-0099 decision 10; Task 006-046 of
-- specs/006-sdk-multilanguage/tasks.md; decision D006-104): the registry's part, dormant. The capability
-- `delivery-acknowledgement-v1` becomes known to the three closed checks (added `NOT VALID`, validated by
-- `20270216000200_delivery_acknowledgement_registry_validate`), but no process lists it and no activation entry records it yet
-- (Task 006-057). What every withdrawal of a marked lease reads, and the trigger that keeps the order of the two requirements:
--
-- 1. `delivery_acknowledgement_required()` reads both requirement rows `FOR SHARE`, in the order their removals take them
--    (`delivery-acknowledgement-v1`, then `lease-withdrawal-v2`), so a removal waits for a withdrawal in flight and the two
--    cannot deadlock. It returns `lease-withdrawal-v2` as recorded only when it is recorded for both kinds,
--    `delivery-acknowledgement-v1` as recorded for engines, and the latter's `required_since` (a lease claimed before the
--    current recording never counts). The runtime role may run it; it reads two rows and changes nothing.
-- 2. A trigger on `platform_requirements`, enabled `ALWAYS` (hand SQL cannot skip it), which takes the epoch row `FOR UPDATE`
--    itself and refuses outside READ COMMITTED: recording `delivery-acknowledgement-v1` while `lease-withdrawal-v2` is not
--    recorded for both kinds, or while any registered process, live or not, lacks `lease-withdrawal-v2` (the census at the
--    insert never sees a process that comes back later, so a lapsed row counts too); and removing `lease-withdrawal-v2` while
--    `delivery-acknowledgement-v1` is recorded.
ALTER TABLE serving_instances
    DROP CONSTRAINT serving_instances_capabilities_known,
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                              'lease-withdrawal-v1', 'lease-withdrawal-v2', 'delivery-acknowledgement-v1']::text[]
        AND cardinality(capabilities) <= 32) NOT VALID;

ALTER TABLE platform_requirements
    DROP CONSTRAINT platform_requirements_capability_known,
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2', 'delivery-acknowledgement-v1')) NOT VALID;

ALTER TABLE platform_requirement_history
    DROP CONSTRAINT platform_requirement_history_capability_known,
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2', 'delivery-acknowledgement-v1')) NOT VALID;

CREATE FUNCTION delivery_acknowledgement_required()
RETURNS TABLE (out_lease_withdrawal_v2 boolean, out_delivery_acknowledgement boolean, out_required_since timestamptz)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_since        timestamptz;
    v_acknowledged boolean;
    v_floor        boolean;
BEGIN
    SELECT r.required_since INTO v_since FROM public.platform_requirements r
     WHERE r.capability = 'delivery-acknowledgement-v1'
       AND r.kinds @> ARRAY['EXECUTION_ENGINE']::text[]
       FOR SHARE;
    v_acknowledged := FOUND;
    PERFORM 1 FROM public.platform_requirements r
     WHERE r.capability = 'lease-withdrawal-v2'
       AND r.kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[]
       FOR SHARE;
    v_floor := FOUND;
    RETURN QUERY SELECT v_floor, v_acknowledged, CASE WHEN v_acknowledged THEN v_since END;
END;
$$;
REVOKE ALL ON FUNCTION delivery_acknowledgement_required() FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION delivery_acknowledgement_required() TO sagat_app;
    END IF;
END $grants$;

CREATE FUNCTION platform_requirements_delivery_acknowledgement_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_lacking uuid;
BEGIN
    IF (TG_OP = 'INSERT' AND NEW.capability = 'delivery-acknowledgement-v1')
       OR (TG_OP = 'DELETE' AND OLD.capability = 'lease-withdrawal-v2') THEN
        IF current_setting('transaction_isolation') <> 'read committed' THEN
            -- the words of the replay-floor guard, which runs after this trigger on a removal
            RAISE EXCEPTION '% is % only under READ COMMITTED (this transaction runs under %): a stricter level keeps a snapshot that cannot see a change committed meanwhile',
                CASE WHEN TG_OP = 'INSERT' THEN NEW.capability ELSE OLD.capability END,
                CASE WHEN TG_OP = 'INSERT' THEN 'recorded' ELSE 'removed' END,
                current_setting('transaction_isolation')
                USING ERRCODE = 'restrict_violation';
        END IF;
        -- the epoch, as every change of a requirement takes it: hand SQL that did not is serialised here
        PERFORM 1 FROM public.platform_requirement_epoch e FOR UPDATE;
    END IF;
    IF TG_OP = 'INSERT' AND NEW.capability = 'delivery-acknowledgement-v1' THEN
        IF NOT EXISTS (SELECT 1 FROM public.platform_requirements r
                        WHERE r.capability = 'lease-withdrawal-v2'
                          AND r.kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[]) THEN
            RAISE EXCEPTION 'delivery-acknowledgement-v1 is recorded only after lease-withdrawal-v2 is recorded for control planes and engines'
                USING ERRCODE = 'restrict_violation';
        END IF;
        SELECT s.instance_id INTO v_lacking FROM public.serving_instances s
         WHERE NOT ('lease-withdrawal-v2' = ANY (s.capabilities))
         ORDER BY s.instance_id LIMIT 1;
        IF FOUND THEN
            RAISE EXCEPTION 'delivery-acknowledgement-v1 is not recorded while a registered process, live or not, lacks lease-withdrawal-v2 (instance %): upgrade it, or confirm it stopped and remove its row', v_lacking
                USING ERRCODE = 'restrict_violation';
        END IF;
        RETURN NEW;
    END IF;
    IF TG_OP = 'DELETE' AND OLD.capability = 'lease-withdrawal-v2'
       AND EXISTS (SELECT 1 FROM public.platform_requirements r WHERE r.capability = 'delivery-acknowledgement-v1') THEN
        RAISE EXCEPTION 'lease-withdrawal-v2 is not removed while delivery-acknowledgement-v1 is recorded: remove delivery-acknowledgement-v1 first'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION platform_requirements_delivery_acknowledgement_guard() FROM PUBLIC;
CREATE TRIGGER platform_requirements_delivery_acknowledgement_guard BEFORE INSERT OR DELETE ON platform_requirements
    FOR EACH ROW EXECUTE FUNCTION platform_requirements_delivery_acknowledgement_guard();
ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirements_delivery_acknowledgement_guard;
