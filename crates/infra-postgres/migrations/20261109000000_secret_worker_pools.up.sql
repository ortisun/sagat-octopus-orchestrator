-- Feature 009, gap G48 (owner decision of 2026-10-16): a secret may list the worker pools allowed to
-- receive it. A pool is a name an administrator grants to a worker identity in one namespace (the
-- grant lives next to the identity's activity bindings); it is never a name the worker claims for
-- itself. A secret with no row here is delivered as before. Additive: nothing existing changes.

CREATE TABLE secret_pools (
    tenant_id  uuid        NOT NULL,
    secret_id  uuid        NOT NULL,
    pool       text        NOT NULL,
    granted_by text        NOT NULL,
    granted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (secret_id, pool),
    CONSTRAINT secret_pools_secret_fk FOREIGN KEY (tenant_id, secret_id)
        REFERENCES secrets (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT secret_pools_pool_valid
        CHECK (char_length(pool) BETWEEN 1 AND 64 AND pool ~ '^[A-Za-z0-9._:+-]+$'),
    CONSTRAINT secret_pools_granted_by_valid
        CHECK (char_length(granted_by) BETWEEN 1 AND 255 AND granted_by !~ '[[:cntrl:]]')
);

CREATE TABLE worker_binding_pools (
    tenant_id    uuid        NOT NULL REFERENCES tenants (id),
    namespace_id uuid        NOT NULL,
    subject      text        NOT NULL,
    pool         text        NOT NULL,
    updated_by   text        NOT NULL,
    updated_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, namespace_id, subject, pool),
    CONSTRAINT worker_binding_pools_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT worker_binding_pools_subject_valid
        CHECK (char_length(subject) BETWEEN 1 AND 255 AND subject !~ '[[:cntrl:]]'),
    CONSTRAINT worker_binding_pools_pool_valid
        CHECK (char_length(pool) BETWEEN 1 AND 64 AND pool ~ '^[A-Za-z0-9._:+-]+$'),
    CONSTRAINT worker_binding_pools_updated_by_valid
        CHECK (char_length(updated_by) BETWEEN 1 AND 255 AND updated_by !~ '[[:cntrl:]]')
);

-- The delivery check joins a secret's pools to the grants of one identity in the secret's namespace.
CREATE INDEX worker_binding_pools_pool_idx
    ON worker_binding_pools (tenant_id, namespace_id, pool, subject);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, DELETE ON secret_pools TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON worker_binding_pools TO sagat_app;
    END IF;
END
$grants$;
