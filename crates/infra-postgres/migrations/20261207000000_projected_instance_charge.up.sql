-- This is RunState::charged_instances, not the lifetime admission/reservation budget. Every
-- scoped row contributes one; a root contributes unless PENDING or CANCELLED. Root cancellation
-- can therefore decrease this projection without releasing an admission or a reservation.
-- Serialize the initial snapshot and trigger installation with writers. Runtime writes lock the
-- run before steps/usage; taking the run table first lets existing writers drain. This migration
-- intentionally pauses writers during its backfill; it is not an online, lock-free migration.
LOCK TABLE runs, steps, run_instance_usage IN EXCLUSIVE MODE;

ALTER TABLE run_instance_usage ADD COLUMN charged_instances bigint NOT NULL DEFAULT 0;
UPDATE run_instance_usage usage SET charged_instances = counted.instances
FROM (
    SELECT tenant_id, run_id, count(*)::bigint AS instances
    FROM steps
    WHERE cardinality(instance_path_nodes) > 0 OR status NOT IN ('PENDING', 'CANCELLED')
    GROUP BY tenant_id, run_id
) counted
WHERE usage.tenant_id=counted.tenant_id AND usage.run_id=counted.run_id;
ALTER TABLE run_instance_usage ADD CONSTRAINT run_instance_usage_charged_nonnegative
    CHECK (charged_instances >= 0);

CREATE FUNCTION project_step_instance_charge() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    previous_charge bigint := 0;
    next_charge bigint := 0;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        previous_charge := CASE WHEN cardinality(OLD.instance_path_nodes) > 0
            OR OLD.status NOT IN ('PENDING', 'CANCELLED') THEN 1 ELSE 0 END;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        next_charge := CASE WHEN cardinality(NEW.instance_path_nodes) > 0
            OR NEW.status NOT IN ('PENDING', 'CANCELLED') THEN 1 ELSE 0 END;
    END IF;
    IF TG_OP = 'UPDATE' AND OLD.tenant_id=NEW.tenant_id AND OLD.run_id=NEW.run_id THEN
        next_charge := next_charge - previous_charge;
        previous_charge := 0;
    END IF;
    IF previous_charge <> 0 THEN
        UPDATE public.run_instance_usage SET charged_instances=charged_instances-previous_charge
        WHERE tenant_id=OLD.tenant_id AND run_id=OLD.run_id;
        -- A parent deletion may already have cascaded to usage. An ordinary move must find it.
        IF NOT FOUND AND TG_OP <> 'DELETE' THEN
            RAISE EXCEPTION 'missing run instance charge projection' USING ERRCODE='check_violation';
        END IF;
    END IF;
    IF next_charge <> 0 THEN
        UPDATE public.run_instance_usage SET charged_instances=charged_instances+next_charge
        WHERE tenant_id=NEW.tenant_id AND run_id=NEW.run_id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'missing run instance charge projection' USING ERRCODE='check_violation';
        END IF;
    END IF;
    RETURN NULL;
END;
$$;
REVOKE ALL ON FUNCTION project_step_instance_charge() FROM PUBLIC;
CREATE TRIGGER steps_instance_charge_projection
    AFTER INSERT OR DELETE OR UPDATE OF status, tenant_id, run_id, instance_path_nodes ON steps
    FOR EACH ROW EXECUTE FUNCTION project_step_instance_charge();
ALTER TABLE steps ENABLE ALWAYS TRIGGER steps_instance_charge_projection;
