-- Feature 006 (lease withdrawal, ADR-0098), the follow-up of pull request #260, first of three (request (9) of decision D006-059
-- of specs/006-sdk-multilanguage/research.md; Task 006-071; decision D006-096): the registry capability `lease-withdrawal-v2`
-- joins the closed list. It means that the binary replays every cause of `step.lease_withdrawn`, `NOT_ACKNOWLEDGED` of
-- protocol 1.8 included, and that an engine whose registry has been silent serves no capability call (request (7)). Each check
-- that `20270208000000_lease_withdrawal` named `*_known` is replaced under the same name, in one `ALTER TABLE` per table, by a
-- superset added `NOT VALID`: every stored row satisfies it, so nothing is scanned, and each `ACCESS EXCLUSIVE` lock lasts from
-- its `ALTER TABLE` to the commit of this migration, which does nothing else (a heartbeat waits for it for no longer than the
-- three catalog changes take). The replay floor of the capability, which builds an index over `run_events`, is the next
-- migration, `20270211000150_lease_withdrawal_v2_floor`, so no registry table is held while the journal is read (review of
-- 2026-11-25, decision D006-100); `20270211000200_lease_withdrawal_v2_validate` validates the checks in a transaction of its own.

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
