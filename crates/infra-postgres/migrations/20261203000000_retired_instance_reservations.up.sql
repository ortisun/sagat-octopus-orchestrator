-- A run's instance budget is a lifetime charge. An unused reservation that a draining
-- iteration can no longer consume is retired, never refunded for unrelated forward work.

ALTER TABLE run_instance_usage
    ADD COLUMN retired_reservations integer NOT NULL DEFAULT 0;

ALTER TABLE run_instance_usage DROP CONSTRAINT run_instance_usage_counts;
ALTER TABLE run_instance_usage ADD CONSTRAINT run_instance_usage_counts CHECK (
    admitted_instances >= 0
    AND reserved_instances >= 0
    AND retired_reservations >= 0);

-- Older drains subtracted unused reservations from the run projection. Reconstruct that
-- immutable charge from each iteration's initial reservation, the exact-path steps that
-- materialized from it, and the reservations that remain available for compensation.
WITH retired AS (
    SELECT tenant_id, run_id,
           sum(greatest(reserved_instances - remaining_reservations - materialized, 0))::integer AS count
    FROM (
        SELECT i.tenant_id, i.run_id, i.scope_id, i.iteration_index,
               i.reserved_instances, i.remaining_reservations,
               greatest(count(s.step_id)::integer - 1, 0) AS materialized
        FROM collection_iterations i
        LEFT JOIN steps s
          ON s.tenant_id = i.tenant_id AND s.run_id = i.run_id
         AND s.instance_path_nodes = i.entry_path_nodes
         AND s.instance_path_indices = i.entry_path_indices
        WHERE i.lifecycle = 'DRAINED'
        GROUP BY i.tenant_id, i.run_id, i.scope_id, i.iteration_index,
                 i.reserved_instances, i.remaining_reservations
    ) collection_drains
    GROUP BY tenant_id, run_id
), loop_retired AS (
    SELECT tenant_id, run_id,
           sum(greatest(reserved_instances - remaining_reservations - materialized, 0))::integer AS count
    FROM (
        SELECT i.tenant_id, i.run_id, i.owner_instance_id, i.iteration_index,
               i.reserved_instances, i.remaining_reservations,
               greatest(count(s.step_id)::integer - 1, 0) AS materialized
        FROM loop_iterations i
        LEFT JOIN steps s
          ON s.tenant_id = i.tenant_id AND s.run_id = i.run_id
         AND s.instance_path_nodes = i.entry_path_nodes
         AND s.instance_path_indices = i.entry_path_indices
        WHERE i.lifecycle = 'DRAINED'
        GROUP BY i.tenant_id, i.run_id, i.owner_instance_id, i.iteration_index,
                 i.reserved_instances, i.remaining_reservations
    ) loop_drains
    GROUP BY tenant_id, run_id
), combined AS (
    SELECT tenant_id, run_id, sum(count)::integer AS count
    FROM (
        SELECT * FROM retired
        UNION ALL
        SELECT * FROM loop_retired
    ) charges
    GROUP BY tenant_id, run_id
)
UPDATE run_instance_usage usage
SET retired_reservations = combined.count
FROM combined
WHERE usage.tenant_id = combined.tenant_id AND usage.run_id = combined.run_id;

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
           AND admitted_instances + reserved_instances + retired_reservations < max_instances;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'run instance budget is unavailable'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NULL;
END;
$$;
REVOKE ALL ON FUNCTION account_root_instance_admission() FROM PUBLIC;
