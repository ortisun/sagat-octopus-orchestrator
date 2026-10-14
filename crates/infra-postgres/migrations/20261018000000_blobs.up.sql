-- FR-002-008: large payloads live in an object store; this is the durable catalogue.
--
-- Bytes are external to PostgreSQL, so their state cannot share the transaction that uploads or
-- deletes them. The catalogue makes both boundaries explicit:
--
--   UPLOADING -> UPLOADED -> COMMITTED
--          \          \
---           +-----------> DELETING -> finalized uploads removed after confirmed deletion
--
-- A step output may claim only an exact UPLOADED row in its own transaction. Cleanup first commits
--- DELETING and a numbered attempt, then calls the backend. Never-confirmed uploads retain a
--- tombstone even after deletion: a PUT with an unknown outcome may still complete later. Retried
--- deletes reconcile those late effects without exposing or committing the bytes.

CREATE TABLE blobs (
    tenant_id    uuid        NOT NULL REFERENCES tenants (id),
    namespace_id uuid        NOT NULL,
    id           uuid        NOT NULL,
    state        text        NOT NULL,
    -- Known only once the bytes are in the store. A failed upload may be DELETING without them.
    size_bytes   bigint,
    sha256       bytea,
    content_type text        NOT NULL,
    created_by   text        NOT NULL,
    -- The exact attempt allowed to attach this upload as its output.
    run_id       uuid        NOT NULL,
    step_id      uuid        NOT NULL,
    attempt      integer     NOT NULL,
    generation   bigint      NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    uploaded_at  timestamptz,
    committed_at timestamptz,
    deletion_attempts        integer NOT NULL DEFAULT 0,
    deletion_last_attempt_at timestamptz,
    delete_after             timestamptz,
    deletion_confirmed_at    timestamptz,
    PRIMARY KEY (tenant_id, namespace_id, id),
    CONSTRAINT blobs_state_valid
        CHECK (state IN ('UPLOADING', 'UPLOADED', 'COMMITTED', 'DELETING')),
    CONSTRAINT blobs_size_non_negative CHECK (size_bytes IS NULL OR size_bytes >= 0),
    CONSTRAINT blobs_sha256_length CHECK (sha256 IS NULL OR octet_length(sha256) = 32),
    CONSTRAINT blobs_measurement_consistent CHECK (
        (size_bytes IS NULL AND sha256 IS NULL AND uploaded_at IS NULL)
        OR (size_bytes IS NOT NULL AND sha256 IS NOT NULL AND uploaded_at IS NOT NULL)),
    CONSTRAINT blobs_measured_when_usable CHECK (
        state NOT IN ('UPLOADED', 'COMMITTED')
        OR (size_bytes IS NOT NULL AND sha256 IS NOT NULL AND uploaded_at IS NOT NULL)),
    CONSTRAINT blobs_uploading_unmeasured CHECK (
        state <> 'UPLOADING'
        OR (size_bytes IS NULL AND sha256 IS NULL AND uploaded_at IS NULL)),
    CONSTRAINT blobs_committed_has_a_time
        CHECK ((state = 'COMMITTED') = (committed_at IS NOT NULL)),
    CONSTRAINT blobs_deletion_evidence CHECK (
        (state = 'DELETING'
            AND deletion_attempts > 0
            AND deletion_last_attempt_at IS NOT NULL
            AND delete_after IS NOT NULL
            AND committed_at IS NULL)
        OR (state <> 'DELETING'
            AND deletion_attempts = 0
            AND deletion_last_attempt_at IS NULL
            AND delete_after IS NULL)),
    CONSTRAINT blobs_deletion_confirmation_valid CHECK (
        deletion_confirmed_at IS NULL OR (state = 'DELETING' AND uploaded_at IS NULL)),
    CONSTRAINT blobs_content_type_length CHECK (char_length(content_type) BETWEEN 3 AND 127),
    CONSTRAINT blobs_created_by_valid
        CHECK (char_length(created_by) BETWEEN 1 AND 255 AND created_by !~ '[[:cntrl:]]'),
    CONSTRAINT blobs_attempt_non_negative CHECK (attempt >= 0 AND generation >= 0),
    CONSTRAINT blobs_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT blobs_run_same_tenant_fk
        FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT blobs_step_same_tenant_fk
        FOREIGN KEY (tenant_id, step_id) REFERENCES steps (tenant_id, step_id)
);

-- Confirmed cleanup tombstones remain for reconciliation without occupying an active upload slot.
CREATE INDEX blobs_pending_by_tenant_idx
    ON blobs (tenant_id) WHERE state <> 'COMMITTED' AND deletion_confirmed_at IS NULL;
CREATE INDEX blobs_unused_by_age_idx
    ON blobs (created_at) WHERE state = 'UPLOADING';
CREATE INDEX blobs_uploaded_by_age_idx
    ON blobs (uploaded_at) WHERE state = 'UPLOADED';
CREATE INDEX blobs_delete_retry_idx
    ON blobs (delete_after) WHERE state = 'DELETING';

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON blobs TO sagat_app;
    END IF;
END
$grants$;
