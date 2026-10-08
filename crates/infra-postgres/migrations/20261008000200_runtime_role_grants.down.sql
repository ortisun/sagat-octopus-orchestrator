ALTER TABLE audit_records ENABLE TRIGGER audit_records_no_truncate;
ALTER TABLE audit_records ENABLE TRIGGER audit_records_append_only;

DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON tenants, memberships, workspaces, namespaces, audit_records FROM sagat_app;
        REVOKE USAGE ON SCHEMA public FROM sagat_app;
    END IF;
END
$revoke$;
