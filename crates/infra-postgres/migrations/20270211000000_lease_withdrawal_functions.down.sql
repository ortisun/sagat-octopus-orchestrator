-- Rollback of `20270211000000_lease_withdrawal_functions`: the two functions get back the bodies of
-- `20270208000200_lease_withdrawal_floor` (the guard, run as the deleting role) and `20270208000000_lease_withdrawal` (the
-- requirement whatever its kinds). Nothing is stored that an older binary cannot read, so nothing is refused.
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

CREATE OR REPLACE FUNCTION lease_withdrawal_required() RETURNS boolean
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    PERFORM 1 FROM public.platform_requirements r WHERE r.capability = 'lease-withdrawal-v1' FOR SHARE;
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
