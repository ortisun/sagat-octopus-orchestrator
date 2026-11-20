-- Rollback is an operator/quiesced-service operation. A run whose start waits for, holds or held a flow-control gate is
-- state an older reader cannot load: rolling back would lose it, so the rollback refuses while one exists.
LOCK TABLE runs IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM runs WHERE flow_control_gates IS NOT NULL) THEN
        RAISE EXCEPTION 'a run''s start holds, waits for or held a flow-control gate; drain and archive the gated runs before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP FUNCTION run_active_counts_lock_slot(uuid, uuid);
ALTER TABLE runs DROP CONSTRAINT runs_flow_control_gates_bounded;
ALTER TABLE runs DROP COLUMN flow_control_gates;
