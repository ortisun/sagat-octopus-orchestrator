-- Rollback of `20270208000200_lease_withdrawal_floor`: the removal waits for its locks as before, the guard is the one of
-- `20270208000000_lease_withdrawal`, the census loses the live kinds, and the index goes. Nothing is stored that an older binary cannot read.
ALTER FUNCTION platform_requirement_remove(text, text) RESET lock_timeout;

CREATE OR REPLACE FUNCTION platform_requirements_replay_floor_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.capability = 'lease-withdrawal-v1'
       AND EXISTS (SELECT 1 FROM public.run_events e WHERE e.event_type = 'step.lease_withdrawn') THEN
        RAISE EXCEPTION 'lease-withdrawal-v1 cannot be removed while a journal holds step.lease_withdrawn: a build without it cannot replay the event; set SAGAT_LEASE_WITHDRAWAL=off on the engines to stop new withdrawals'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;
REVOKE ALL ON FUNCTION platform_requirements_replay_floor_guard() FROM PUBLIC;

DROP INDEX IF EXISTS run_events_lease_withdrawn_idx;

DROP FUNCTION platform_capability_census(text, text[]);
CREATE FUNCTION platform_capability_census(p_capability text, p_kinds text[])
RETURNS TABLE (out_live integer, out_lacking integer, out_examples uuid[], out_recorded boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT count(*)::integer,
           (count(*) FILTER (WHERE NOT (p_capability = ANY (s.capabilities))))::integer,
           coalesce((array_agg(s.instance_id ORDER BY s.instance_id)
                     FILTER (WHERE NOT (p_capability = ANY (s.capabilities))))[1:16], '{}'::uuid[]),
           EXISTS (SELECT 1 FROM public.platform_requirements r WHERE r.capability = p_capability)
      FROM public.serving_instances s
     WHERE s.kind = ANY (p_kinds) AND s.last_seen > clock_timestamp() - interval '30 seconds';
$$;
REVOKE ALL ON FUNCTION platform_capability_census(text, text[]) FROM PUBLIC;
