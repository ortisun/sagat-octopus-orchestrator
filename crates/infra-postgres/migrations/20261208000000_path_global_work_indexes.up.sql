-- Bounded path status needs existential facts about other open work, never historical steps.
CREATE INDEX steps_path_live_idx ON steps
    (tenant_id, run_id, instance_path_nodes, instance_path_indices)
    WHERE status IN ('READY','LEASED','RUNNING','RETRY_WAIT');
CREATE INDEX steps_path_review_idx ON steps
    (tenant_id, run_id, instance_path_nodes, instance_path_indices)
    WHERE status='WAITING' AND waiting_kind='OPERATOR';
CREATE INDEX run_sagas_compensating_idx ON run_sagas (tenant_id, run_id)
    WHERE phase IN ('ABORTING','COMPENSATING');
-- steps_run_open_idx, steps_run_waiting_idx and scope_sagas_open_idx already cover the other facts.
