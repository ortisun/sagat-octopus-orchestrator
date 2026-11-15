-- Design 043 (run input validation, ADR-0090): the action outcome `INPUT_INVALID` (a schedule action whose input the target
-- version refused: acted on, nothing started, never retried) and the registry capability `run-input-validation-v1` (listed by
-- a process only in `enforce` mode). Each widened CHECK is added under a new name `NOT VALID`, validated, and the old one is
-- dropped, so the table is not scanned under its strongest lock (D043-031). No function, grant or role changes: the runtime
-- role already inserts schedule actions and its own registry row (NFR-043-SEC).

ALTER TABLE schedule_actions
    ADD CONSTRAINT schedule_actions_outcome_known CHECK (outcome IN
        ('STARTED', 'BUFFERED', 'SKIPPED_OVERLAP', 'DROPPED_BUFFER_FULL', 'DROPPED_DELETED', 'COALESCED',
         'MISSED_CATCHUP_WINDOW', 'ALREADY_ACTED', 'AUTHORITY_REVOKED', 'BUSINESS_ID_CONFLICT', 'INPUT_INVALID')) NOT VALID;
ALTER TABLE schedule_actions VALIDATE CONSTRAINT schedule_actions_outcome_known;
ALTER TABLE schedule_actions DROP CONSTRAINT schedule_actions_outcome_valid;

ALTER TABLE serving_instances
    ADD CONSTRAINT serving_instances_capabilities_known CHECK (
        capabilities <@ ARRAY['schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1']::text[]
        AND cardinality(capabilities) <= 32) NOT VALID;
ALTER TABLE serving_instances VALIDATE CONSTRAINT serving_instances_capabilities_known;
ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_closed;

ALTER TABLE platform_requirements
    ADD CONSTRAINT platform_requirements_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1')) NOT VALID;
ALTER TABLE platform_requirements VALIDATE CONSTRAINT platform_requirements_capability_known;
ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_valid;

ALTER TABLE platform_requirement_history
    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (
        capability IN ('schedules-v1', 'access-narrowing-v1', 'payload-encoding-v1', 'run-input-validation-v1')) NOT VALID;
ALTER TABLE platform_requirement_history VALIDATE CONSTRAINT platform_requirement_history_capability_known;
ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_valid;
