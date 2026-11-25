-- The merge of features 022 and 041 (decision D022-089), second of two: the CHECKs 20270215000000 restated `NOT VALID`
-- are validated, each under `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while a table is scanned. Nothing else
-- is in this transaction (design 043, D043-145, D043-153).
ALTER TABLE quota_ceilings VALIDATE CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_limits VALIDATE CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits VALIDATE CONSTRAINT quota_limits_tenant_scoped;
