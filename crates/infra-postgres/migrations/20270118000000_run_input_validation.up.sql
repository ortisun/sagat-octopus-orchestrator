-- Design 043 (run input validation, ADR-0090), first of two: the action outcome `INPUT_INVALID` and the registry capability
-- `run-input-validation-v1`. Each table gets, in one `ALTER TABLE`, a new CHECK added `NOT VALID` (a superset of the old
-- one, so every stored row satisfies it) and the old CHECK dropped: the `ACCESS EXCLUSIVE` lock is taken once, briefly, and
-- nothing is scanned. `20270118000100_run_input_validation_validate` validates the new CHECKs in a transaction of its own,
-- under `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while a table is scanned (D043-031, D043-145, D043-153). No
-- function, grant or role changes (NFR-043-SEC).

ALTER TABLE schedule_actions
    ADD CONSTRAINT schedule_actions_outcome_known CHECK (outcome IN
        ('STARTED', 'BUFFERED', 'SKIPPED_OVERLAP', 'DROPPED_BUFFER_FULL', 'DROPPED_DELETED', 'COALESCED',
         'MISSED_CATCHUP_WINDOW', 'ALREADY_ACTED', 'AUTHORITY_REVOKED', 'BUSINESS_ID_CONFLICT', 'INPUT_INVALID')) NOT VALID,
    DROP CONSTRAINT schedule_actions_outcome_valid;

ALTER TABLE serving_instances
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1']::text[]
        AND cardinality(capabilities) <= 32) NOT VALID,
    DROP CONSTRAINT serving_instances_capabilities_closed;

ALTER TABLE platform_requirements
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1')) NOT VALID,
    DROP CONSTRAINT platform_requirements_capability_valid;

ALTER TABLE platform_requirement_history
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1')) NOT VALID,
    DROP CONSTRAINT platform_requirement_history_capability_valid;
