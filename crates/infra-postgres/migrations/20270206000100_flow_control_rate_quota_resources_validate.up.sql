-- Feature 022 (T022-077): the CHECKs 20270206000000 restated `NOT VALID` are validated, each under `SHARE UPDATE
-- EXCLUSIVE`, so reads and writes go on while a table is scanned (the rule of design 043, D043-145, D043-153).
ALTER TABLE quota_ceilings VALIDATE CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_limits VALIDATE CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits VALIDATE CONSTRAINT quota_limits_tenant_scoped;
