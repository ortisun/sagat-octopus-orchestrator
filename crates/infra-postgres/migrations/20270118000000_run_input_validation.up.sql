-- Design 043 (run input validation, ADR-0090), first of two: the action outcome `INPUT_INVALID` and the registry capability
-- `run-input-validation-v1` are added to new CHECKs `NOT VALID`, beside the old ones. Adding them takes the table's
-- `ACCESS EXCLUSIVE` lock briefly and scans nothing; `20270118000100_run_input_validation_validate` validates them in a
-- transaction of its own, under a lock that lets reads and writes go on, and drops the old ones (D043-031, D043-145). No
-- function, grant or role changes (NFR-043-SEC).

ALTER TABLE schedule_actions
    ADD CONSTRAINT schedule_actions_outcome_known CHECK (outcome IN
        ('STARTED', 'BUFFERED', 'SKIPPED_OVERLAP', 'DROPPED_BUFFER_FULL', 'DROPPED_DELETED', 'COALESCED',
         'MISSED_CATCHUP_WINDOW', 'ALREADY_ACTED', 'AUTHORITY_REVOKED', 'BUSINESS_ID_CONFLICT', 'INPUT_INVALID')) NOT VALID;

ALTER TABLE serving_instances
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1']::text[]
        AND cardinality(capabilities) <= 32) NOT VALID;

ALTER TABLE platform_requirements
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1')) NOT VALID;

ALTER TABLE platform_requirement_history
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1')) NOT VALID;
