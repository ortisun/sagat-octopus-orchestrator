-- Rollback is an operator/quiesced-service operation. An entry that expired is evidence of what happened to a request that
-- cancelled a customer's run and never made its replacement: rolling back never destroys it, and it cannot be kept under
-- the list of reasons this restores. Keep a compatible reader, or move the entry by hand after reading
-- `docs/runbooks/run-reset.md`.
LOCK TABLE run_resets IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM run_resets WHERE failure_reason = 'SOURCE_DID_NOT_END') THEN
        RAISE EXCEPTION 'resets that expired are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;

ALTER TABLE run_resets DROP CONSTRAINT run_resets_failure_reason_valid;
ALTER TABLE run_resets ADD CONSTRAINT run_resets_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
    ('BOUNDARY_NO_LONGER_ELIGIBLE', 'ADMISSION_REFUSED', 'BUSINESS_ID_IN_USE',
     'CONCURRENCY_PERMIT_UNAVAILABLE', 'RUN_PURGED',
     'AUTHORITY_REVOKED', 'REEXECUTION_NOT_ACKNOWLEDGED', 'REAPPLY_TOO_MANY'));
