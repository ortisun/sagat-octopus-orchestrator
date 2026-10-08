-- Least privilege for the process that serves requests, and a trail that survives a bypassed role.
--
-- The runtime role `sagat_app` is created by whoever provisions the database (it is a cluster-level
-- object with a password, so it does not belong in a migration); this migration grants it exactly
-- what the control plane needs and nothing else:
--   * read the tenants and memberships that operators provision,
--   * create and read workspaces and namespaces,
--   * append to and read the audit trail.
-- It gets no UPDATE, DELETE or TRUNCATE on any table, no DDL, and does not own anything, so it can
-- neither rewrite the audit trail nor disable the triggers that protect it. Schema changes, tenant
-- provisioning and the development seed run as the owning role instead.
--
-- If the role does not exist when this runs, the grants are skipped; create the role first and
-- re-create the grants with `GRANT` statements equivalent to the ones below.
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT USAGE ON SCHEMA public TO sagat_app;
        GRANT SELECT ON tenants, memberships TO sagat_app;
        GRANT SELECT, INSERT ON workspaces, namespaces, audit_records TO sagat_app;
    END IF;
END
$grants$;

-- Fire the audit protections even in a session that sets `session_replication_role = replica`,
-- the usual way to bypass triggers (it needs a privileged role, which the runtime role is not).
ALTER TABLE audit_records ENABLE ALWAYS TRIGGER audit_records_append_only;
ALTER TABLE audit_records ENABLE ALWAYS TRIGGER audit_records_no_truncate;
