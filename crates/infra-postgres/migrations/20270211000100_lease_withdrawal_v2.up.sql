-- Feature 006 (lease withdrawal, ADR-0098), the follow-up of pull request #260, first of two (request (9) of decision D006-059
-- of specs/006-sdk-multilanguage/research.md; Task 006-071; decision D006-096): the registry capability `lease-withdrawal-v2`
-- joins the closed list. It means that the binary replays every cause of `step.lease_withdrawn`, `NOT_ACKNOWLEDGED` of
-- protocol 1.8 included, and that an engine whose registry has been silent serves no capability call (request (7)). Each check
-- that `20270208000000_lease_withdrawal` named `*_known` is replaced under the same name, in one `ALTER TABLE`, by a superset
-- added `NOT VALID` (every stored row satisfies it, so nothing is scanned and the `ACCESS EXCLUSIVE` lock is taken once,
-- briefly); `20270211000200_lease_withdrawal_v2_validate` validates them in a transaction of its own.
--
-- The requirement is a replay floor for the one cause a build without it cannot replay: the guard of `platform_requirements`
-- refuses to remove `lease-withdrawal-v2` while a journal holds `step.lease_withdrawn` with the cause `NOT_ACKNOWLEDGED`, and,
-- as for `lease-withdrawal-v1` (decision D006-084), refuses the removal outside READ COMMITTED. So recording it stays
-- reversible until the first withdrawal of protocol 1.8. Its look at the journal is a probe of a partial index that holds
-- only those rows, so the removal stays within its bound under the epoch lock however many other withdrawals the journal
-- holds (decision D006-085). The index is built here with `IF NOT EXISTS`, so an operator can build it `CONCURRENTLY` first
-- (docs/runbooks/lease-withdrawal.md, "Before upgrading"); an invalid index left by a failed concurrent build is refused.

ALTER TABLE serving_instances
    DROP CONSTRAINT serving_instances_capabilities_known,
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                              'lease-withdrawal-v1', 'lease-withdrawal-v2']::text[]
        AND cardinality(capabilities) <= 32) NOT VALID;

ALTER TABLE platform_requirements
    DROP CONSTRAINT platform_requirements_capability_known,
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2')) NOT VALID;

ALTER TABLE platform_requirement_history
    DROP CONSTRAINT platform_requirement_history_capability_known,
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1',
                       'lease-withdrawal-v1', 'lease-withdrawal-v2')) NOT VALID;

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
