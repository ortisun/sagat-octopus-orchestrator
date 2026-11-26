-- Feature 006 (lease withdrawal, ADR-0098), third: the replay floor of `lease-withdrawal-v1` made explicit about the isolation
-- level and bounded (decisions D006-084 and D006-085 of specs/006-sdk-multilanguage/research.md).
--
-- 1. A partial index holds only the rows of `step.lease_withdrawn`, so the floor's look at the journal is an index probe
--    (the down of this migration drops it first, so the offline check of `20270208000000`'s down scans), not a scan of `run_events` under the epoch lock; a journal write pays for it only when
--    it is a withdrawal. The build reads `run_events` once under a SHARE lock, so journal writes wait for it; `IF NOT EXISTS`
--    lets an operator build it CONCURRENTLY first on a large database (docs/runbooks/lease-withdrawal.md, "Before the
--    activation"); an invalid index left by a failed concurrent build is refused (the pattern of
--    `20270119000000_batch_purge`).
-- 2. The guard refuses the removal outside READ COMMITTED: only there does the statement that looks at the journal take a new
--    snapshot after the row lock is granted, and so see a withdrawal that committed while the removal waited.
-- 3. The removal waits at most one second for each lock; the runbook runs it under a statement timeout of 1.5 s, which bounds
--    the whole call, so a registration that queues behind its epoch lock (on the shared pool's 5 s statement timeout) is
--    never held past it (decision D006-085).
-- 4. The census of `control-plane capability-activate` also says which of the kinds asked have a live process, for an entry
--    that requires one of each (decision D006-082).
CREATE INDEX IF NOT EXISTS run_events_lease_withdrawn_idx ON run_events (tenant_id)
    WHERE event_type = 'step.lease_withdrawn';
DO $valid$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
                WHERE NOT i.indisvalid AND c.relname = 'run_events_lease_withdrawn_idx') THEN
        RAISE EXCEPTION 'invalid index left by a failed concurrent build, drop it and run again: run_events_lease_withdrawn_idx';
    END IF;
END
$valid$;

CREATE OR REPLACE FUNCTION platform_requirements_replay_floor_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.capability = 'lease-withdrawal-v1' THEN
        IF current_setting('transaction_isolation') <> 'read committed' THEN
            RAISE EXCEPTION 'lease-withdrawal-v1 is removed only under READ COMMITTED (this transaction runs under %): a stricter level keeps a snapshot that cannot see a withdrawal committed meanwhile', current_setting('transaction_isolation')
                USING ERRCODE = 'restrict_violation';
        END IF;
        IF EXISTS (SELECT 1 FROM public.run_events e WHERE e.event_type = 'step.lease_withdrawn') THEN
            RAISE EXCEPTION 'lease-withdrawal-v1 cannot be removed while a journal holds step.lease_withdrawn: a build without it cannot replay the event; set SAGAT_LEASE_WITHDRAWAL=off on the engines to stop new withdrawals'
                USING ERRCODE = 'restrict_violation';
        END IF;
    END IF;
    RETURN OLD;
END;
$$;
REVOKE ALL ON FUNCTION platform_requirements_replay_floor_guard() FROM PUBLIC;

ALTER FUNCTION platform_requirement_remove(text, text) SET lock_timeout = '1s';

DROP FUNCTION platform_capability_census(text, text[]);
CREATE FUNCTION platform_capability_census(p_capability text, p_kinds text[])
RETURNS TABLE (out_live integer, out_lacking integer, out_examples uuid[], out_recorded boolean, out_kinds_live text[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT count(*)::integer,
           (count(*) FILTER (WHERE NOT (p_capability = ANY (s.capabilities))))::integer,
           coalesce((array_agg(s.instance_id ORDER BY s.instance_id)
                     FILTER (WHERE NOT (p_capability = ANY (s.capabilities))))[1:16], '{}'::uuid[]),
           EXISTS (SELECT 1 FROM public.platform_requirements r WHERE r.capability = p_capability),
           coalesce(array_agg(DISTINCT s.kind ORDER BY s.kind), '{}'::text[])
      FROM public.serving_instances s
     WHERE s.kind = ANY (p_kinds) AND s.last_seen > clock_timestamp() - interval '30 seconds';
$$;
REVOKE ALL ON FUNCTION platform_capability_census(text, text[]) FROM PUBLIC;
