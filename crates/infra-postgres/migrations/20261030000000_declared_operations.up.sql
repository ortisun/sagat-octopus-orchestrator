-- Persistent declarations, business state and idempotent receipts for declared operations.
-- Public operation endpoints remain disabled until their API, security and conformance gates pass.

CREATE TABLE declared_operation_definitions (
    tenant_id             uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    workflow_version_id   uuid        NOT NULL,
    operation_kind        text        NOT NULL,
    operation_name        text        NOT NULL,
    declaration_digest    bytea       NOT NULL,
    state_schema_digest   bytea       NOT NULL,
    result_schema_digest  bytea       NOT NULL,
    disclosure            text        NOT NULL,
    created_at            timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, workflow_version_id, operation_kind, operation_name),
    CONSTRAINT declared_operation_definitions_namespace_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT declared_operation_definitions_version_fk
        FOREIGN KEY (tenant_id, workflow_version_id) REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT declared_operation_definitions_kind CHECK (operation_kind IN ('QUERY', 'UPDATE')),
    CONSTRAINT declared_operation_definitions_name CHECK (
        operation_name ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
    CONSTRAINT declared_operation_definitions_digests CHECK (
        octet_length(declaration_digest) = 32
        AND octet_length(state_schema_digest) = 32
        AND octet_length(result_schema_digest) = 32),
    CONSTRAINT declared_operation_definitions_disclosure CHECK (
        disclosure = 'declared-business-result-v1')
);

CREATE TRIGGER declared_operation_definitions_immutable
    BEFORE UPDATE OR DELETE ON declared_operation_definitions
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER declared_operation_definitions_no_truncate
    BEFORE TRUNCATE ON declared_operation_definitions
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE declared_operation_definitions ENABLE ALWAYS TRIGGER declared_operation_definitions_immutable;
ALTER TABLE declared_operation_definitions ENABLE ALWAYS TRIGGER declared_operation_definitions_no_truncate;

CREATE TABLE workflow_operation_states (
    tenant_id                uuid        NOT NULL,
    namespace_id             uuid        NOT NULL,
    run_id                   uuid        NOT NULL,
    state_digest             bytea       NOT NULL,
    state_schema_digest      bytea       NOT NULL,
    state_size_bytes         integer     NOT NULL,
    state_revision           bigint      NOT NULL DEFAULT 0,
    last_changed_run_version bigint      NOT NULL,
    updated_at               timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT workflow_operation_states_run_fk
        FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT workflow_operation_states_namespace_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT workflow_operation_states_payload_fk
        FOREIGN KEY (tenant_id, state_digest) REFERENCES payloads (tenant_id, digest),
    CONSTRAINT workflow_operation_states_digests CHECK (
        octet_length(state_digest) = 32 AND octet_length(state_schema_digest) = 32),
    CONSTRAINT workflow_operation_states_size CHECK (state_size_bytes BETWEEN 0 AND 262144),
    CONSTRAINT workflow_operation_states_revisions CHECK (
        state_revision >= 0 AND last_changed_run_version >= 0)
);

CREATE TABLE operation_update_receipts (
    tenant_id             uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    address_scope         text        NOT NULL,
    target_id             uuid        NOT NULL,
    update_key            text        NOT NULL,
    resolved_run_id       uuid        NOT NULL,
    operation_name        text        NOT NULL,
    request_hash          bytea       NOT NULL,
    status                text        NOT NULL,
    rejection_code        text,
    rejection_expected    bigint,
    rejection_actual      bigint,
    result_digest         bytea,
    result_schema_digest  bytea,
    result_size_bytes     integer,
    before_state_revision bigint      NOT NULL,
    after_state_revision  bigint      NOT NULL,
    run_version           bigint      NOT NULL,
    actor                 text        NOT NULL,
    outbox_id             uuid,
    committed_at          timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, address_scope, target_id, update_key),
    CONSTRAINT operation_update_receipts_namespace_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT operation_update_receipts_run_fk
        FOREIGN KEY (tenant_id, resolved_run_id) REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT operation_update_receipts_result_fk
        FOREIGN KEY (tenant_id, result_digest) REFERENCES payloads (tenant_id, digest),
    CONSTRAINT operation_update_receipts_outbox_uniq UNIQUE (outbox_id),
    CONSTRAINT operation_update_receipts_scope CHECK (address_scope IN ('RUN', 'CHAIN')),
    CONSTRAINT operation_update_receipts_run_target CHECK (
        address_scope <> 'RUN' OR target_id = resolved_run_id),
    CONSTRAINT operation_update_receipts_key CHECK (
        char_length(update_key) BETWEEN 1 AND 128
        AND update_key ~ '^[!-~]+$'),
    CONSTRAINT operation_update_receipts_name CHECK (
        operation_name ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
    CONSTRAINT operation_update_receipts_hash CHECK (octet_length(request_hash) = 32),
    CONSTRAINT operation_update_receipts_status CHECK (status IN ('APPLIED', 'REJECTED')),
    CONSTRAINT operation_update_receipts_revisions CHECK (
        before_state_revision >= 0 AND after_state_revision >= 0 AND run_version >= 0),
    CONSTRAINT operation_update_receipts_result CHECK (
        (status = 'APPLIED'
            AND rejection_code IS NULL AND rejection_expected IS NULL AND rejection_actual IS NULL
            AND result_digest IS NOT NULL AND result_schema_digest IS NOT NULL
            AND result_size_bytes BETWEEN 0 AND 262144
            AND after_state_revision = before_state_revision + 1
            AND outbox_id IS NOT NULL)
        OR
        (status = 'REJECTED'
            AND rejection_code IN ('REVISION_CONFLICT', 'UPDATE_REJECTED')
            AND result_digest IS NULL AND result_schema_digest IS NULL AND result_size_bytes IS NULL
            AND after_state_revision = before_state_revision
            AND outbox_id IS NULL)),
    CONSTRAINT operation_update_receipts_rejection_revision CHECK (
        (rejection_code = 'REVISION_CONFLICT'
            AND rejection_expected IS NOT NULL AND rejection_expected >= 0
            AND rejection_actual IS NOT NULL AND rejection_actual >= 0)
        OR
        (rejection_code IS DISTINCT FROM 'REVISION_CONFLICT'
            AND rejection_expected IS NULL AND rejection_actual IS NULL)),
    CONSTRAINT operation_update_receipts_result_digest CHECK (
        result_digest IS NULL OR octet_length(result_digest) = 32),
    CONSTRAINT operation_update_receipts_schema_digest CHECK (
        result_schema_digest IS NULL OR octet_length(result_schema_digest) = 32),
    CONSTRAINT operation_update_receipts_actor CHECK (
        char_length(actor) BETWEEN 1 AND 255 AND actor !~ '[[:cntrl:]]')
);

CREATE INDEX operation_update_receipts_run_idx
    ON operation_update_receipts (tenant_id, resolved_run_id, committed_at);

CREATE TRIGGER operation_update_receipts_immutable
    BEFORE UPDATE OR DELETE ON operation_update_receipts
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER operation_update_receipts_no_truncate
    BEFORE TRUNCATE ON operation_update_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE operation_update_receipts ENABLE ALWAYS TRIGGER operation_update_receipts_immutable;
ALTER TABLE operation_update_receipts ENABLE ALWAYS TRIGGER operation_update_receipts_no_truncate;

CREATE TRIGGER declared_operation_definitions_quota_delta
    AFTER INSERT OR UPDATE OR DELETE ON declared_operation_definitions
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta(
        'content', 'octet_length_utf8', 'operation_name');
ALTER TABLE declared_operation_definitions ENABLE ALWAYS TRIGGER declared_operation_definitions_quota_delta;
CREATE CONSTRAINT TRIGGER declared_operation_definitions_quota_flush
    AFTER INSERT OR UPDATE OR DELETE ON declared_operation_definitions
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE declared_operation_definitions ENABLE ALWAYS TRIGGER declared_operation_definitions_quota_flush;

CREATE TRIGGER operation_update_receipts_quota_delta
    AFTER INSERT OR UPDATE OR DELETE ON operation_update_receipts
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta(
        'content', 'octet_length_utf8', 'operation_name',
        'content', 'octet_length_utf8', 'update_key');
ALTER TABLE operation_update_receipts ENABLE ALWAYS TRIGGER operation_update_receipts_quota_delta;
CREATE CONSTRAINT TRIGGER operation_update_receipts_quota_flush
    AFTER INSERT OR UPDATE OR DELETE ON operation_update_receipts
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE operation_update_receipts ENABLE ALWAYS TRIGGER operation_update_receipts_quota_flush;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT ON declared_operation_definitions TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON workflow_operation_states TO sagat_app;
        GRANT SELECT, INSERT ON operation_update_receipts TO sagat_app;
    END IF;
END
$grants$;
