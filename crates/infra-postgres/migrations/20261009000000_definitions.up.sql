-- Workflow definitions (feature 002, FR-001-002, FR-002-008): a mutable draft per workflow and
-- immutable, numbered versions. Every foreign key is tenant-scoped, so the database itself refuses a
-- workflow that points at another tenant's namespace or a version of another tenant's workflow.

-- A machine identity that executes tasks (role `worker`) joins the built-in roles.
ALTER TABLE memberships DROP CONSTRAINT memberships_role_valid;
ALTER TABLE memberships
    ADD CONSTRAINT memberships_role_valid
    CHECK (role IN ('admin', 'developer', 'operator', 'viewer', 'worker'));

CREATE TABLE workflows (
    id                uuid        PRIMARY KEY,
    tenant_id         uuid        NOT NULL REFERENCES tenants (id),
    namespace_id      uuid        NOT NULL,
    name              text        NOT NULL,
    -- Compare-and-swap token of the draft: every edit increments it in the same statement that
    -- checks it, so two editors can never silently overwrite each other.
    draft_revision    bigint      NOT NULL DEFAULT 1,
    draft             jsonb       NOT NULL,
    active_version_id uuid,
    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL,
    CONSTRAINT workflows_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT workflows_tenant_namespace_name_uniq UNIQUE (tenant_id, namespace_id, name),
    CONSTRAINT workflows_name_format CHECK (name ~ '^[a-z][a-z0-9-]{1,61}[a-z0-9]$' AND name !~ '--'),
    CONSTRAINT workflows_revision_positive CHECK (draft_revision >= 1),
    CONSTRAINT workflows_draft_is_object CHECK (jsonb_typeof(draft) = 'object'),
    CONSTRAINT workflows_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id)
);

CREATE INDEX workflows_namespace_idx ON workflows (tenant_id, namespace_id, id);

CREATE TABLE workflow_versions (
    id             uuid        PRIMARY KEY,
    tenant_id      uuid        NOT NULL REFERENCES tenants (id),
    workflow_id    uuid        NOT NULL,
    version_number integer     NOT NULL,
    -- SHA-256 of the canonical document, lower-case hex.
    digest         text        NOT NULL,
    -- The canonical JSON the digest was computed over, byte for byte.
    document       bytea       NOT NULL,
    release_notes  text,
    published_by   text        NOT NULL,
    published_at   timestamptz NOT NULL,
    CONSTRAINT workflow_versions_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT workflow_versions_number_uniq UNIQUE (workflow_id, version_number),
    CONSTRAINT workflow_versions_digest_uniq UNIQUE (workflow_id, digest),
    CONSTRAINT workflow_versions_number_positive CHECK (version_number >= 1),
    CONSTRAINT workflow_versions_digest_format CHECK (digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT workflow_versions_workflow_same_tenant_fk
        FOREIGN KEY (tenant_id, workflow_id) REFERENCES workflows (tenant_id, id)
);

ALTER TABLE workflows
    ADD CONSTRAINT workflows_active_version_same_tenant_fk
    FOREIGN KEY (tenant_id, active_version_id) REFERENCES workflow_versions (tenant_id, id);

-- A published version never changes: runs are bound to it for their whole life.
CREATE FUNCTION workflow_versions_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'workflow_versions is immutable (% attempted)', TG_OP
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER workflow_versions_immutable
    BEFORE UPDATE OR DELETE ON workflow_versions
    FOR EACH ROW EXECUTE FUNCTION workflow_versions_reject_mutation();

CREATE TRIGGER workflow_versions_no_truncate
    BEFORE TRUNCATE ON workflow_versions
    FOR EACH STATEMENT EXECUTE FUNCTION workflow_versions_reject_mutation();

-- Fire even in a session that sets `session_replication_role = replica`.
ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_immutable;
ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_no_truncate;

-- Idempotency records (`Idempotency-Key`): the first request with a key wins, a repeat with the same
-- request returns the same resource, a different request with the same key is refused. The key is
-- scoped to the tenant and to the operation, so two tenants (or two operations) never collide.
CREATE TABLE command_dedupe (
    tenant_id       uuid        NOT NULL REFERENCES tenants (id),
    scope           text        NOT NULL,
    idempotency_key text        NOT NULL,
    -- SHA-256 of the request fingerprint; the request itself is never stored.
    request_hash    bytea       NOT NULL,
    -- Identifier of the resource the first request produced.
    resource_id     text        NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, scope, idempotency_key),
    CONSTRAINT command_dedupe_key_length CHECK (char_length(idempotency_key) BETWEEN 8 AND 160),
    CONSTRAINT command_dedupe_hash_length CHECK (octet_length(request_hash) = 32)
);

CREATE INDEX command_dedupe_age_idx ON command_dedupe (created_at);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON workflows TO sagat_app;
        GRANT SELECT, INSERT ON workflow_versions TO sagat_app;
        -- DELETE only on the idempotency records, which expire.
        GRANT SELECT, INSERT, DELETE ON command_dedupe TO sagat_app;
    END IF;
END
$grants$;
