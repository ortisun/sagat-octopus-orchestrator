-- Rollback is an operator/quiesced-service operation. A held signal is a request a caller was answered `202` for: rolling
-- back never drops it. Let the queued runs that hold one be granted (the grant delivers it) or end (their end drops it,
-- audited) first.
LOCK TABLE flow_control_held_signals IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM flow_control_held_signals) THEN
        RAISE EXCEPTION 'held Signal-with-Start signals remain; let their queued runs be granted or end first'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

DROP TABLE flow_control_held_signals;
