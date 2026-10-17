-- G21 (owner decision 2026-10-16): a run's result is the output of the `end` step that finished
-- it, evaluated from `outputExpr` in the transaction that reaches `end`. The digest names that
-- step's payload, so the parent of a child run can read it without knowing the child's graph.
-- Disclosure through the API waits for ADR-0033; nothing here exposes it.
ALTER TABLE runs ADD COLUMN result_digest bytea
    CONSTRAINT runs_result_digest_size CHECK (result_digest IS NULL OR octet_length(result_digest) = 32);
ALTER TABLE runs ADD CONSTRAINT runs_result_same_tenant_fk
    FOREIGN KEY (tenant_id, result_digest) REFERENCES payloads (tenant_id, digest);
