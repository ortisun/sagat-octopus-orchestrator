-- Durable execution (feature 002; FR-002-002, FR-002-003, FR-002-004, FR-002-006, FR-002-007).
--
-- PostgreSQL is the single authority (ADR-0002). A run changes only inside one transaction that
-- appends journal events, updates the projections below, and writes the outbox, so a crash leaves
-- either all of it or none. Every foreign key is tenant-scoped: the database itself refuses a run, a
-- step or an event that points into another tenant.

-- Content-addressed payloads (run inputs, step outputs). Inline up to 256 KiB; a blob store takes the
-- larger ones in a later feature. Rows never change.
CREATE TABLE payloads (
    tenant_id  uuid        NOT NULL REFERENCES tenants (id),
    digest     bytea       NOT NULL,
    content    bytea       NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, digest),
    CONSTRAINT payloads_digest_length CHECK (octet_length(digest) = 32),
    CONSTRAINT payloads_size_limit CHECK (octet_length(content) <= 262144)
);

CREATE TABLE runs (
    id                  uuid        PRIMARY KEY,
    tenant_id           uuid        NOT NULL REFERENCES tenants (id),
    namespace_id        uuid        NOT NULL,
    workflow_id         uuid        NOT NULL,
    workflow_version_id uuid        NOT NULL,
    status              text        NOT NULL,
    -- Number of journal events applied; equals the sequence of the last journal entry.
    run_version         bigint      NOT NULL DEFAULT 0,
    stop_reason         text,
    stop_code           text,
    paused              boolean     NOT NULL DEFAULT false,
    input_digest        bytea       NOT NULL,
    correlation_id      text,
    started_by          text        NOT NULL,
    trace_id            text,
    deadline_at         timestamptz,
    created_at          timestamptz NOT NULL,
    updated_at          timestamptz NOT NULL,
    finished_at         timestamptz,
    -- A child run (FR-002-009): the run and step that started it, what happens to it when the parent
    -- stops, and how deeply it is nested. The link never changes; the two flags are set by the parent
    -- (under the parent's lock) so the scheduler finds exactly the children it still has to act on.
    parent_run_id       uuid,
    -- The top of the tree this run belongs to (itself for a run started through the API); the whole
    -- tree shares one size budget.
    root_run_id         uuid        NOT NULL,
    parent_node         text,
    cancel_policy       text,
    depth               smallint    NOT NULL DEFAULT 0,
    -- The parent no longer waits for this run (it was told the outcome, or its step was cancelled).
    parent_settled      boolean     NOT NULL DEFAULT false,
    -- The parent was stopped under a policy that cancels this run.
    parent_cancel       boolean     NOT NULL DEFAULT false,
    CONSTRAINT runs_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT runs_status_valid CHECK (status IN
        ('RUNNING', 'WAITING', 'PAUSED', 'CANCELLING', 'SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT')),
    CONSTRAINT runs_stop_reason_valid CHECK (stop_reason IS NULL OR stop_reason IN ('CANCELLED', 'TIMED_OUT', 'FAILED')),
    CONSTRAINT runs_version_nonnegative CHECK (run_version >= 0),
    CONSTRAINT runs_trace_id_format CHECK (trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$'),
    CONSTRAINT runs_parent_consistent CHECK (
        (parent_run_id IS NULL) = (parent_node IS NULL) AND (parent_run_id IS NULL) = (cancel_policy IS NULL)),
    CONSTRAINT runs_cancel_policy_valid CHECK (cancel_policy IS NULL OR cancel_policy IN ('PROPAGATE', 'REQUEST_CANCEL', 'DETACH')),
    CONSTRAINT runs_depth_valid CHECK (depth BETWEEN 0 AND 16),
    CONSTRAINT runs_root_has_depth_zero CHECK (parent_run_id IS NOT NULL OR depth = 0),
    CONSTRAINT runs_finished_consistent CHECK ((finished_at IS NULL) = (status NOT IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'TIMED_OUT'))),
    CONSTRAINT runs_namespace_same_tenant_fk FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT runs_workflow_same_tenant_fk FOREIGN KEY (tenant_id, workflow_id) REFERENCES workflows (tenant_id, id),
    CONSTRAINT runs_version_same_tenant_fk FOREIGN KEY (tenant_id, workflow_version_id) REFERENCES workflow_versions (tenant_id, id),
    CONSTRAINT runs_input_same_tenant_fk FOREIGN KEY (tenant_id, input_digest) REFERENCES payloads (tenant_id, digest),
    CONSTRAINT runs_parent_same_tenant_fk FOREIGN KEY (tenant_id, parent_run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT runs_root_same_tenant_fk FOREIGN KEY (tenant_id, root_run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT runs_root_of_a_root CHECK (parent_run_id IS NOT NULL OR root_run_id = id)
);

CREATE INDEX runs_tree_idx ON runs (tenant_id, root_run_id);

CREATE INDEX runs_listing_idx ON runs (tenant_id, namespace_id, id DESC);
CREATE INDEX runs_by_workflow_idx ON runs (tenant_id, workflow_id, id DESC);
-- Runs that can still hit their deadline.
CREATE INDEX runs_deadline_idx ON runs (deadline_at) WHERE finished_at IS NULL AND deadline_at IS NOT NULL;
-- Child runs that ended while their parent has not been told, and running children to cancel.
CREATE INDEX runs_unsettled_children_idx ON runs (finished_at)
    WHERE parent_run_id IS NOT NULL AND parent_settled = false AND finished_at IS NOT NULL;
CREATE INDEX runs_children_to_cancel_idx ON runs (id)
    WHERE parent_cancel AND finished_at IS NULL AND stop_reason IS NULL;

-- The journal: ordered, append-only, one sequence per run. The run's state can be rebuilt from it.
CREATE TABLE run_events (
    tenant_id      uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    seq            bigint      NOT NULL,
    event_type     text        NOT NULL,
    node           text,
    attempt        integer,
    -- The event's own fields (codes, times, targets), enough to rebuild it.
    detail         jsonb       NOT NULL,
    payload_digest bytea,
    actor          text        NOT NULL,
    trace_id       text,
    occurred_at    timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, seq),
    CONSTRAINT run_events_seq_positive CHECK (seq >= 1),
    CONSTRAINT run_events_trace_id_format CHECK (trace_id IS NULL OR trace_id ~ '^[0-9a-f]{32}$'),
    CONSTRAINT run_events_run_same_tenant_fk FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT run_events_payload_same_tenant_fk FOREIGN KEY (tenant_id, payload_digest) REFERENCES payloads (tenant_id, digest)
);

-- The current state of every step (a projection of the journal, written in the same transaction).
-- Timers and event subscriptions are rows of this table with status WAITING; their indexes below are
-- what lets a suspended run cost nothing but a row (FR-002-007).
CREATE TABLE steps (
    tenant_id          uuid        NOT NULL,
    run_id             uuid        NOT NULL,
    node               text        NOT NULL,
    -- Stable identity: derived from (version, run, path, node); unique per tenant to catch collisions.
    step_id            uuid        NOT NULL,
    namespace_id       uuid        NOT NULL,
    status             text        NOT NULL,
    attempt            integer     NOT NULL DEFAULT 0,
    generation         bigint      NOT NULL DEFAULT 0,
    cancel_requested   boolean     NOT NULL DEFAULT false,
    routed             boolean     NOT NULL DEFAULT false,
    condition          boolean,
    timed_out          boolean     NOT NULL DEFAULT false,
    failure_code       text,
    next_attempt_at    timestamptz,
    -- What a WAITING step is parked on, and when.
    waiting_kind       text,
    due_at             timestamptz,
    event_type         text,
    correlation_key    text,
    -- Scheduling: the activity a worker must advertise, when the step became READY, and when a
    -- notification about it was last written to the outbox.
    activity           text,
    ready_at           timestamptz,
    last_hinted_at     timestamptz,
    -- How many times the hint was written again because it may have been lost; capped.
    hint_count         integer     NOT NULL DEFAULT 0,
    -- The lease. Only a hash of the token is stored; renewals push the expiry out, never past the
    -- attempt deadline.
    lease_token_hash   bytea,
    lease_expires_at   timestamptz,
    attempt_deadline_at timestamptz,
    leased_by          text,
    output_digest      bytea,
    -- A `child` step: the run it reserved (kept after the step ends), whether that run has been
    -- created, and the input it starts with (dropped once it has).
    child_run_id       uuid,
    child_started      boolean     NOT NULL DEFAULT false,
    child_input        bytea,
    updated_at         timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, node),
    CONSTRAINT steps_step_id_uniq UNIQUE (tenant_id, step_id),
    CONSTRAINT steps_status_valid CHECK (status IN
        ('PENDING', 'READY', 'LEASED', 'RUNNING', 'WAITING', 'RETRY_WAIT', 'SUCCEEDED', 'FAILED', 'CANCELLED')),
    CONSTRAINT steps_waiting_kind_valid CHECK (waiting_kind IS NULL OR waiting_kind IN ('TIMER', 'EVENT', 'BRANCHES', 'CHILD')),
    CONSTRAINT steps_child_consistent CHECK (waiting_kind IS DISTINCT FROM 'CHILD' OR child_run_id IS NOT NULL),
    CONSTRAINT steps_child_input_size CHECK (child_input IS NULL OR octet_length(child_input) <= 262144),
    CONSTRAINT steps_waiting_consistent CHECK ((status = 'WAITING') = (waiting_kind IS NOT NULL)),
    CONSTRAINT steps_lease_consistent CHECK ((status IN ('LEASED', 'RUNNING')) = (lease_token_hash IS NOT NULL)),
    CONSTRAINT steps_counters_nonnegative CHECK (attempt >= 0 AND generation >= 0),
    CONSTRAINT steps_run_same_tenant_fk FOREIGN KEY (tenant_id, run_id) REFERENCES runs (tenant_id, id),
    CONSTRAINT steps_namespace_same_tenant_fk FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT steps_output_same_tenant_fk FOREIGN KEY (tenant_id, output_digest) REFERENCES payloads (tenant_id, digest)
);

-- Everything the scheduler asks is an index range scan over a small partial index.
CREATE INDEX steps_ready_idx ON steps (tenant_id, namespace_id, activity, ready_at) WHERE status = 'READY';
CREATE INDEX steps_run_ready_idx ON steps (tenant_id, run_id) WHERE status = 'READY';
CREATE INDEX steps_lease_expiry_idx ON steps (lease_expires_at) WHERE status IN ('LEASED', 'RUNNING');
CREATE INDEX steps_attempt_deadline_idx ON steps (attempt_deadline_at)
    WHERE status IN ('LEASED', 'RUNNING') AND attempt_deadline_at IS NOT NULL;
CREATE INDEX steps_retry_idx ON steps (next_attempt_at) WHERE status = 'RETRY_WAIT';
CREATE INDEX steps_timer_idx ON steps (due_at) WHERE status = 'WAITING' AND waiting_kind = 'TIMER';
CREATE INDEX steps_wait_deadline_idx ON steps (due_at) WHERE status = 'WAITING' AND waiting_kind = 'EVENT';
CREATE INDEX steps_event_match_idx ON steps (tenant_id, namespace_id, event_type, correlation_key)
    WHERE status = 'WAITING' AND waiting_kind = 'EVENT';
CREATE INDEX steps_rehint_idx ON steps (last_hinted_at) WHERE status = 'READY' AND hint_count < 5;
-- Child runs reserved by a step and not created yet.
CREATE INDEX steps_child_pending_idx ON steps (updated_at)
    WHERE status = 'WAITING' AND waiting_kind = 'CHILD' AND child_started = false;

CREATE TABLE worker_sessions (
    id           uuid        PRIMARY KEY,
    tenant_id    uuid        NOT NULL REFERENCES tenants (id),
    namespace_id uuid        NOT NULL,
    subject      text        NOT NULL,
    worker_name  text        NOT NULL,
    -- SHA-256 of the session token; the token itself is shown once, at registration.
    token_hash   bytea       NOT NULL,
    activities   text[]      NOT NULL,
    languages    text[]      NOT NULL DEFAULT '{}',
    pools        text[]      NOT NULL DEFAULT '{}',
    max_inflight integer     NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at   timestamptz NOT NULL,
    CONSTRAINT worker_sessions_tenant_id_uniq UNIQUE (tenant_id, id),
    CONSTRAINT worker_sessions_token_length CHECK (octet_length(token_hash) = 32),
    CONSTRAINT worker_sessions_inflight_valid CHECK (max_inflight BETWEEN 1 AND 1000),
    CONSTRAINT worker_sessions_namespace_same_tenant_fk FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id)
);

CREATE INDEX worker_sessions_expiry_idx ON worker_sessions (expires_at);
-- One session per identity and worker name: registering again replaces it.
CREATE UNIQUE INDEX worker_sessions_identity_uniq ON worker_sessions (tenant_id, subject, worker_name);

-- Transactional outbox: written in the same transaction as the state change, published to the broker
-- by a dispatcher. Delivery is at-least-once; the broker is a hint and never the source of truth.
CREATE TABLE event_outbox (
    id              uuid        PRIMARY KEY,
    tenant_id       uuid        NOT NULL REFERENCES tenants (id),
    aggregate_id    uuid        NOT NULL,
    event_type      text        NOT NULL,
    -- Narrows the broker subject (the activity of a `task.ready` hint); NULL when the event type
    -- and the tenant are enough.
    routing_key     text,
    payload         jsonb       NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT clock_timestamp(),
    next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    attempt_count   integer     NOT NULL DEFAULT 0,
    delivered_at    timestamptz,
    last_error_code text
);

CREATE INDEX event_outbox_pending_idx ON event_outbox (next_attempt_at) WHERE delivered_at IS NULL;
-- Delivered messages are kept for a while and then purged in bounded batches.
CREATE INDEX event_outbox_delivered_idx ON event_outbox (delivered_at) WHERE delivered_at IS NOT NULL;

-- Append-only and content-addressed tables never change.
CREATE FUNCTION execution_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% is append-only (% attempted)', TG_TABLE_NAME, TG_OP
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER run_events_append_only BEFORE UPDATE OR DELETE ON run_events
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER run_events_no_truncate BEFORE TRUNCATE ON run_events
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER payloads_append_only BEFORE UPDATE OR DELETE ON payloads
    FOR EACH ROW EXECUTE FUNCTION execution_reject_mutation();
CREATE TRIGGER payloads_no_truncate BEFORE TRUNCATE ON payloads
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();

-- The runtime role may delete delivered outbox messages (retention) but never one that is still owed:
-- that would be a lost event, whoever asks.
CREATE FUNCTION outbox_reject_undelivered_delete() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.delivered_at IS NULL THEN
        RAISE EXCEPTION 'an undelivered outbox message cannot be deleted'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;

CREATE TRIGGER event_outbox_keep_undelivered BEFORE DELETE ON event_outbox
    FOR EACH ROW EXECUTE FUNCTION outbox_reject_undelivered_delete();

ALTER TABLE event_outbox ENABLE ALWAYS TRIGGER event_outbox_keep_undelivered;
ALTER TABLE run_events ENABLE ALWAYS TRIGGER run_events_append_only;
ALTER TABLE run_events ENABLE ALWAYS TRIGGER run_events_no_truncate;
ALTER TABLE payloads ENABLE ALWAYS TRIGGER payloads_append_only;
ALTER TABLE payloads ENABLE ALWAYS TRIGGER payloads_no_truncate;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT ON payloads, run_events TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON runs, steps TO sagat_app;
        -- Delivered outbox messages are purged after their retention; nothing else may be deleted.
        GRANT SELECT, INSERT, UPDATE, DELETE ON event_outbox TO sagat_app;
        -- Sessions expire, so the runtime may delete them (and only them).
        GRANT SELECT, INSERT, UPDATE, DELETE ON worker_sessions TO sagat_app;
    END IF;
END
$grants$;
