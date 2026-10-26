DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM run_instance_usage WHERE retired_reservations <> 0)
    THEN
        RAISE EXCEPTION 'retired instance reservations cannot be refunded safely'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$rollback$;

CREATE OR REPLACE FUNCTION account_root_instance_admission() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    should_charge boolean := false;
BEGIN
    IF cardinality(NEW.instance_path_nodes) = 0 THEN
        IF TG_OP = 'INSERT' THEN
            should_charge := NEW.status NOT IN ('PENDING', 'CANCELLED');
        ELSIF OLD.status = 'PENDING' AND NEW.status NOT IN ('PENDING', 'CANCELLED') THEN
            should_charge := true;
        END IF;
    END IF;
    IF should_charge THEN
        UPDATE public.run_instance_usage
           SET admitted_instances = admitted_instances + 1
         WHERE tenant_id = NEW.tenant_id AND run_id = NEW.run_id
           AND admitted_instances + reserved_instances < max_instances;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'run instance budget is unavailable'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NULL;
END;
$$;
REVOKE ALL ON FUNCTION account_root_instance_admission() FROM PUBLIC;

ALTER TABLE run_instance_usage DROP CONSTRAINT run_instance_usage_counts;
ALTER TABLE run_instance_usage ADD CONSTRAINT run_instance_usage_counts CHECK (
    admitted_instances >= 0 AND reserved_instances >= 0);
ALTER TABLE run_instance_usage DROP COLUMN retired_reservations;
