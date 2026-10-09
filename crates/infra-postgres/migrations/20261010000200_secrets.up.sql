-- Feature 009: the secret store (FR-009-002, FR-009-003). Values are stored sealed (AES-256-GCM, the
-- associated data binds a ciphertext to its tenant, namespace, name and row), a few versions are kept
-- so a rotation can be undone, and each secret has its own access list: who may reference it from a
-- workflow they publish.

CREATE TABLE secrets (
    id              uuid        PRIMARY KEY,
    tenant_id       uuid        NOT NULL REFERENCES tenants (id),
    namespace_id    uuid        NOT NULL,
    name            text        NOT NULL,
    current_version integer     NOT NULL,
    created_by      text        NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at      timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- Target of the composite foreign keys of the tables below.
    CONSTRAINT secrets_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT secrets_name_uniq UNIQUE (tenant_id, namespace_id, name),
    CONSTRAINT secrets_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT secrets_name_format CHECK (name ~ '^[a-z][a-z0-9._-]{0,63}$'),
    CONSTRAINT secrets_version_positive CHECK (current_version >= 1)
);

CREATE TABLE secret_versions (
    -- Part of the associated data the value was sealed with.
    id          uuid        PRIMARY KEY,
    tenant_id   uuid        NOT NULL,
    secret_id   uuid        NOT NULL,
    version     integer     NOT NULL,
    key_id      text        NOT NULL,
    nonce       bytea       NOT NULL,
    ciphertext  bytea       NOT NULL,
    created_by  text        NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT secret_versions_unique UNIQUE (secret_id, version),
    CONSTRAINT secret_versions_secret_fk FOREIGN KEY (tenant_id, secret_id)
        REFERENCES secrets (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT secret_versions_nonce_len CHECK (octet_length(nonce) = 12),
    -- 1 to 16384 bytes of value plus the 16-byte tag.
    CONSTRAINT secret_versions_ciphertext_len CHECK (octet_length(ciphertext) BETWEEN 17 AND 16400),
    CONSTRAINT secret_versions_key_id CHECK (key_id ~ '^[a-z0-9_-]{1,32}$')
);

-- Finding what is still sealed under a retired key.
CREATE INDEX secret_versions_key_idx ON secret_versions (key_id);

CREATE TABLE secret_access (
    tenant_id  uuid        NOT NULL,
    secret_id  uuid        NOT NULL,
    subject    text        NOT NULL,
    granted_by text        NOT NULL,
    granted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (secret_id, subject),
    CONSTRAINT secret_access_secret_fk FOREIGN KEY (tenant_id, secret_id)
        REFERENCES secrets (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT secret_access_subject_valid
        CHECK (char_length(subject) BETWEEN 1 AND 255 AND subject !~ '[[:cntrl:]]')
);

-- Secrets are a quota resource, counted per namespace.
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets'));

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON secrets TO sagat_app;
        -- A rotation appends a version, the re-sealing job replaces an envelope, pruning deletes.
        GRANT SELECT, INSERT, UPDATE, DELETE ON secret_versions TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON secret_access TO sagat_app;
    END IF;
END
$grants$;
