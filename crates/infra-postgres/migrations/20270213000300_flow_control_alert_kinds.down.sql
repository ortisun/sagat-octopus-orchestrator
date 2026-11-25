-- Rollback is an operator/quiesced-service operation. An alert of a flow-control kind is the record of a condition an
-- operator was told about (and its subscribers were sent): rolling back never destroys it, and it cannot be kept under the
-- list of kinds this restores. Delete the resolved ones after reading them, or keep a compatible reader.
LOCK TABLE operational_alerts IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM operational_alerts
                WHERE kind IN ('CONCURRENCY_QUEUE_STUCK', 'RATE_LIMIT_GRANT_LAGGING', 'RATE_LIMIT_QUEUE_SATURATED',
                               'FLOW_CONTROL_WAITER_ORPHANED')) THEN
        RAISE EXCEPTION 'flow-control alerts are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE operational_alerts DROP CONSTRAINT operational_alerts_kind_valid;
ALTER TABLE operational_alerts ADD CONSTRAINT operational_alerts_kind_valid CHECK (kind IN
    ('OUTBOX_STUCK', 'LEASE_NOT_REAPED', 'STEP_UNCLAIMED', 'REVIEW_PENDING', 'DEAD_ITEMS'));
