DO $revoke$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON command_dedupe, workflow_versions, workflows FROM sagat_app;
    END IF;
END
$revoke$;

DROP TABLE command_dedupe;
ALTER TABLE workflows DROP CONSTRAINT workflows_active_version_same_tenant_fk;
DROP TRIGGER workflow_versions_no_truncate ON workflow_versions;
DROP TRIGGER workflow_versions_immutable ON workflow_versions;
DROP TABLE workflow_versions;
DROP FUNCTION workflow_versions_reject_mutation();
DROP TABLE workflows;

-- Workers cannot be members any more; refuse to downgrade while any exists.
ALTER TABLE memberships DROP CONSTRAINT memberships_role_valid;
ALTER TABLE memberships
    ADD CONSTRAINT memberships_role_valid
    CHECK (role IN ('admin', 'developer', 'operator', 'viewer'));
