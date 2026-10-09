-- Feature 009: worker bindings (FR-009-002, gap G31). `worker:execute` makes an identity a worker; a
-- binding says which activities of which namespace it may serve. No binding, no work.

CREATE TABLE worker_bindings (
    tenant_id    uuid        NOT NULL REFERENCES tenants (id),
    namespace_id uuid        NOT NULL,
    subject      text        NOT NULL,
    -- `*`, `<name>.*` or an exact activity name (the grammar of sagat_domain::binding).
    pattern      text        NOT NULL,
    updated_by   text        NOT NULL,
    updated_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, namespace_id, subject, pattern),
    CONSTRAINT worker_bindings_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT worker_bindings_subject_valid
        CHECK (char_length(subject) BETWEEN 1 AND 255 AND subject !~ '[[:cntrl:]]'),
    CONSTRAINT worker_bindings_pattern_valid
        CHECK (char_length(pattern) BETWEEN 1 AND 130
               AND pattern ~ '^(\*|[A-Za-z0-9][A-Za-z0-9_.:-]*(\.\*)?)$')
);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON worker_bindings TO sagat_app;
    END IF;
END
$grants$;
