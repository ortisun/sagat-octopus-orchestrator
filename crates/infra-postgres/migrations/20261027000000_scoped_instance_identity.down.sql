-- Older code cannot address scoped rows. Refuse a destructive rollback once any writer has used the
-- new identity; operators must first drain compatible runs and remove their retained scoped records.
DO $rollback$
BEGIN
    IF EXISTS (
        SELECT 1 FROM steps
        WHERE cardinality(instance_path_nodes) > 0 OR cardinality(instance_path_indices) > 0)
       OR EXISTS (SELECT 1 FROM run_events WHERE instance_id IS NOT NULL)
       OR EXISTS (SELECT 1 FROM event_deliveries WHERE instance_id IS NOT NULL)
       OR EXISTS (SELECT 1 FROM run_sagas WHERE instance_id IS NOT NULL)
       OR EXISTS (SELECT 1 FROM script_executions WHERE instance_id IS NOT NULL)
    THEN
        RAISE EXCEPTION 'scoped execution identity is in use; keep a compatible reader until retained rows drain'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END
$rollback$;

DROP INDEX script_executions_instance_idx;
ALTER TABLE script_executions
    DROP CONSTRAINT script_executions_instance_identity_consistent,
    DROP COLUMN instance_path_indices,
    DROP COLUMN instance_path_nodes,
    DROP COLUMN instance_id;

DROP INDEX run_sagas_instance_idx;
ALTER TABLE run_sagas
    DROP CONSTRAINT run_sagas_instance_identity_consistent,
    DROP COLUMN instance_path_indices,
    DROP COLUMN instance_path_nodes,
    DROP COLUMN instance_id;

DROP INDEX event_deliveries_instance_idx;
ALTER TABLE event_deliveries
    DROP CONSTRAINT event_deliveries_instance_identity_consistent,
    DROP COLUMN instance_path_indices,
    DROP COLUMN instance_path_nodes,
    DROP COLUMN instance_id;

DROP INDEX run_events_instance_idx;
ALTER TABLE run_events
    DROP CONSTRAINT run_events_instance_identity_consistent,
    DROP COLUMN instance_path_indices,
    DROP COLUMN instance_path_nodes,
    DROP COLUMN instance_id;

ALTER TABLE steps DROP CONSTRAINT steps_pkey;
ALTER TABLE steps ADD CONSTRAINT steps_pkey PRIMARY KEY (tenant_id, run_id, node);
ALTER TABLE steps
    DROP CONSTRAINT steps_instance_path_valid,
    DROP COLUMN instance_path_indices,
    DROP COLUMN instance_path_nodes;

DROP FUNCTION execution_instance_path_valid(text[], integer[]);
