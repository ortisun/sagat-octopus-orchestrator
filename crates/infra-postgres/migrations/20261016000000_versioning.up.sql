-- Versions, approvals and the active pointer (feature 011; FR-011-001, FR-011-003, FR-011-005,
-- FR-011-006).
--
-- A published version never changes. What moves is the pointer to the version new runs use, and the
-- permission to publish into a namespace. This records both: who may publish into a namespace and
-- with how many approvals, the requests to publish and what each person decided, every change of the
-- pointer (the history a rollback is read from), and the artifacts each version depends on.

-- How many people other than the author must approve a publication into a namespace. No row means
-- the default of the namespace's environment (one in production, none elsewhere): the platform
-- decides it, so a namespace created later is covered without a row for it.
CREATE TABLE publication_policies (
    tenant_id          uuid        NOT NULL,
    namespace_id       uuid        NOT NULL,
    required_approvals smallint    NOT NULL,
    set_by             text        NOT NULL,
    set_at             timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id),
    CONSTRAINT publication_policies_range CHECK (required_approvals BETWEEN 0 AND 3),
    CONSTRAINT publication_policies_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id)
);

-- A request to publish a workflow into a namespace that asks for approvals. It names the draft
-- revision and the digest of the document, so the people who approve it approve that document.
CREATE TABLE publication_requests (
    id                       uuid        PRIMARY KEY,
    tenant_id                uuid        NOT NULL REFERENCES tenants (id),
    namespace_id             uuid        NOT NULL,
    workflow_id              uuid        NOT NULL,
    kind                     text        NOT NULL,
    draft_revision           bigint      NOT NULL,
    digest                   text        NOT NULL,
    release_notes            text,
    -- For a promotion: the version, in another namespace, whose document this publishes.
    promoted_from_version_id uuid,
    requested_by             text        NOT NULL,
    requested_at             timestamptz NOT NULL,
    -- What the policy asked for when the request was opened; the policy in force is read again
    -- when it is published.
    required_approvals       smallint    NOT NULL,
    state                    text        NOT NULL,
    published_version_id     uuid,
    published_by             text,
    published_at             timestamptz,
    updated_at               timestamptz NOT NULL,
    CONSTRAINT publication_requests_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT publication_requests_kind_valid CHECK (kind IN ('PUBLISH', 'PROMOTE')),
    CONSTRAINT publication_requests_state_valid CHECK (
        state IN ('PENDING', 'APPROVED', 'REJECTED', 'WITHDRAWN', 'SUPERSEDED', 'PUBLISHED')),
    CONSTRAINT publication_requests_digest_format CHECK (digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT publication_requests_range CHECK (
        required_approvals BETWEEN 0 AND 3 AND draft_revision >= 1),
    CONSTRAINT publication_requests_published_consistent CHECK (
        (state = 'PUBLISHED') = (published_version_id IS NOT NULL)
        AND (published_version_id IS NULL) = (published_by IS NULL)
        AND (published_version_id IS NULL) = (published_at IS NULL)),
    CONSTRAINT publication_requests_promoted_consistent CHECK (
        (kind = 'PROMOTE') = (promoted_from_version_id IS NOT NULL)),
    CONSTRAINT publication_requests_workflow_fk FOREIGN KEY (tenant_id, workflow_id)
        REFERENCES workflows (tenant_id, id),
    CONSTRAINT publication_requests_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT publication_requests_promoted_fk FOREIGN KEY (tenant_id, promoted_from_version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT publication_requests_published_fk FOREIGN KEY (tenant_id, published_version_id)
        REFERENCES workflow_versions (tenant_id, id)
);
-- One request is open for a workflow at a time: what people are asked to approve is unambiguous.
CREATE UNIQUE INDEX publication_requests_open_uniq ON publication_requests (tenant_id, workflow_id)
    WHERE state IN ('PENDING', 'APPROVED');
CREATE INDEX publication_requests_listing_idx ON publication_requests (tenant_id, workflow_id, id DESC);

-- What each person decided about a request, and why. A person decides once.
CREATE TABLE publication_decisions (
    tenant_id  uuid        NOT NULL,
    request_id uuid        NOT NULL,
    approver   text        NOT NULL,
    verdict    text        NOT NULL,
    reason     text        NOT NULL,
    decided_at timestamptz NOT NULL,
    PRIMARY KEY (request_id, approver),
    CONSTRAINT publication_decisions_verdict_valid CHECK (verdict IN ('APPROVED', 'REJECTED')),
    CONSTRAINT publication_decisions_reason_length CHECK (char_length(reason) BETWEEN 8 AND 2000),
    CONSTRAINT publication_decisions_request_fk FOREIGN KEY (tenant_id, request_id)
        REFERENCES publication_requests (tenant_id, id)
);

-- Every change of the pointer to the active version of a workflow, oldest to newest (by time, then
-- by identifier). The last row of a workflow is its pointer; a rollback does not rewrite anything
-- above it.
CREATE TABLE workflow_activations (
    id                       uuid        PRIMARY KEY,
    tenant_id                uuid        NOT NULL REFERENCES tenants (id),
    workflow_id              uuid        NOT NULL,
    version_id               uuid        NOT NULL,
    previous_version_id      uuid,
    kind                     text        NOT NULL,
    reason                   text,
    actor                    text        NOT NULL,
    request_id               uuid,
    promoted_from_version_id uuid,
    created_at               timestamptz NOT NULL,
    CONSTRAINT workflow_activations_kind_valid CHECK (kind IN ('PUBLISH', 'PROMOTE', 'ROLLBACK', 'ACTIVATE')),
    CONSTRAINT workflow_activations_workflow_fk FOREIGN KEY (tenant_id, workflow_id)
        REFERENCES workflows (tenant_id, id),
    CONSTRAINT workflow_activations_version_fk FOREIGN KEY (tenant_id, version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT workflow_activations_previous_fk FOREIGN KEY (tenant_id, previous_version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT workflow_activations_request_fk FOREIGN KEY (tenant_id, request_id)
        REFERENCES publication_requests (tenant_id, id),
    CONSTRAINT workflow_activations_promoted_fk FOREIGN KEY (tenant_id, promoted_from_version_id)
        REFERENCES workflow_versions (tenant_id, id)
);
CREATE INDEX workflow_activations_history_idx ON workflow_activations (tenant_id, workflow_id, created_at DESC, id DESC);

CREATE FUNCTION workflow_activations_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'workflow_activations is append-only (% attempted)', TG_OP
        USING ERRCODE = 'restrict_violation';
END;
$$;
CREATE TRIGGER workflow_activations_append_only
    BEFORE UPDATE OR DELETE ON workflow_activations
    FOR EACH ROW EXECUTE FUNCTION workflow_activations_reject_mutation();
CREATE TRIGGER workflow_activations_no_truncate
    BEFORE TRUNCATE ON workflow_activations
    FOR EACH STATEMENT EXECUTE FUNCTION workflow_activations_reject_mutation();

-- The script artifacts a version pins (FR-011-001). The foreign key is the guard of FR-011-006: the
-- database refuses to remove an artifact that a published version depends on, whatever policy of
-- retention is built later, because a rollback can make any published version active again.
CREATE TABLE workflow_version_scripts (
    tenant_id           uuid NOT NULL,
    workflow_version_id uuid NOT NULL,
    digest              text NOT NULL,
    PRIMARY KEY (tenant_id, workflow_version_id, digest),
    CONSTRAINT workflow_version_scripts_version_fk FOREIGN KEY (tenant_id, workflow_version_id)
        REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT workflow_version_scripts_artifact_fk FOREIGN KEY (tenant_id, digest)
        REFERENCES script_artifacts (tenant_id, digest) ON DELETE RESTRICT
);
CREATE INDEX workflow_version_scripts_digest_idx ON workflow_version_scripts (tenant_id, digest);

-- Versions published before this migration: the artifacts their documents pin.
INSERT INTO workflow_version_scripts (tenant_id, workflow_version_id, digest)
SELECT DISTINCT v.tenant_id, v.id, n ->> 'artifactDigest'
FROM workflow_versions v
CROSS JOIN LATERAL jsonb_array_elements(convert_from(v.document, 'UTF8')::jsonb -> 'nodes') AS n
WHERE n ->> 'type' = 'script'
  AND n ->> 'artifactDigest' ~ '^sha256:[0-9a-f]{64}$'
  AND EXISTS (SELECT 1 FROM script_artifacts a
              WHERE a.tenant_id = v.tenant_id AND a.digest = n ->> 'artifactDigest');

-- Until now every publication made the version active, so the history that existed is the list of
-- publications; and a pointer that was moved by publishing an identical document again is closed
-- with one row that says it was recorded when the history began.
INSERT INTO workflow_activations
    (id, tenant_id, workflow_id, version_id, previous_version_id, kind, reason, actor, created_at)
SELECT gen_random_uuid(), v.tenant_id, v.workflow_id, v.id,
       lag(v.id) OVER (PARTITION BY v.workflow_id ORDER BY v.version_number),
       'PUBLISH', v.release_notes, v.published_by, v.published_at
FROM workflow_versions v
ORDER BY v.published_at, v.version_number;

INSERT INTO workflow_activations
    (id, tenant_id, workflow_id, version_id, previous_version_id, kind, reason, actor, created_at)
SELECT gen_random_uuid(), w.tenant_id, w.id, w.active_version_id,
       (SELECT a.version_id FROM workflow_activations a
        WHERE a.tenant_id = w.tenant_id AND a.workflow_id = w.id ORDER BY a.created_at DESC, a.id DESC LIMIT 1),
       'ACTIVATE', 'recorded when the history of the active version began', 'system:migration',
       w.updated_at
FROM workflows w
WHERE w.active_version_id IS NOT NULL
  AND w.active_version_id IS DISTINCT FROM
      (SELECT a.version_id FROM workflow_activations a
       WHERE a.tenant_id = w.tenant_id AND a.workflow_id = w.id ORDER BY a.created_at DESC, a.id DESC LIMIT 1);

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, UPDATE ON publication_policies, publication_requests TO sagat_app;
        GRANT SELECT, INSERT ON publication_decisions, workflow_activations, workflow_version_scripts TO sagat_app;
    END IF;
END
$grants$;
