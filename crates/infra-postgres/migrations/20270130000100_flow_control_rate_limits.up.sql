-- Feature 022, release 0.3.0 slice (T022-070; FR-022-040, FR-022-056, FR-022-058; ADR-0088, ADR-0089): the durable window of
-- a rate-limited key, its grants and its buffered waiters (`specs/022-flow-control-step-policies/data-model.md`, "Rate
-- limits" as revised after the reviews of 2026-11-13).
--
-- A window row per key (tenant, namespace, pool, scope, group, keyed digest) is the key's lock: its grant and waiter rows
-- change only under it. A grant counts for an acquirer with window `per` while it is unleased (a step) and for `per` after
-- its lease instant (a run: its grant instant); the rows are the instants, so a window is strict whatever its period
-- (ADR-0088 decision 2). `waiters`, `live_grants`, `grants_total` and `max_period_ms` are kept by SECURITY DEFINER
-- triggers on the grant and waiter rows; the runtime role updates only `next_eligible_at` and `updated_at` of a window,
-- and only `leased_at` of a grant. A window that no grant and no waiter uses any more is deleted by the transaction that
-- empties it.
--
-- Additive: nothing existing reads or writes any of it; no build writes a row of it yet (D022-037).

CREATE TABLE rate_limit_windows (
    tenant_id        uuid        NOT NULL,
    namespace_id     uuid        NOT NULL,
    pool             text        NOT NULL,
    limit_scope      text        NOT NULL,
    group_name       text        NOT NULL,
    key_digest       bytea       NOT NULL,
    waiters          integer     NOT NULL DEFAULT 0,
    live_grants      integer     NOT NULL DEFAULT 0,
    -- every grant ever made under this row, for the fault suite's check against the journals
    grants_total     bigint      NOT NULL DEFAULT 0,
    -- the longest period among its grants and waiters: the purge keeps a grant for it plus one minute (FR-022-056)
    max_period_ms    integer     NOT NULL,
    key_version      smallint    NOT NULL REFERENCES flow_control_key_versions (version),
    key_shard        smallint    NOT NULL,
    next_eligible_at timestamptz,
    updated_at       timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest),
    CONSTRAINT rate_limit_windows_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT rate_limit_windows_pool_valid CHECK (pool IN ('ORDINARY', 'DRAFT_TEST')),
    CONSTRAINT rate_limit_windows_scope_valid CHECK (limit_scope IN ('RUN', 'STEP')),
    CONSTRAINT rate_limit_windows_group_valid CHECK (
        char_length(group_name) <= 130
        AND (group_name ~ '^[a-z][a-z0-9.-]{0,63}$'
             OR group_name ~ '^=[a-z][a-z0-9-]{0,63}(/[A-Za-z0-9][A-Za-z0-9_.-]{0,63})?$')),
    CONSTRAINT rate_limit_windows_digest_sha256 CHECK (octet_length(key_digest) = 32),
    CONSTRAINT rate_limit_windows_counts CHECK (waiters >= 0 AND live_grants >= 0 AND grants_total >= live_grants),
    CONSTRAINT rate_limit_windows_period_valid CHECK (max_period_ms IN (1000, 60000, 3600000)),
    CONSTRAINT rate_limit_windows_shard_valid CHECK (key_shard BETWEEN 0 AND 63)
);
CREATE INDEX rate_limit_windows_due_idx ON rate_limit_windows (key_shard, next_eligible_at) WHERE waiters > 0;
CREATE INDEX rate_limit_windows_key_version_idx ON rate_limit_windows (key_version);

CREATE TABLE rate_limit_grants (
    tenant_id      uuid        NOT NULL,
    namespace_id   uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    node           text        NOT NULL,
    attempt        integer     NOT NULL,
    pool           text        NOT NULL,
    limit_scope    text        NOT NULL,
    group_name     text        NOT NULL,
    key_digest     bytea       NOT NULL,
    declared_limit integer     NOT NULL,
    period_ms      integer     NOT NULL,
    granted_at     timestamptz NOT NULL,
    leased_at      timestamptz,
    shard          smallint    NOT NULL,
    PRIMARY KEY (tenant_id, run_id, node, attempt),
    CONSTRAINT rate_limit_grants_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT rate_limit_grants_window_fk FOREIGN KEY (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest)
        REFERENCES rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest),
    CONSTRAINT rate_limit_grants_scope_valid CHECK (limit_scope IN ('RUN', 'STEP')),
    -- a run's slot counts from its grant; a step's from its attempt's lease, and without end until it is recorded
    CONSTRAINT rate_limit_grants_shape CHECK (
        (limit_scope = 'RUN' AND node = '' AND attempt = 0 AND leased_at = granted_at)
        OR (limit_scope = 'STEP' AND node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$' AND attempt >= 1
            AND (leased_at IS NULL OR leased_at >= granted_at))),
    CONSTRAINT rate_limit_grants_limit_valid CHECK (declared_limit BETWEEN 1 AND 10000),
    CONSTRAINT rate_limit_grants_period_valid CHECK (period_ms IN (1000, 60000, 3600000)),
    CONSTRAINT rate_limit_grants_shard_valid CHECK (shard BETWEEN 0 AND 63)
);
CREATE INDEX rate_limit_grants_count_idx
    ON rate_limit_grants (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, leased_at);
CREATE INDEX rate_limit_grants_unleased_idx ON rate_limit_grants (shard) WHERE leased_at IS NULL;

CREATE TABLE rate_limit_waiters (
    tenant_id      uuid        NOT NULL,
    namespace_id   uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    node           text        NOT NULL,
    attempt        integer     NOT NULL,
    pool           text        NOT NULL,
    limit_scope    text        NOT NULL,
    group_name     text        NOT NULL,
    key_digest     bytea       NOT NULL,
    declared_limit integer     NOT NULL,
    period_ms      integer     NOT NULL,
    enqueued_at    timestamptz NOT NULL,
    deadline_at    timestamptz NOT NULL,
    shard          smallint    NOT NULL,
    PRIMARY KEY (tenant_id, run_id, node),
    CONSTRAINT rate_limit_waiters_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT rate_limit_waiters_window_fk FOREIGN KEY (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest)
        REFERENCES rate_limit_windows (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest),
    CONSTRAINT rate_limit_waiters_scope_valid CHECK (limit_scope IN ('RUN', 'STEP')),
    CONSTRAINT rate_limit_waiters_node_shape CHECK (
        (limit_scope = 'RUN' AND node = '' AND attempt = 0)
        OR (limit_scope = 'STEP' AND node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$' AND attempt >= 1)),
    CONSTRAINT rate_limit_waiters_limit_valid CHECK (declared_limit BETWEEN 1 AND 10000),
    CONSTRAINT rate_limit_waiters_period_valid CHECK (period_ms IN (1000, 60000, 3600000)),
    CONSTRAINT rate_limit_waiters_deadline_after CHECK (deadline_at > enqueued_at),
    CONSTRAINT rate_limit_waiters_shard_valid CHECK (shard BETWEEN 0 AND 63)
);
CREATE INDEX rate_limit_waiters_order_idx
    ON rate_limit_waiters (tenant_id, namespace_id, pool, limit_scope, group_name, key_digest, enqueued_at, run_id, node);
CREATE INDEX rate_limit_waiters_deadline_idx ON rate_limit_waiters (shard, deadline_at);

-- ---------------------------------------------------------------------------------------------------------------------
-- The counters of a window follow its grant and waiter rows, in the same transaction.
-- ---------------------------------------------------------------------------------------------------------------------
CREATE FUNCTION rate_limit_window_count() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    row_of  record;
    delta   integer := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    grant_r boolean := TG_TABLE_NAME = 'rate_limit_grants';
    changed integer;
BEGIN
    IF TG_OP = 'INSERT' THEN row_of := NEW; ELSE row_of := OLD; END IF;
    UPDATE public.rate_limit_windows w
       SET waiters = w.waiters + CASE WHEN grant_r THEN 0 ELSE delta END,
           live_grants = w.live_grants + CASE WHEN grant_r THEN delta ELSE 0 END,
           grants_total = w.grants_total + CASE WHEN grant_r AND TG_OP = 'INSERT' THEN 1 ELSE 0 END,
           max_period_ms = CASE WHEN TG_OP = 'INSERT' THEN greatest(w.max_period_ms, row_of.period_ms)
                                ELSE w.max_period_ms END,
           updated_at = clock_timestamp()
     WHERE w.tenant_id = row_of.tenant_id AND w.namespace_id = row_of.namespace_id AND w.pool = row_of.pool
       AND w.limit_scope = row_of.limit_scope AND w.group_name = row_of.group_name AND w.key_digest = row_of.key_digest;
    GET DIAGNOSTICS changed = ROW_COUNT;
    IF changed <> 1 THEN
        RAISE EXCEPTION 'a rate-limit grant or waiter has no window row'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    IF TG_OP = 'DELETE' THEN
        -- the purge decrements by the rows it actually deleted (FR-022-056), and the transaction that empties a key
        -- deletes it
        DELETE FROM public.rate_limit_windows w
         WHERE w.tenant_id = row_of.tenant_id AND w.namespace_id = row_of.namespace_id AND w.pool = row_of.pool
           AND w.limit_scope = row_of.limit_scope AND w.group_name = row_of.group_name
           AND w.key_digest = row_of.key_digest AND w.waiters = 0 AND w.live_grants = 0;
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION rate_limit_window_count() FROM PUBLIC;

-- A waiter is inserted or deleted, never changed; a grant changes only by the recording of its lease instant, once.
CREATE FUNCTION rate_limit_rows_guard() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF TG_TABLE_NAME = 'rate_limit_grants'
       AND OLD.leased_at IS NULL AND NEW.leased_at IS NOT NULL
       AND (NEW.tenant_id, NEW.namespace_id, NEW.run_id, NEW.node, NEW.attempt, NEW.pool, NEW.limit_scope,
            NEW.group_name, NEW.key_digest, NEW.declared_limit, NEW.period_ms, NEW.granted_at, NEW.shard)
           IS NOT DISTINCT FROM
           (OLD.tenant_id, OLD.namespace_id, OLD.run_id, OLD.node, OLD.attempt, OLD.pool, OLD.limit_scope,
            OLD.group_name, OLD.key_digest, OLD.declared_limit, OLD.period_ms, OLD.granted_at, OLD.shard) THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'a rate-limit waiter is never changed, and a grant only records its lease instant once'
        USING ERRCODE = 'restrict_violation';
END
$$;

CREATE TRIGGER rate_limit_grants_count AFTER INSERT OR DELETE ON rate_limit_grants
    FOR EACH ROW EXECUTE FUNCTION rate_limit_window_count();
CREATE TRIGGER rate_limit_waiters_count AFTER INSERT OR DELETE ON rate_limit_waiters
    FOR EACH ROW EXECUTE FUNCTION rate_limit_window_count();
CREATE TRIGGER rate_limit_grants_guard BEFORE UPDATE ON rate_limit_grants
    FOR EACH ROW EXECUTE FUNCTION rate_limit_rows_guard();
CREATE TRIGGER rate_limit_waiters_guard BEFORE UPDATE ON rate_limit_waiters
    FOR EACH ROW EXECUTE FUNCTION rate_limit_rows_guard();
ALTER TABLE rate_limit_grants ENABLE ALWAYS TRIGGER rate_limit_grants_count;
ALTER TABLE rate_limit_waiters ENABLE ALWAYS TRIGGER rate_limit_waiters_count;
ALTER TABLE rate_limit_grants ENABLE ALWAYS TRIGGER rate_limit_grants_guard;
ALTER TABLE rate_limit_waiters ENABLE ALWAYS TRIGGER rate_limit_waiters_guard;

-- The key rotation refusal of T022-007 covers the rate rows too (T022-070, ADR-0088 decision 8).
CREATE OR REPLACE FUNCTION flow_control_key_activate(p_version smallint, p_actor text)
RETURNS TABLE (out_outcome text, out_rows bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_rows bigint;
BEGIN
    LOCK TABLE public.flow_control_key_versions IN EXCLUSIVE MODE;
    LOCK TABLE public.concurrency_semaphores, public.rate_limit_windows IN SHARE MODE;
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'ACTIVE') THEN
        RETURN QUERY SELECT 'ALREADY_ACTIVE'::text, 0::bigint;
        RETURN;
    END IF;
    SELECT (SELECT count(*) FROM public.concurrency_semaphores WHERE key_version <> p_version)
         + (SELECT count(*) FROM public.rate_limit_windows WHERE key_version <> p_version)
      INTO v_rows;
    IF v_rows > 0 THEN
        RETURN QUERY SELECT 'ROWS_UNDER_OTHER_VERSION'::text, v_rows;
        RETURN;
    END IF;
    UPDATE public.flow_control_key_versions SET state = 'RETIRING' WHERE state = 'ACTIVE';
    INSERT INTO public.flow_control_key_versions (version, state, activated_at, activated_by)
    VALUES (p_version, 'ACTIVE', clock_timestamp(), p_actor)
    ON CONFLICT (version) DO UPDATE
        SET state = 'ACTIVE', activated_at = EXCLUDED.activated_at, activated_by = EXCLUDED.activated_by,
            retired_at = NULL;
    RETURN QUERY SELECT 'ACTIVATED'::text, 0::bigint;
END;
$$;
REVOKE ALL ON FUNCTION flow_control_key_activate(smallint, text) FROM PUBLIC;

REVOKE ALL PRIVILEGES ON rate_limit_windows, rate_limit_grants, rate_limit_waiters FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT ON rate_limit_windows TO sagat_app;
        GRANT UPDATE (next_eligible_at, updated_at) ON rate_limit_windows TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON rate_limit_grants TO sagat_app;
        GRANT UPDATE (leased_at) ON rate_limit_grants TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON rate_limit_waiters TO sagat_app;
    END IF;
END
$grants$;
