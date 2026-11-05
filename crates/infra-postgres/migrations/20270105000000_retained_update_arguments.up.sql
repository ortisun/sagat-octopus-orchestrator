-- Retained arguments of a declared Update (feature 018, FR-018-039, T018-043). An Update whose declaration
-- opts in with `reapplyOnReset` keeps its canonical arguments as content of the tenant (a `payloads` row, so
-- charged to `tenant_content_bytes` by the trigger that charges every payload), and the receipt of the Update
-- points at them, so a reset can validate and apply them again. The journal row `state.updated` already
-- references the new state through its own `payload_digest`; the arguments are referenced from the receipt,
-- which goes with the run (`ON DELETE CASCADE`) and is written in the same transaction as that row.
ALTER TABLE operation_update_receipts ADD COLUMN arguments_digest bytea;

-- NOT VALID first, then validated: no row has the column set yet.
ALTER TABLE operation_update_receipts
    ADD CONSTRAINT operation_update_receipts_arguments_fk
        FOREIGN KEY (tenant_id, arguments_digest) REFERENCES payloads (tenant_id, digest) NOT VALID;
ALTER TABLE operation_update_receipts VALIDATE CONSTRAINT operation_update_receipts_arguments_fk;

-- Arguments are kept for an Update that was applied, never for one a validator refused, and are a SHA-256.
ALTER TABLE operation_update_receipts
    ADD CONSTRAINT operation_update_receipts_arguments CHECK (
        arguments_digest IS NULL
        OR (status = 'APPLIED' AND octet_length(arguments_digest) = 32)) NOT VALID;
ALTER TABLE operation_update_receipts VALIDATE CONSTRAINT operation_update_receipts_arguments;
