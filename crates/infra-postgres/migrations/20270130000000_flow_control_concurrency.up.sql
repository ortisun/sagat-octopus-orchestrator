-- Feature 022, release 0.3.0 slice (T022-009, T022-007; FR-022-020, FR-022-053; ADR-0056, ADR-0089): the durable
-- permits of concurrency keys, and the deployment's versions of the key that digests a key value.
--
-- `specs/022-flow-control-step-policies/data-model.md`, "Concurrency" as revised after the reviews of 2026-11-13: a
-- semaphore row per key (tenant, namespace, pool, scope, group, keyed digest), the holders of its permits and its
-- waiters, run or step (`step_concurrency_requirements` of the first draft is not built: a step waiting for a permit is
-- a waiter row with its node). Every foreign key to `runs` names the namespace (`runs_tenant_namespace_id_uniq`).
--
-- The counters of a semaphore (`holders`, `waiters`) are kept by SECURITY DEFINER triggers on the holder and waiter
-- rows, in the transaction that changes them, and a semaphore that no holder and no waiter uses any more is deleted by
-- the transaction that empties it. The runtime role may insert a semaphore (to lock it), and update only its
-- `next_eligible_at` and `updated_at` (which is also what `SELECT ... FOR UPDATE` needs), never a counter.
--
-- Additive: no existing table is altered and nothing existing reads or writes any of it. No build writes a row of it
-- yet (D022-037: a gated definition is refused at its admission until the gate is built).

-- The versions of the flow-control digest key (FR-022-053: a database row, never a per-process setting). The key
-- material is configuration and never a row. At most one version is ACTIVE. Changed only by
-- `flow_control_key_activate`, which the runtime role cannot execute.
CREATE TABLE flow_control_key_versions (
    version      smallint    PRIMARY KEY,
    state        text        NOT NULL,
    activated_at timestamptz NOT NULL,
    activated_by text        NOT NULL,
    retired_at   timestamptz,
    CONSTRAINT flow_control_key_versions_version_positive CHECK (version >= 1),
    CONSTRAINT flow_control_key_versions_state_valid CHECK (state IN ('ACTIVE', 'RETIRING', 'RETIRED')),
    CONSTRAINT flow_control_key_versions_retired CHECK ((state = 'RETIRED') = (retired_at IS NOT NULL)),
    CONSTRAINT flow_control_key_versions_by_bounded CHECK (char_length(activated_by) BETWEEN 1 AND 255)
);
CREATE UNIQUE INDEX flow_control_key_versions_one_active ON flow_control_key_versions ((true)) WHERE state = 'ACTIVE';

-- One row per key. `key_shard` is the shard whose owner grants the key (ADR-0089 decision 3); `next_eligible_at` a hint
-- kept under the key's lock, early and never late.
CREATE TABLE concurrency_semaphores (
    tenant_id        uuid        NOT NULL,
    namespace_id     uuid        NOT NULL,
    pool             text        NOT NULL,
    permit_scope     text        NOT NULL,
    group_name       text        NOT NULL,
    key_digest       bytea       NOT NULL,
    holders          integer     NOT NULL DEFAULT 0,
    waiters          integer     NOT NULL DEFAULT 0,
    key_version      smallint    NOT NULL REFERENCES flow_control_key_versions (version),
    key_shard        smallint    NOT NULL,
    next_eligible_at timestamptz,
    updated_at       timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest),
    CONSTRAINT concurrency_semaphores_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id),
    CONSTRAINT concurrency_semaphores_pool_valid CHECK (pool IN ('ORDINARY', 'DRAFT_TEST')),
    CONSTRAINT concurrency_semaphores_scope_valid CHECK (permit_scope IN ('RUN', 'STEP')),
    CONSTRAINT concurrency_semaphores_group_valid CHECK (
        char_length(group_name) <= 130
        AND (group_name ~ '^[a-z][a-z0-9.-]{0,63}$'
             OR group_name ~ '^=[a-z][a-z0-9-]{0,63}(/[A-Za-z0-9][A-Za-z0-9_.-]{0,63})?$')),
    CONSTRAINT concurrency_semaphores_digest_sha256 CHECK (octet_length(key_digest) = 32),
    CONSTRAINT concurrency_semaphores_counts CHECK (holders >= 0 AND waiters >= 0),
    CONSTRAINT concurrency_semaphores_shard_valid CHECK (key_shard BETWEEN 0 AND 63)
);
CREATE INDEX concurrency_semaphores_due_idx ON concurrency_semaphores (key_shard, next_eligible_at)
    WHERE waiters > 0;
CREATE INDEX concurrency_semaphores_key_version_idx ON concurrency_semaphores (key_version);

-- Who holds a permit: a run (its start), or one attempt of one step (granted before it is READY, ADR-0089 decision 1).
CREATE TABLE concurrency_holders (
    tenant_id      uuid        NOT NULL,
    namespace_id   uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    holder_scope   text        NOT NULL,
    node           text        NOT NULL,
    attempt        integer     NOT NULL,
    pool           text        NOT NULL,
    group_name     text        NOT NULL,
    key_digest     bytea       NOT NULL,
    declared_limit integer     NOT NULL,
    acquired_at    timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, run_id, holder_scope, node),
    CONSTRAINT concurrency_holders_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT concurrency_holders_semaphore_fk FOREIGN KEY (tenant_id, namespace_id, pool, holder_scope, group_name, key_digest)
        REFERENCES concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest),
    CONSTRAINT concurrency_holders_scope_valid CHECK (holder_scope IN ('RUN', 'STEP')),
    -- a run's permit is the run's own; a step's names its node and the attempt it is for
    CONSTRAINT concurrency_holders_node_shape CHECK (
        (holder_scope = 'RUN' AND node = '' AND attempt = 0)
        OR (holder_scope = 'STEP' AND node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$' AND attempt >= 1)),
    CONSTRAINT concurrency_holders_limit_valid CHECK (declared_limit BETWEEN 1 AND 10000)
);
CREATE INDEX concurrency_holders_key_idx
    ON concurrency_holders (tenant_id, namespace_id, pool, holder_scope, group_name, key_digest);

-- Who waits, oldest first by the commit that queued it (ties by run, then node).
CREATE TABLE concurrency_waiters (
    tenant_id      uuid        NOT NULL,
    namespace_id   uuid        NOT NULL,
    run_id         uuid        NOT NULL,
    node           text        NOT NULL,
    permit_scope   text        NOT NULL,
    attempt        integer     NOT NULL,
    pool           text        NOT NULL,
    group_name     text        NOT NULL,
    key_digest     bytea       NOT NULL,
    declared_limit integer     NOT NULL,
    enqueued_at    timestamptz NOT NULL,
    deadline_at    timestamptz NOT NULL,
    shard          smallint    NOT NULL,
    PRIMARY KEY (tenant_id, run_id, node),
    CONSTRAINT concurrency_waiters_run_fk FOREIGN KEY (tenant_id, namespace_id, run_id)
        REFERENCES runs (tenant_id, namespace_id, id),
    CONSTRAINT concurrency_waiters_semaphore_fk FOREIGN KEY (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest)
        REFERENCES concurrency_semaphores (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest),
    CONSTRAINT concurrency_waiters_scope_valid CHECK (permit_scope IN ('RUN', 'STEP')),
    CONSTRAINT concurrency_waiters_node_shape CHECK (
        (permit_scope = 'RUN' AND node = '' AND attempt = 0)
        OR (permit_scope = 'STEP' AND node ~ '^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$' AND attempt >= 1)),
    CONSTRAINT concurrency_waiters_limit_valid CHECK (declared_limit BETWEEN 1 AND 10000),
    CONSTRAINT concurrency_waiters_deadline_after CHECK (deadline_at > enqueued_at),
    CONSTRAINT concurrency_waiters_shard_valid CHECK (shard BETWEEN 0 AND 63)
);
CREATE INDEX concurrency_waiters_order_idx
    ON concurrency_waiters (tenant_id, namespace_id, pool, permit_scope, group_name, key_digest, enqueued_at, run_id, node);
CREATE INDEX concurrency_waiters_deadline_idx ON concurrency_waiters (shard, deadline_at);

-- ---------------------------------------------------------------------------------------------------------------------
-- The counters of a semaphore follow its holder and waiter rows, in the same transaction. SECURITY DEFINER with a pinned
-- path: the runtime role writes the rows, never a counter; every name is qualified.
-- ---------------------------------------------------------------------------------------------------------------------

CREATE FUNCTION concurrency_semaphore_count() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    holder_delta integer := 0;
    waiter_delta integer := 0;
    k_tenant uuid; k_namespace uuid; k_pool text; k_scope text; k_group text; k_digest bytea;
    changed integer;
BEGIN
    IF TG_TABLE_NAME = 'concurrency_holders' THEN
        holder_delta := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    ELSE
        waiter_delta := CASE TG_OP WHEN 'INSERT' THEN 1 ELSE -1 END;
    END IF;
    -- a field is read only from the table that has it: PL/pgSQL resolves every field an expression names
    IF TG_OP = 'INSERT' THEN
        k_tenant := NEW.tenant_id; k_namespace := NEW.namespace_id; k_pool := NEW.pool; k_group := NEW.group_name;
        k_digest := NEW.key_digest;
        IF TG_TABLE_NAME = 'concurrency_holders' THEN
            k_scope := NEW.holder_scope;
        ELSE
            k_scope := NEW.permit_scope;
        END IF;
    ELSE
        k_tenant := OLD.tenant_id; k_namespace := OLD.namespace_id; k_pool := OLD.pool; k_group := OLD.group_name;
        k_digest := OLD.key_digest;
        IF TG_TABLE_NAME = 'concurrency_holders' THEN
            k_scope := OLD.holder_scope;
        ELSE
            k_scope := OLD.permit_scope;
        END IF;
    END IF;
    UPDATE public.concurrency_semaphores s
       SET holders = s.holders + holder_delta,
           waiters = s.waiters + waiter_delta,
           updated_at = clock_timestamp()
     WHERE s.tenant_id = k_tenant AND s.namespace_id = k_namespace AND s.pool = k_pool AND s.permit_scope = k_scope
       AND s.group_name = k_group AND s.key_digest = k_digest;
    GET DIAGNOSTICS changed = ROW_COUNT;
    IF changed <> 1 THEN
        RAISE EXCEPTION 'a concurrency holder or waiter has no semaphore row'
            USING ERRCODE = 'foreign_key_violation';
    END IF;
    IF TG_OP = 'DELETE' THEN
        -- the transaction that empties a key deletes it
        DELETE FROM public.concurrency_semaphores s
         WHERE s.tenant_id = k_tenant AND s.namespace_id = k_namespace AND s.pool = k_pool AND s.permit_scope = k_scope
           AND s.group_name = k_group AND s.key_digest = k_digest AND s.holders = 0 AND s.waiters = 0;
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION concurrency_semaphore_count() FROM PUBLIC;

-- A holder or waiter is inserted or deleted, never changed: the key a run waits for or holds is fixed when it is decided.
CREATE FUNCTION concurrency_rows_immutable() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'a concurrency holder or waiter is inserted or deleted, never changed'
        USING ERRCODE = 'restrict_violation';
END
$$;

CREATE TRIGGER concurrency_holders_count AFTER INSERT OR DELETE ON concurrency_holders
    FOR EACH ROW EXECUTE FUNCTION concurrency_semaphore_count();
CREATE TRIGGER concurrency_waiters_count AFTER INSERT OR DELETE ON concurrency_waiters
    FOR EACH ROW EXECUTE FUNCTION concurrency_semaphore_count();
CREATE TRIGGER concurrency_holders_immutable BEFORE UPDATE ON concurrency_holders
    FOR EACH ROW EXECUTE FUNCTION concurrency_rows_immutable();
CREATE TRIGGER concurrency_waiters_immutable BEFORE UPDATE ON concurrency_waiters
    FOR EACH ROW EXECUTE FUNCTION concurrency_rows_immutable();
ALTER TABLE concurrency_holders ENABLE ALWAYS TRIGGER concurrency_holders_count;
ALTER TABLE concurrency_waiters ENABLE ALWAYS TRIGGER concurrency_waiters_count;
ALTER TABLE concurrency_holders ENABLE ALWAYS TRIGGER concurrency_holders_immutable;
ALTER TABLE concurrency_waiters ENABLE ALWAYS TRIGGER concurrency_waiters_immutable;

-- ---------------------------------------------------------------------------------------------------------------------
-- The operator's command (T022-007, ADR-0056 decision 7): a new key version is ACTIVE, the one before is RETIRING; it is
-- refused while any flow-control row of a key still uses another version. Per-key rotation, which lets a rotation run
-- with live rows, is T022-095 (FR-022-056). NOT executable by the runtime role.
--
-- outcome: ACTIVATED | ALREADY_ACTIVE | ROWS_UNDER_OTHER_VERSION (out_rows names how many)
-- ---------------------------------------------------------------------------------------------------------------------
CREATE FUNCTION flow_control_key_activate(p_version smallint, p_actor text)
RETURNS TABLE (out_outcome text, out_rows bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_rows bigint;
BEGIN
    -- one rotation at a time, and no row of a key is written by a run that is admitted meanwhile under the old version
    LOCK TABLE public.flow_control_key_versions IN EXCLUSIVE MODE;
    LOCK TABLE public.concurrency_semaphores IN SHARE MODE;
    IF EXISTS (SELECT 1 FROM public.flow_control_key_versions WHERE version = p_version AND state = 'ACTIVE') THEN
        RETURN QUERY SELECT 'ALREADY_ACTIVE'::text, 0::bigint;
        RETURN;
    END IF;
    SELECT count(*) INTO v_rows FROM public.concurrency_semaphores WHERE key_version <> p_version;
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

-- ---------------------------------------------------------------------------------------------------------------------
-- Grants (`data-model.md`, "Grants and counters").
-- ---------------------------------------------------------------------------------------------------------------------
REVOKE ALL PRIVILEGES ON flow_control_key_versions, concurrency_semaphores, concurrency_holders, concurrency_waiters
    FROM PUBLIC;
DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT ON flow_control_key_versions TO sagat_app;
        GRANT SELECT, INSERT ON concurrency_semaphores TO sagat_app;
        GRANT UPDATE (next_eligible_at, updated_at) ON concurrency_semaphores TO sagat_app;
        GRANT SELECT, INSERT, DELETE ON concurrency_holders, concurrency_waiters TO sagat_app;
    END IF;
END
$grants$;
