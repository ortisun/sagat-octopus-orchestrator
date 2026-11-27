-- Feature 006 (lease withdrawal, ADR-0098), the follow-up of pull request #260 (requests (6) and (8) of decision D006-059 of
-- specs/006-sdk-multilanguage/research.md; Task 006-070; decision D006-095). Migration `20270208000000_lease_withdrawal` may
-- have been applied anywhere, so its two functions are rewritten here, in place, with `CREATE OR REPLACE FUNCTION` (the grants
-- are kept by the replacement, and restated):
--
-- 1. The replay-floor guard says that `SAGAT_LEASE_WITHDRAWAL=off` stops new withdrawals of unmarked leases: from protocol 1.8
--    the switch does not reach a lease whose acknowledgement mark is written (ADR-0099 decision 10), so "new withdrawals" alone
--    would promise more than the switch does. The guard now runs as its owner with the fixed search path, as every other
--    function of the registry does: it reads `run_events` whoever deletes the row.
-- 2. `lease_withdrawal_required()` answers yes only when `lease-withdrawal-v1` is recorded for both kinds, control planes and
--    engines: the gate of an engine asks the same question (`ServingGate::requires` over both kinds), and a requirement
--    recorded for one kind by hand SQL leaves the other kind's binaries free to serve without replaying the event.
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

-- Whether `lease-withdrawal-v1` is recorded for control planes and engines, read under the share lock of its row (decision
-- D006-032): a removal deletes the row, so it waits for the withdrawal and its guard then sees the event. A row recorded for one
-- kind only is not the requirement: nothing is withdrawn under it.
CREATE OR REPLACE FUNCTION lease_withdrawal_required() RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM 1 FROM public.platform_requirements r
     WHERE r.capability = 'lease-withdrawal-v1'
       AND r.kinds @> ARRAY['CONTROL_PLANE', 'EXECUTION_ENGINE']::text[]
       FOR SHARE;
    RETURN FOUND;
END;
$$;
REVOKE ALL ON FUNCTION lease_withdrawal_required() FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT EXECUTE ON FUNCTION lease_withdrawal_required() TO sagat_app;
    END IF;
END $grants$;
