-- Feature 006 (lease withdrawal, ADR-0098), the follow-up of pull request #260, second of three (decision D006-096, split out
-- by decision D006-100): the replay floor of `lease-withdrawal-v2`. It touches no registry table, so the build of its index,
-- which reads `run_events` under a SHARE lock, never holds a heartbeat or a registration.
--
-- The guard of `platform_requirements` refuses to remove `lease-withdrawal-v2` while a journal holds `step.lease_withdrawn`
-- with the cause `NOT_ACKNOWLEDGED`, and, as for `lease-withdrawal-v1` (decision D006-084), refuses the removal outside READ
-- COMMITTED. So recording it stays reversible until the first withdrawal of protocol 1.8. Its look at the journal is a probe of a
-- partial index that holds only those rows, so the removal stays within its bound under the epoch lock however many other
-- withdrawals the journal holds (decision D006-085). The index is built with `IF NOT EXISTS`, so an operator can build it
-- `CONCURRENTLY` first (docs/runbooks/lease-withdrawal.md); an invalid index left by a failed concurrent build is refused.
CREATE INDEX IF NOT EXISTS run_events_lease_not_acknowledged_idx ON run_events (tenant_id)
    WHERE event_type = 'step.lease_withdrawn' AND detail->>'cause' = 'NOT_ACKNOWLEDGED';
DO $valid$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
                WHERE NOT i.indisvalid AND c.relname = 'run_events_lease_not_acknowledged_idx') THEN
        RAISE EXCEPTION 'invalid index left by a failed concurrent build, drop it and run again: run_events_lease_not_acknowledged_idx';
    END IF;
END
$valid$;

CREATE OR REPLACE FUNCTION platform_requirements_replay_floor_guard() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF OLD.capability IN ('lease-withdrawal-v1', 'lease-withdrawal-v2')
       AND current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION '% is removed only under READ COMMITTED (this transaction runs under %): a stricter level keeps a snapshot that cannot see a withdrawal committed meanwhile', OLD.capability, current_setting('transaction_isolation')
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.capability = 'lease-withdrawal-v1'
       AND EXISTS (SELECT 1 FROM public.run_events e WHERE e.event_type = 'step.lease_withdrawn') THEN
        RAISE EXCEPTION 'lease-withdrawal-v1 cannot be removed while a journal holds step.lease_withdrawn: a build without it cannot replay the event; set SAGAT_LEASE_WITHDRAWAL=off on the engines to stop new withdrawals of unmarked leases'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.capability = 'lease-withdrawal-v2'
       AND EXISTS (SELECT 1 FROM public.run_events e
                    WHERE e.event_type = 'step.lease_withdrawn' AND e.detail->>'cause' = 'NOT_ACKNOWLEDGED') THEN
        RAISE EXCEPTION 'lease-withdrawal-v2 cannot be removed while a journal holds step.lease_withdrawn with the cause NOT_ACKNOWLEDGED: a build without it cannot replay that cause'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;
REVOKE ALL ON FUNCTION platform_requirements_replay_floor_guard() FROM PUBLIC;
