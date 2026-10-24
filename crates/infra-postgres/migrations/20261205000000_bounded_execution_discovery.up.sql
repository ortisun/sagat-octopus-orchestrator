-- Runtime discovery uses immutable row metadata and exact instance coordinates. It does not parse
-- workflow documents or scan sibling histories to decide what kind of work a row represents.

ALTER TABLE steps ADD COLUMN node_kind text NOT NULL DEFAULT 'legacyControl';
ALTER TABLE steps ADD CONSTRAINT steps_node_kind_valid CHECK (node_kind IN (
    'activity', 'timer', 'waitEvent', 'condition', 'switch', 'approval', 'parallel', 'child',
    'saga', 'forEach', 'while', 'repeatUntil', 'return', 'continueAsNew', 'end',
    'legacyControl')) NOT VALID;

-- Build indexes before the backfill queues deferred steps_quota_flush trigger events. PostgreSQL
-- refuses CREATE INDEX on a table with pending trigger events in this same transaction. Keep the
-- normal deferred quota flush and the entire migration atomic; no trigger is disabled.
CREATE INDEX steps_path_actionable_idx
    ON steps (tenant_id, run_id, instance_path_nodes, instance_path_indices, status, node)
    WHERE (status='PENDING' OR (status IN ('SUCCEEDED','FAILED','SKIPPED') AND NOT routed));
CREATE INDEX iteration_scopes_open_owner_idx
    ON iteration_scopes (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node)
    WHERE lifecycle IN ('OPEN','DRAINING');
CREATE INDEX collection_iterations_actionable_idx
    ON collection_iterations (tenant_id, run_id, scope_id, lifecycle, iteration_index)
    WHERE lifecycle IN ('ADMITTED','CHECKPOINTED');
CREATE INDEX loop_scopes_actionable_idx
    ON loop_scopes (tenant_id, run_id, lifecycle, owner_path_nodes, owner_path_indices, owner_node)
    WHERE lifecycle IN ('READY','RUNNING','DELAYING','DRAINING');

UPDATE steps step SET node_kind = CASE
    WHEN step.activity IS NOT NULL THEN 'activity'
    WHEN step.waiting_kind = 'TIMER' THEN 'timer'
    WHEN step.waiting_kind = 'EVENT' THEN 'waitEvent'
    WHEN step.waiting_kind = 'CHILD' THEN 'child'
    WHEN step.waiting_kind = 'APPROVAL' THEN 'approval'
    WHEN step.condition IS NOT NULL THEN 'condition'
    WHEN step.switch_case_id IS NOT NULL THEN 'switch'
    WHEN EXISTS (
        SELECT 1 FROM iteration_scopes scope
        WHERE scope.tenant_id=step.tenant_id AND scope.run_id=step.run_id
          AND scope.owner_path_nodes=step.instance_path_nodes
          AND scope.owner_path_indices=step.instance_path_indices
          AND scope.owner_node=step.node) THEN 'forEach'
    WHEN EXISTS (
        SELECT 1 FROM loop_scopes scope
        WHERE scope.tenant_id=step.tenant_id AND scope.run_id=step.run_id
          AND scope.owner_path_nodes=step.instance_path_nodes
          AND scope.owner_path_indices=step.instance_path_indices
          AND scope.owner_node=step.node AND scope.kind='WHILE') THEN 'while'
    WHEN EXISTS (
        SELECT 1 FROM loop_scopes scope
        WHERE scope.tenant_id=step.tenant_id AND scope.run_id=step.run_id
          AND scope.owner_path_nodes=step.instance_path_nodes
          AND scope.owner_path_indices=step.instance_path_indices
          AND scope.owner_node=step.node AND scope.kind='REPEAT_UNTIL') THEN 'repeatUntil'
    WHEN EXISTS (
        SELECT 1 FROM collection_iterations member
        WHERE member.tenant_id=step.tenant_id AND member.run_id=step.run_id
          AND member.return_path_nodes=step.instance_path_nodes
          AND member.return_path_indices=step.instance_path_indices
          AND member.return_node=step.node) OR EXISTS (
        SELECT 1 FROM loop_iterations member
        WHERE member.tenant_id=step.tenant_id AND member.run_id=step.run_id
          AND member.entry_path_nodes=step.instance_path_nodes
          AND member.entry_path_indices=step.instance_path_indices
          AND member.return_node=step.node) THEN 'return'
    WHEN EXISTS (
        SELECT 1 FROM run_events event
        WHERE event.tenant_id=step.tenant_id AND event.run_id=step.run_id
          AND event.instance_path_nodes=step.instance_path_nodes
          AND event.instance_path_indices=step.instance_path_indices
          AND event.node=step.node AND event.event_type='end.reached') THEN 'end'
    ELSE step.node_kind
END;
