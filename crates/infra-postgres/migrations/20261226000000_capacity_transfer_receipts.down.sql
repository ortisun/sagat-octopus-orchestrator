-- Rollback is an operator/quiesced-service operation, not an online caller protocol.
-- Drop the step index before locking receipts: native Vacate reads steps before inserting a receipt.
-- SQLx keeps the drop, retained-data guard and table removal in one migration transaction.
DROP INDEX steps_run_capacity_lease_idx;
LOCK TABLE capacity_group_transfer_receipts IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM capacity_group_transfer_receipts) THEN
        RAISE EXCEPTION 'capacity transfer receipts are retained; keep a compatible reader'
            USING ERRCODE='55000';
    END IF;
END $guard$;
DROP TABLE capacity_group_transfer_receipts;
