DO $rollback$
BEGIN
    IF EXISTS (SELECT 1 FROM iteration_scopes)
    THEN
        RAISE EXCEPTION 'collection progress is retained; keep a compatible reader until affected runs drain'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$rollback$;

ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR')) NOT VALID;

DROP TABLE iteration_checkpoints;
DROP INDEX collection_iterations_active_idx;
DROP TABLE collection_iterations;
DROP TABLE collection_scope_items;
DROP TABLE iteration_scopes;

DROP TRIGGER tenants_iteration_usage ON tenants;
DROP FUNCTION provision_tenant_iteration_usage();
DROP TABLE tenant_iteration_usage;
DROP TABLE tenant_collection_policies;

DROP TRIGGER steps_root_instance_admission ON steps;
DROP FUNCTION account_root_instance_admission();

DROP TRIGGER runs_instance_usage ON runs;
DROP FUNCTION provision_run_instance_usage();
DROP TABLE run_instance_usage;
