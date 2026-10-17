ALTER TABLE steps DROP CONSTRAINT steps_approval_consistent;
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION', 'LOOP')) NOT VALID;
ALTER TABLE steps DROP COLUMN approval_request_id;

DROP TABLE approval_timers;
DROP TRIGGER approval_decision_receipts_no_truncate ON approval_decision_receipts;
DROP TRIGGER approval_decision_receipts_immutable ON approval_decision_receipts;
DROP TABLE approval_decision_receipts;
DROP TABLE approval_requests;
DROP FUNCTION approval_policy_snapshot_valid(text[], text[]);
