-- Feature 006 (lease withdrawal, ADR-0098; decision D006-004 of specs/006-sdk-multilanguage/research.md), first of two: the
-- registry capability `lease-withdrawal-v1` joins the closed list. Each check that `20270118000000_run_input_validation` named
-- `*_known` is replaced under the same name, in one `ALTER TABLE`, by a superset added `NOT VALID` (every stored row satisfies
-- it, so nothing is scanned and the `ACCESS EXCLUSIVE` lock is taken once, briefly).
-- `20270208000100_lease_withdrawal_validate` validates them in a transaction of its own, under `SHARE UPDATE EXCLUSIVE`, so
-- reads and writes go on while a table is scanned (as design 043 did: D043-145). The operator's census, not executable by the
-- runtime role; the replay floor of the requirement (decision D006-032): a guard that refuses to remove it while a journal holds
-- the event, and the read under a share lock that a withdrawal makes, the one function the runtime role is given. The
-- journal needs no change: `run_events.event_type` is not a closed list, and the lease columns of `steps` are cleared by the
-- check `steps_lease_consistent` the withdrawal already satisfies (a `READY` step holds no lease).

ALTER TABLE serving_instances
    DROP CONSTRAINT serving_instances_capabilities_known,
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                              'lease-withdrawal-v1']::text[]
        AND cardinality(capabilities) <= 32) NOT VALID;

ALTER TABLE platform_requirements
    DROP CONSTRAINT platform_requirements_capability_known,
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1')) NOT VALID;

ALTER TABLE platform_requirement_history
    DROP CONSTRAINT platform_requirement_history_capability_known,
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1')) NOT VALID;

-- The census of `control-plane capability-activate`: the live processes (seen within 30 seconds, as the activation counts
-- them) of the kinds asked, those of them that do not list the capability with up to sixteen of their ids, and whether the
-- requirement is recorded. Read through the owner's function, as the handover's census is: the runtime role reads the
-- registry by column only and may not run it.
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

-- The requirement is a replay floor (decision D006-032): a build without this feature cannot replay `step.lease_withdrawn`, so
-- once a journal holds one the requirement that keeps such builds from serving is never removed. A new durable event is
-- forward-only; `SAGAT_LEASE_WITHDRAWAL=off` on an engine stops new withdrawals instead. The scan of `run_events` by event type
-- runs only when an operator removes this requirement.
CREATE FUNCTION platform_requirements_replay_floor_guard() RETURNS trigger
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
CREATE TRIGGER platform_requirements_replay_floor_guard BEFORE DELETE ON platform_requirements
    FOR EACH ROW EXECUTE FUNCTION platform_requirements_replay_floor_guard();
ALTER TABLE platform_requirements ENABLE ALWAYS TRIGGER platform_requirements_replay_floor_guard;

-- Whether `lease-withdrawal-v1` is a recorded requirement, read under the share lock of its row. Every withdrawal transaction
-- calls it before it writes: a removal deletes the row, so it waits for the withdrawal and its guard then sees the event; a
-- removal that committed first leaves no row, and the withdrawal writes nothing. The runtime role may run it; it reads one row
-- and changes nothing.
CREATE FUNCTION lease_withdrawal_required() RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM 1 FROM public.platform_requirements r WHERE r.capability = 'lease-withdrawal-v1' FOR SHARE;
    RETURN FOUND;
END;
$$;
REVOKE ALL ON FUNCTION platform_requirements_replay_floor_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION lease_withdrawal_required() FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION lease_withdrawal_required() TO sagat_app;
    END IF;
END $grants$;
