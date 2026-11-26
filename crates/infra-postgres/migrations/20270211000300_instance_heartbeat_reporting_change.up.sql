-- Feature 006, the follow-up of pull request #260 (request (7) of decision D006-059 of specs/006-sdk-multilanguage/research.md;
-- Task 006-072; decision D006-098; the amendment of ADR-0082 decision 5): the beat that never waits for the requirement epoch
-- and says when an owner transaction holds it.
--
-- It does what `instance_heartbeat` (migration `20270113000000_instance_registry`) does, then takes the epoch row
-- `FOR SHARE SKIP LOCKED`: the locked read's epoch is returned with `out_change_in_progress = false`, so an activation that
-- committed during the beat is never seen as the old epoch with no change in progress, and an activation that starts
-- meanwhile waits for this beat and then counts it live (its census is a statement after its lock, under READ COMMITTED).
-- Only when an owner transaction (an activation, a removal, the lock of the handover or of a hand-made change) holds the row
-- does it read the committed epoch plainly and return `true`: the beat succeeds, and the process keeps its capability
-- decision only inside its window (ADR-0082, amendment of 2026-11-24). The epoch is taken after the process's own row and the
-- day rule, so the share lock lasts only for the counts and the commit, and it is never waited for. A missing epoch row is
-- an error, never an epoch.
--
-- A function of its own name, not an overload: the two-argument `instance_heartbeat` keeps its contract for older binaries.
CREATE FUNCTION instance_heartbeat_reporting_change(p_instance uuid, p_token bytea)
RETURNS TABLE (out_found boolean, out_epoch bigint, out_live_kinds text[], out_live_counts integer[],
               out_change_in_progress boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_found  boolean;
    v_epoch  bigint;
    v_change boolean;
    v_kinds  text[];
    v_counts integer[];
BEGIN
    -- another process's row, or a token that does not match, is a row that is not found
    UPDATE public.serving_instances SET last_seen = clock_timestamp()
     WHERE instance_id = p_instance AND token_hash = sha256(coalesce(p_token, ''::bytea));
    v_found := FOUND;
    DELETE FROM public.serving_instances d WHERE d.instance_id IN (
        SELECT s.instance_id FROM public.serving_instances s
         WHERE s.last_seen < clock_timestamp() - interval '1 day'
         ORDER BY s.instance_id
         LIMIT 100
         FOR UPDATE SKIP LOCKED);
    SELECT e.epoch INTO v_epoch FROM public.platform_requirement_epoch e FOR SHARE SKIP LOCKED;
    IF FOUND THEN
        v_change := false;
    ELSE
        SELECT e.epoch INTO v_epoch FROM public.platform_requirement_epoch e;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'the requirement epoch row is missing'
                USING ERRCODE = 'object_not_in_prerequisite_state';
        END IF;
        v_change := true;
    END IF;
    SELECT coalesce(array_agg(t.kind ORDER BY t.kind), '{}'), coalesce(array_agg(t.n::integer ORDER BY t.kind), '{}')
      INTO v_kinds, v_counts
      FROM (SELECT s.kind, count(*) AS n FROM public.serving_instances s
             WHERE s.last_seen > clock_timestamp() - interval '30 seconds'
             GROUP BY s.kind) t;
    RETURN QUERY SELECT v_found, v_epoch, v_kinds, v_counts, v_change;
END;
$$;
REVOKE ALL ON FUNCTION instance_heartbeat_reporting_change(uuid, bytea) FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION instance_heartbeat_reporting_change(uuid, bytea) TO sagat_app;
    END IF;
END $grants$;
