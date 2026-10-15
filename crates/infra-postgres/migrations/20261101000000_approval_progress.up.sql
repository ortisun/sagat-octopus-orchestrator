-- Durable runtime approval projection. The run journal remains the canonical history.

CREATE FUNCTION approval_policy_snapshot_valid(subjects text[], roles text[]) RETURNS boolean
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT cardinality(subjects) + cardinality(roles) BETWEEN 1 AND 32
       AND cardinality(subjects) = (SELECT count(DISTINCT value) FROM unnest(subjects) AS item(value))
       AND cardinality(roles) = (SELECT count(DISTINCT value) FROM unnest(roles) AS item(value))
       AND NOT EXISTS (
            SELECT 1 FROM unnest(subjects) AS subject(value)
            WHERE value = '' OR char_length(value) > 255 OR value ~ '[[:cntrl:]]')
       AND roles <@ ARRAY['admin', 'developer', 'operator']::text[]
$$;

CREATE TABLE approval_requests (
    tenant_id             uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    request_id            uuid        NOT NULL,
    run_id                uuid        NOT NULL,
    owner_instance_id     uuid        NOT NULL,
    owner_node            text        NOT NULL,
    owner_path_nodes      text[]      NOT NULL,
    owner_path_indices    integer[]   NOT NULL,
    policy_subjects       text[]      NOT NULL,
    policy_roles          text[]      NOT NULL,
    exclude_initiator     boolean     NOT NULL,
    initiator             text        NOT NULL,
    opened_sequence       bigint      NOT NULL,
    deadline              timestamptz NOT NULL,
    status                text        NOT NULL,
    winning_receipt_id    uuid,
    outcome_sequence      bigint,
    settled_at            timestamptz,
    approved_node         text        NOT NULL,
    rejected_node         text        NOT NULL,
    expired_node          text        NOT NULL,
    routed_node           text,
    open_outbox_id        uuid        NOT NULL,
    outcome_outbox_id     uuid,
    created_at            timestamptz NOT NULL,
    updated_at            timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, request_id),
    CONSTRAINT approval_requests_namespace_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT approval_requests_run_fk
        FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT approval_requests_owner_fk
        FOREIGN KEY (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node)
        REFERENCES steps (tenant_id, run_id, instance_path_nodes, instance_path_indices, node),
    CONSTRAINT approval_requests_owner_unique
        UNIQUE (tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node),
    CONSTRAINT approval_requests_owner_id_unique UNIQUE (tenant_id, owner_instance_id),
    CONSTRAINT approval_requests_open_outbox_unique UNIQUE (open_outbox_id),
    CONSTRAINT approval_requests_outcome_outbox_unique UNIQUE (outcome_outbox_id),
    CONSTRAINT approval_requests_path_valid
        CHECK (execution_instance_path_valid(owner_path_nodes, owner_path_indices)),
    CONSTRAINT approval_requests_owner_node_valid
        CHECK (owner_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'),
    CONSTRAINT approval_requests_policy_snapshot CHECK (
        approval_policy_snapshot_valid(policy_subjects, policy_roles)),
    CONSTRAINT approval_requests_initiator CHECK (
        initiator <> '' AND char_length(initiator) <= 255 AND initiator !~ '[[:cntrl:]]'),
    CONSTRAINT approval_requests_sequences CHECK (
        opened_sequence >= 1 AND (outcome_sequence IS NULL OR outcome_sequence > opened_sequence)),
    CONSTRAINT approval_requests_status CHECK (
        status IN ('PENDING', 'APPROVED', 'REJECTED', 'EXPIRED', 'CANCELLED')),
    CONSTRAINT approval_requests_routes CHECK (
        approved_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
        AND rejected_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
        AND expired_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$'
        AND (routed_node IS NULL OR routed_node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$')),
    CONSTRAINT approval_requests_outcome_consistent CHECK (
        (status = 'PENDING'
            AND winning_receipt_id IS NULL AND outcome_sequence IS NULL AND settled_at IS NULL
            AND routed_node IS NULL AND outcome_outbox_id IS NULL)
        OR (status = 'APPROVED'
            AND winning_receipt_id IS NOT NULL AND outcome_sequence IS NOT NULL
            AND settled_at IS NOT NULL AND routed_node = approved_node
            AND outcome_outbox_id IS NOT NULL)
        OR (status = 'REJECTED'
            AND winning_receipt_id IS NOT NULL AND outcome_sequence IS NOT NULL
            AND settled_at IS NOT NULL AND routed_node = rejected_node
            AND outcome_outbox_id IS NOT NULL)
        OR (status = 'EXPIRED'
            AND winning_receipt_id IS NULL AND outcome_sequence IS NOT NULL
            AND settled_at IS NOT NULL AND routed_node = expired_node
            AND outcome_outbox_id IS NOT NULL)
        OR (status = 'CANCELLED'
            AND winning_receipt_id IS NULL AND outcome_sequence IS NOT NULL
            AND settled_at IS NOT NULL AND routed_node IS NULL))
);

CREATE INDEX approval_requests_run_idx
    ON approval_requests (tenant_id, run_id, opened_sequence);

CREATE TABLE approval_decision_receipts (
    tenant_id        uuid        NOT NULL,
    namespace_id     uuid        NOT NULL,
    receipt_id       uuid        NOT NULL,
    request_id       uuid        NOT NULL,
    decision_key     text        NOT NULL,
    request_hash     bytea       NOT NULL,
    actor            text        NOT NULL,
    decision         text        NOT NULL,
    reason_code      text,
    decided_at       timestamptz NOT NULL,
    outcome_sequence bigint      NOT NULL,
    PRIMARY KEY (tenant_id, request_id, decision_key),
    CONSTRAINT approval_decision_receipts_id_unique UNIQUE (tenant_id, receipt_id),
    CONSTRAINT approval_decision_receipts_request_fk
        FOREIGN KEY (tenant_id, request_id) REFERENCES approval_requests (tenant_id, request_id)
        ON DELETE CASCADE,
    CONSTRAINT approval_decision_receipts_namespace_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT approval_decision_receipts_key CHECK (
        char_length(decision_key) BETWEEN 1 AND 128 AND decision_key ~ '^[!-~]+$'),
    CONSTRAINT approval_decision_receipts_hash CHECK (octet_length(request_hash) = 32),
    CONSTRAINT approval_decision_receipts_actor CHECK (
        actor <> '' AND char_length(actor) <= 255 AND actor !~ '[[:cntrl:]]'),
    CONSTRAINT approval_decision_receipts_decision CHECK (decision IN ('APPROVE', 'REJECT')),
    CONSTRAINT approval_decision_receipts_reason CHECK (
        reason_code IS NULL OR
        (char_length(reason_code) BETWEEN 1 AND 64 AND reason_code ~ '^[A-Za-z0-9_.:-]+$')),
    CONSTRAINT approval_decision_receipts_sequence CHECK (outcome_sequence >= 2)
);

CREATE TRIGGER approval_decision_receipts_immutable
    BEFORE UPDATE OR DELETE ON approval_decision_receipts
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER approval_decision_receipts_no_truncate
    BEFORE TRUNCATE ON approval_decision_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE approval_decision_receipts
    ENABLE ALWAYS TRIGGER approval_decision_receipts_immutable;
ALTER TABLE approval_decision_receipts
    ENABLE ALWAYS TRIGGER approval_decision_receipts_no_truncate;

CREATE TABLE approval_timers (
    tenant_id   uuid        NOT NULL,
    request_id  uuid        NOT NULL,
    due_at      timestamptz NOT NULL,
    state       text        NOT NULL,
    updated_at  timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, request_id),
    CONSTRAINT approval_timers_request_fk
        FOREIGN KEY (tenant_id, request_id) REFERENCES approval_requests (tenant_id, request_id)
        ON DELETE CASCADE,
    CONSTRAINT approval_timers_state CHECK (state IN ('PENDING', 'CANCELLED', 'FIRED'))
);

CREATE INDEX approval_timers_due_idx ON approval_timers (due_at)
    WHERE state = 'PENDING';

ALTER TABLE steps ADD COLUMN approval_request_id uuid;
ALTER TABLE steps DROP CONSTRAINT steps_waiting_kind_valid;
ALTER TABLE steps ADD CONSTRAINT steps_waiting_kind_valid CHECK (
    waiting_kind IS NULL OR waiting_kind IN
    ('TIMER', 'EVENT', 'BRANCHES', 'CHILD', 'SAGA', 'RECONCILIATION', 'OPERATOR', 'COLLECTION', 'LOOP', 'APPROVAL')) NOT VALID;
ALTER TABLE steps ADD CONSTRAINT steps_approval_consistent CHECK (
    ((waiting_kind = 'APPROVAL') IS TRUE) = (approval_request_id IS NOT NULL)) NOT VALID;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT ON approval_requests TO sagat_app;
        GRANT UPDATE (status, winning_receipt_id, outcome_sequence, settled_at, routed_node,
            outcome_outbox_id, updated_at) ON approval_requests TO sagat_app;
        GRANT SELECT, INSERT ON approval_decision_receipts TO sagat_app;
        GRANT SELECT, INSERT ON approval_timers TO sagat_app;
        GRANT UPDATE (state, updated_at) ON approval_timers TO sagat_app;
    END IF;
END
$grants$;
