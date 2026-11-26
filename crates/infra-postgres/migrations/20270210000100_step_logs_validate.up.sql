-- Feature 041, second of two: the CHECK 20270210000000_step_logs added `NOT VALID` to an existing table is validated
-- under `SHARE UPDATE EXCLUSIVE`, so reads and writes go on while the table is scanned. Nothing else is in this
-- transaction (design 043, D043-145, D043-153).

ALTER TABLE tenant_retained_usage VALIDATE CONSTRAINT tenant_retained_usage_step_log_leased_nonnegative;
