-- Rollback is an operator/quiesced-service operation. A step that waits for, or holds, a flow-control gate is state an older
-- reader cannot load: rolling back would lose it, so the rollback refuses while one exists.
LOCK TABLE steps IN SHARE ROW EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM steps WHERE flow_control_gates IS NOT NULL OR waiting_kind IN ('CONCURRENCY', 'RATE_LIMIT')) THEN
        RAISE EXCEPTION 'a step holds or waits for a flow-control gate; drain the gated runs before rolling back'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE steps DROP CONSTRAINT steps_gate_wait_projected;
ALTER TABLE steps DROP CONSTRAINT steps_flow_control_gates_bounded;
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION', 'LOOP', 'APPROVAL')) NOT VALID;
ALTER TABLE steps DROP COLUMN flow_control_gates;
