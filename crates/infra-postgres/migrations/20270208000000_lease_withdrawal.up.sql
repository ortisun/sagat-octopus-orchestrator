-- Feature 006 (lease withdrawal, ADR-0098; decision D006-004 of specs/006-sdk-multilanguage/research.md), first of two: the
-- registry capability `lease-withdrawal-v1` joins the closed list. Each check that `20270118000000_run_input_validation` named
-- `*_known` is replaced under the same name, in one `ALTER TABLE`, by a superset added `NOT VALID` (every stored row satisfies
-- it, so nothing is scanned and the `ACCESS EXCLUSIVE` lock is taken once, briefly).
-- `20270208000100_lease_withdrawal_validate` validates them in a transaction of its own, under `SHARE UPDATE EXCLUSIVE`, so
-- reads and writes go on while a table is scanned (as design 043 did: D043-145). No function, grant or role changes. The
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
