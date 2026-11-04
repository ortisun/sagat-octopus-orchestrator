-- Feature 018: batch operations. A job applies one action to a selection of runs, each run in its own
-- transaction at a durable rate (FR-018-027..034). The job and its items are evidence of what was done
-- to which runs, by whom and why; they are retained for the window (30 days, at most 90) and purged by a
-- bounded function added with the maintenance work.
--
-- Additive: an older engine or control plane never reads these tables.

-- The ceilings tables name every resource they may hold, so the lists are widened by the two counts of
-- active jobs (2 and 8 by default in code; hard 16 and 64). Only `batch_jobs` may be set for a single
-- namespace; `tenant_batch_jobs` is the tenant's.
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency', 'iteration_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days',
     'database_statement_bytes', 'database_parameters', 'database_parameter_bytes',
     'database_rows', 'database_result_bytes', 'database_timeout_ms',
     'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms',
     'database_connections', 'database_concurrency', 'tenant_database_concurrency',
     'secret_provider_operations', 'schedules', 'schedule_backfills',
     'batch_jobs', 'tenant_batch_jobs'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'tenant_database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs', 'tenant_batch_jobs'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (namespace_id IS NULL OR resource IN ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs', 'activity_concurrency', 'api_requests_per_second', 'events_per_second', 'event_retention_seconds', 'script_retention_days', 'database_statement_bytes', 'database_parameters', 'database_parameter_bytes', 'database_rows', 'database_result_bytes', 'database_timeout_ms', 'database_connect_timeout_ms', 'database_pool_size', 'database_idle_timeout_ms', 'database_connections', 'database_concurrency', 'secret_provider_operations', 'schedules', 'schedule_backfills', 'batch_jobs'));

CREATE TABLE batch_jobs (
    tenant_id          uuid        NOT NULL REFERENCES tenants (id),
    namespace_id       uuid        NOT NULL,
    id                 uuid        NOT NULL,
    action             text        NOT NULL,
    -- the parameters of the action (a step id, an instance path, an operation name, an update key)
    params             jsonb       NOT NULL DEFAULT '{}',
    -- the signal payload or the update arguments, a payload of the tenant
    payload_digest     bytea,
    -- the structured filter, or the list of run ids; may carry a business-id prefix (customer text)
    selector           jsonb       NOT NULL,
    reason             text        NOT NULL,
    created_by         text        NOT NULL,
    state              text        NOT NULL,
    failure_reason     text,
    max_items          integer     NOT NULL,
    -- items per second times 1,000
    rate_milli         integer     NOT NULL,
    selected           integer     NOT NULL DEFAULT 0,
    succeeded          integer     NOT NULL DEFAULT 0,
    skipped            integer     NOT NULL DEFAULT 0,
    failed             integer     NOT NULL DEFAULT 0,
    cancelled          integer     NOT NULL DEFAULT 0,
    deferred           integer     NOT NULL DEFAULT 0,
    -- keyset cursor of the selection: the last run id looked at
    select_cursor      uuid,
    -- the selection looks at the runs created at or before this instant (the database clock)
    selection_bound_at timestamptz NOT NULL,
    -- the selection starts once every transaction that was open when the job was created has ended
    -- (`pg_snapshot_xmin(pg_current_snapshot())` has reached it), so no run that committed later with
    -- an earlier creation time is missed
    selection_horizon  xid8        NOT NULL,
    -- the durable rate slot: the earliest instant the next item may be applied
    next_slot_at       timestamptz NOT NULL,
    shard              smallint    GENERATED ALWAYS AS (run_shard(id)) STORED,
    created_at         timestamptz NOT NULL,
    started_at         timestamptz,
    finished_at        timestamptz,
    expires_at         timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, id),
    CONSTRAINT batch_jobs_namespace_same_tenant_fk
        FOREIGN KEY (tenant_id, namespace_id) REFERENCES namespaces (tenant_id, id),
    CONSTRAINT batch_jobs_payload_fk
        FOREIGN KEY (tenant_id, payload_digest) REFERENCES payloads (tenant_id, digest),
    CONSTRAINT batch_jobs_action_valid CHECK (action IN
        ('CANCEL', 'SIGNAL', 'UPDATE', 'RERUN', 'RETRY_STEP', 'PAUSE', 'RESUME', 'MARK_MANUAL_REVIEW', 'REPIN')),
    CONSTRAINT batch_jobs_state_valid CHECK (state IN
        ('SELECTING', 'RUNNING', 'CANCELLING', 'COMPLETED', 'CANCELLED', 'FAILED')),
    CONSTRAINT batch_jobs_failure_reason_valid CHECK (failure_reason IS NULL OR failure_reason IN
        ('AUTHORITY_REVOKED', 'SELECTION_FAILED')),
    CONSTRAINT batch_jobs_failure_only_when_failed CHECK ((state = 'FAILED') = (failure_reason IS NOT NULL)),
    CONSTRAINT batch_jobs_params_is_object CHECK (jsonb_typeof(params) = 'object'),
    CONSTRAINT batch_jobs_params_bounded CHECK (octet_length(params::text) <= 1024),
    CONSTRAINT batch_jobs_payload_digest_size CHECK (payload_digest IS NULL OR octet_length(payload_digest) = 32),
    CONSTRAINT batch_jobs_selector_is_object CHECK (jsonb_typeof(selector) = 'object'),
    CONSTRAINT batch_jobs_selector_bounded CHECK (octet_length(selector::text) <= 65536),
    CONSTRAINT batch_jobs_reason_bounded CHECK (char_length(reason) BETWEEN 8 AND 2000),
    CONSTRAINT batch_jobs_created_by_bounded CHECK (char_length(created_by) BETWEEN 1 AND 255),
    CONSTRAINT batch_jobs_max_items_valid CHECK (max_items BETWEEN 1 AND 100000),
    CONSTRAINT batch_jobs_rate_valid CHECK (rate_milli BETWEEN 1 AND 200000),
    CONSTRAINT batch_jobs_counters_valid CHECK (
        selected >= 0 AND succeeded >= 0 AND skipped >= 0 AND failed >= 0 AND cancelled >= 0 AND deferred >= 0
        AND selected <= max_items
        AND succeeded + skipped + failed + cancelled <= selected
        AND deferred <= selected),
    CONSTRAINT batch_jobs_finished_consistent CHECK (
        (state IN ('COMPLETED', 'CANCELLED', 'FAILED')) = (finished_at IS NOT NULL))
);
-- The jobs a pass looks after, earliest slot first.
CREATE INDEX batch_jobs_due_idx ON batch_jobs (shard, next_slot_at)
    WHERE state IN ('SELECTING', 'RUNNING', 'CANCELLING');
-- The jobs of a namespace, newest first (ids are time-ordered), and the active ones for the ceilings.
CREATE INDEX batch_jobs_namespace_idx ON batch_jobs (tenant_id, namespace_id, id DESC);
CREATE INDEX batch_jobs_active_idx ON batch_jobs (tenant_id, namespace_id)
    WHERE state IN ('SELECTING', 'RUNNING', 'CANCELLING');

-- What a job is, and why it exists, never changes; only its progress does.
CREATE FUNCTION batch_jobs_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.state NOT IN ('COMPLETED', 'CANCELLED', 'FAILED') THEN
            RAISE EXCEPTION 'a batch job that has not finished is never purged'
                USING ERRCODE = 'restrict_violation';
        END IF;
        RETURN OLD;
    END IF;
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.namespace_id IS DISTINCT FROM OLD.namespace_id
        OR NEW.id IS DISTINCT FROM OLD.id
        OR NEW.action IS DISTINCT FROM OLD.action
        OR NEW.params IS DISTINCT FROM OLD.params
        OR NEW.payload_digest IS DISTINCT FROM OLD.payload_digest
        OR NEW.selector IS DISTINCT FROM OLD.selector
        OR NEW.reason IS DISTINCT FROM OLD.reason
        OR NEW.created_by IS DISTINCT FROM OLD.created_by
        OR NEW.max_items IS DISTINCT FROM OLD.max_items
        OR NEW.rate_milli IS DISTINCT FROM OLD.rate_milli
        OR NEW.selection_bound_at IS DISTINCT FROM OLD.selection_bound_at
        OR NEW.selection_horizon IS DISTINCT FROM OLD.selection_horizon
        OR NEW.created_at IS DISTINCT FROM OLD.created_at
        OR NEW.expires_at IS DISTINCT FROM OLD.expires_at THEN
        RAISE EXCEPTION 'what a batch job is never changes; only its progress does'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.state IN ('COMPLETED', 'CANCELLED', 'FAILED') AND NEW.state IS DISTINCT FROM OLD.state THEN
        RAISE EXCEPTION 'a batch job that has finished stays finished'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER batch_jobs_guard BEFORE UPDATE OR DELETE ON batch_jobs
    FOR EACH ROW EXECUTE FUNCTION batch_jobs_guard();
CREATE TRIGGER batch_jobs_no_truncate BEFORE TRUNCATE ON batch_jobs
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE batch_jobs ENABLE ALWAYS TRIGGER batch_jobs_guard;
ALTER TABLE batch_jobs ENABLE ALWAYS TRIGGER batch_jobs_no_truncate;

-- The justification and the selector are customer content: charged to the tenant by the ALWAYS
-- accounting triggers, as the manifest of storage classes says.
CREATE TRIGGER batch_jobs_quota_delta AFTER INSERT OR UPDATE OR DELETE ON batch_jobs
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'canonical_json_utf8', 'selector', 'content', 'octet_length_utf8', 'reason');
ALTER TABLE batch_jobs ENABLE ALWAYS TRIGGER batch_jobs_quota_delta;
CREATE CONSTRAINT TRIGGER batch_jobs_quota_flush AFTER INSERT OR UPDATE OR DELETE ON batch_jobs
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE batch_jobs ENABLE ALWAYS TRIGGER batch_jobs_quota_flush;

-- One row per selected run. Items do not reference `runs` by a foreign key, so a run purged by a
-- retention policy does not block the retention of its item; the item keeps the identifier only.
CREATE TABLE batch_items (
    tenant_id       uuid        NOT NULL,
    job_id          uuid        NOT NULL,
    ordinal         integer     NOT NULL,
    run_id          uuid        NOT NULL,
    state           text        NOT NULL DEFAULT 'PENDING',
    -- why it was skipped, or why it is deferred: a closed code
    reason          text,
    -- the `Error.code` of a failure: a closed code
    error_code      text,
    attempts        smallint    NOT NULL DEFAULT 0,
    next_attempt_at timestamptz,
    -- the run a rerun created
    new_run_id      uuid,
    updated_at      timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, job_id, ordinal),
    CONSTRAINT batch_items_job_fk
        FOREIGN KEY (tenant_id, job_id) REFERENCES batch_jobs (tenant_id, id),
    CONSTRAINT batch_items_one_per_run UNIQUE (tenant_id, job_id, run_id),
    CONSTRAINT batch_items_ordinal_positive CHECK (ordinal >= 1),
    CONSTRAINT batch_items_state_valid CHECK (state IN
        ('PENDING', 'DEFERRED', 'SUCCEEDED', 'SKIPPED', 'FAILED', 'CANCELLED')),
    CONSTRAINT batch_items_reason_closed CHECK (reason IS NULL OR reason ~ '^[A-Z][A-Z0-9_]{0,63}$'),
    CONSTRAINT batch_items_error_code_closed CHECK (error_code IS NULL OR error_code ~ '^[A-Z][A-Z0-9_]{0,63}$'),
    CONSTRAINT batch_items_attempts_valid CHECK (attempts >= 0)
);
CREATE INDEX batch_items_state_idx ON batch_items (tenant_id, job_id, state, ordinal);

-- An item that has ended does not change again; before that it moves on to any state.
CREATE FUNCTION batch_items_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.job_id IS DISTINCT FROM OLD.job_id
        OR NEW.ordinal IS DISTINCT FROM OLD.ordinal
        OR NEW.run_id IS DISTINCT FROM OLD.run_id THEN
        RAISE EXCEPTION 'which run an item stands for never changes'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF OLD.state IN ('SUCCEEDED', 'SKIPPED', 'FAILED', 'CANCELLED') AND NEW.state IS DISTINCT FROM OLD.state THEN
        RAISE EXCEPTION 'a batch item that has ended stays as it ended'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER batch_items_guard BEFORE UPDATE OR DELETE ON batch_items
    FOR EACH ROW EXECUTE FUNCTION batch_items_guard();
CREATE TRIGGER batch_items_no_truncate BEFORE TRUNCATE ON batch_items
    FOR EACH STATEMENT EXECUTE FUNCTION execution_reject_mutation();
ALTER TABLE batch_items ENABLE ALWAYS TRIGGER batch_items_guard;
ALTER TABLE batch_items ENABLE ALWAYS TRIGGER batch_items_no_truncate;

-- The runtime role reads and writes what the stores need. UPDATE is what lets it take the row lock of
-- a job or an item. Nothing is deleted by the role: retention runs through a bounded purge function.
REVOKE ALL PRIVILEGES ON batch_jobs, batch_items FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL PRIVILEGES ON batch_jobs, batch_items FROM sagat_app;
        GRANT SELECT, INSERT, UPDATE ON batch_jobs, batch_items TO sagat_app;
    END IF;
END $grants$;
