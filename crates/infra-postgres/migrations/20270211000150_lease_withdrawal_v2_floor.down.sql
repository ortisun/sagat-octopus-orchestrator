-- Rollback of `20270211000150_lease_withdrawal_v2_floor`: refused while a journal holds `step.lease_withdrawn` with the cause
-- `NOT_ACKNOWLEDGED` (the guard it restores no longer keeps `lease-withdrawal-v2` once one exists); the journal is held in SHARE
-- mode while it is read. Otherwise the guard is the one of `20270211000000_lease_withdrawal_functions` and the index goes.
LOCK TABLE run_events IN SHARE MODE;
DO $guard$
BEGIN
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
