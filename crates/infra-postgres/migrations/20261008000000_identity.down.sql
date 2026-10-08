-- Rollback of 20261008000000_identity.up.sql, in reverse dependency order.
DROP TRIGGER IF EXISTS audit_records_no_truncate ON audit_records;
DROP TRIGGER IF EXISTS audit_records_append_only ON audit_records;
DROP FUNCTION IF EXISTS audit_records_reject_mutation();
DROP TABLE IF EXISTS audit_records;
DROP TABLE IF EXISTS memberships;
DROP TABLE IF EXISTS namespaces;
DROP TABLE IF EXISTS workspaces;
DROP TABLE IF EXISTS tenants;
