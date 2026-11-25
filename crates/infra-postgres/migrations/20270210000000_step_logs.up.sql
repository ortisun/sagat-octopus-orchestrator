-- Feature 041 (step logs), task T041-5: the two step-log tables (specs/041-step-logs/data-model.md §2), the rest of
-- the accounting of ADR-0027's Amendment of 2026-11-15 and its Amendment of 2026-11-20 (§3; OD-2026-11-15-16,
-- OD-2026-11-20-16), the exact quota leases of ADR-0097 (§3b; OD-2026-11-20-15, OD-2026-11-20-18, OD-2026-11-21-9)
-- and the grants of §4.
--
-- Additive: older readers never read the new tables or columns. `sagat_quota_add_delta` is dropped and created again
-- with a fifth argument that defaults to 0, so that the present four-argument calls keep working; every function this
-- file creates or replaces keeps or has `SECURITY DEFINER` (where it had it), its pinned `search_path` and its grants.
--
-- The one CHECK added to an existing table (`tenant_retained_usage_step_log_leased_nonnegative`) is added `NOT VALID`
-- and validated by 20270210000100_step_logs_validate, so that no table is scanned under the `ACCESS EXCLUSIVE` lock of
-- the `ADD` (design 043, D043-145, D043-153). The CHECKs of the two new tables are plain: no row exists to validate.

-- ---------------------------------------------------------------------------------------------------------------------
-- §2. The tables.

CREATE TABLE step_log_streams (
    tenant_id             uuid        NOT NULL,
    stream_id             bigint      GENERATED ALWAYS AS IDENTITY,
    public_id             uuid        NOT NULL,
    namespace_id          uuid        NOT NULL,
    run_id                uuid        NOT NULL,
    step_id               uuid,
    node                  text        NOT NULL,
    instance_path_nodes   text[],
    instance_path_indices integer[],
    attempt               integer     NOT NULL,
    generation            bigint,
    source                text        NOT NULL DEFAULT 'worker',
    records               integer     NOT NULL DEFAULT 0,
    bytes                 bigint      NOT NULL DEFAULT 0,
    last_sequence         bigint      NOT NULL DEFAULT 0,
    ceiling_sequence      bigint,
    next_marker_sequence  bigint      NOT NULL DEFAULT 4611686018427387904,
    dropped_by_worker     bigint      NOT NULL DEFAULT 0,
    dropped_by_server     bigint      NOT NULL DEFAULT 0,
    ceiling_reason        text,
    not_stored_reason     text,
    truncated_records     integer     NOT NULL DEFAULT 0,
    retention_days        smallint    NOT NULL,
    last_stored_xid       xid8,
    final_at              timestamptz,
    first_at              timestamptz,
    last_at               timestamptz NOT NULL,
    expires_at            timestamptz NOT NULL,
    -- Whole days since the Unix epoch (UTC) of `expires_at`: the expiry index's key, so that an update that keeps the
    -- day changes no indexed column and stays HOT (D041-111). No time zone enters the expression, so it is immutable.
    expires_day           integer     GENERATED ALWAYS AS
        ((floor(extract(epoch FROM (expires_at - '1970-01-01 00:00:00+00'::timestamptz)) / 86400))::integer) STORED,
    deleted_at            timestamptz,
    removed_at            timestamptz,
    purged_at             timestamptz,
    PRIMARY KEY (tenant_id, stream_id),
    CONSTRAINT step_log_streams_public_uniq UNIQUE (tenant_id, public_id),
    CONSTRAINT step_log_streams_attempt_uniq UNIQUE (tenant_id, run_id, step_id, attempt, generation, source),
    CONSTRAINT step_log_streams_source_valid CHECK (source IN ('worker', 'script')),
    CONSTRAINT step_log_streams_source_shape CHECK (
        (source = 'worker' AND step_id IS NOT NULL AND generation IS NOT NULL
            AND instance_path_nodes IS NOT NULL AND instance_path_indices IS NOT NULL)
        OR (source = 'script' AND step_id IS NULL AND generation IS NULL
            AND instance_path_nodes IS NULL AND instance_path_indices IS NULL)),
    CONSTRAINT step_log_streams_attempt_valid CHECK (attempt >= 1 AND (generation IS NULL OR generation >= 0)),
    CONSTRAINT step_log_streams_counters_valid CHECK (records >= 0 AND bytes >= 0 AND last_sequence >= 0
        AND dropped_by_worker BETWEEN 0 AND 9007199254740992 AND dropped_by_server >= 0),
    CONSTRAINT step_log_streams_ceiling_valid CHECK (ceiling_reason IS NULL OR ceiling_reason IN
        ('attempt_records', 'attempt_bytes', 'tenant_quota')),
    CONSTRAINT step_log_streams_not_stored_valid CHECK (not_stored_reason IS NULL
        OR not_stored_reason IN ('service_busy')),
    CONSTRAINT step_log_streams_path_valid CHECK (instance_path_nodes IS NULL
        OR execution_instance_path_valid(instance_path_nodes, instance_path_indices)),
    CONSTRAINT step_log_streams_retention_valid CHECK (retention_days BETWEEN 1 AND 365),
    CONSTRAINT step_log_streams_removal_order CHECK ((deleted_at IS NULL OR removed_at IS NOT NULL)
        AND (purged_at IS NULL OR removed_at IS NOT NULL))
) WITH (fillfactor = 70);
CREATE INDEX step_log_streams_run_idx ON step_log_streams (tenant_id, run_id, stream_id);
CREATE INDEX step_log_streams_expiry_idx ON step_log_streams (expires_day) WHERE removed_at IS NULL;
CREATE INDEX step_log_streams_removal_idx ON step_log_streams (tenant_id, stream_id)
    WHERE removed_at IS NOT NULL AND purged_at IS NULL;

-- What a stream is never changes, its last sequence only rises, and a final mark, a deletion, a removal, a purge and the
-- order guard are never undone (data-model.md §2, design section 2a).
CREATE FUNCTION step_log_streams_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
        OR NEW.stream_id IS DISTINCT FROM OLD.stream_id
        OR NEW.public_id IS DISTINCT FROM OLD.public_id
        OR NEW.namespace_id IS DISTINCT FROM OLD.namespace_id
        OR NEW.run_id IS DISTINCT FROM OLD.run_id
        OR NEW.step_id IS DISTINCT FROM OLD.step_id
        OR NEW.node IS DISTINCT FROM OLD.node
        OR NEW.instance_path_nodes IS DISTINCT FROM OLD.instance_path_nodes
        OR NEW.instance_path_indices IS DISTINCT FROM OLD.instance_path_indices
        OR NEW.attempt IS DISTINCT FROM OLD.attempt
        OR NEW.generation IS DISTINCT FROM OLD.generation
        OR NEW.source IS DISTINCT FROM OLD.source
        OR NEW.retention_days IS DISTINCT FROM OLD.retention_days
        OR NEW.last_sequence < OLD.last_sequence
        OR (OLD.final_at IS NOT NULL AND NEW.final_at IS NULL)
        OR (OLD.deleted_at IS NOT NULL AND NEW.deleted_at IS NULL)
        OR (OLD.removed_at IS NOT NULL AND NEW.removed_at IS NULL)
        OR (OLD.purged_at IS NOT NULL AND NEW.purged_at IS NULL)
        OR (OLD.last_stored_xid IS NOT NULL AND NEW.last_stored_xid IS NULL)
        OR NEW.last_stored_xid < OLD.last_stored_xid THEN
        RAISE EXCEPTION 'what a step-log stream is, and what it has recorded, never goes back'
            USING ERRCODE = 'restrict_violation', CONSTRAINT = 'step_log_streams_guard';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION step_log_streams_guard() FROM PUBLIC;
CREATE TRIGGER step_log_streams_guard BEFORE UPDATE ON step_log_streams
    FOR EACH ROW EXECUTE FUNCTION step_log_streams_guard();
ALTER TABLE step_log_streams ENABLE ALWAYS TRIGGER step_log_streams_guard;

CREATE TABLE step_log_lines (
    tenant_id     uuid        NOT NULL,
    stream_id     bigint      NOT NULL,
    sequence      bigint      NOT NULL,
    run_id        uuid        NOT NULL,
    stored_xid    xid8        NOT NULL DEFAULT pg_current_xact_id(),
    stored_at     timestamptz NOT NULL DEFAULT clock_timestamp(),
    worker_time   timestamptz,
    level         text        NOT NULL,
    kind          text        NOT NULL,
    truncated     boolean     NOT NULL DEFAULT false,
    message       text,
    exception     text,
    fields        text,
    marker_reason text,
    script_execution_id uuid,
    PRIMARY KEY (tenant_id, stream_id, sequence),
    CONSTRAINT step_log_lines_stream_fk FOREIGN KEY (tenant_id, stream_id)
        REFERENCES step_log_streams (tenant_id, stream_id),
    CONSTRAINT step_log_lines_level_valid CHECK (level IN ('trace', 'debug', 'info', 'warn', 'error')),
    CONSTRAINT step_log_lines_kind_valid CHECK (
        (kind = 'line' AND marker_reason IS NULL AND script_execution_id IS NULL AND sequence < 4611686018427387904)
        OR (kind = 'ceiling_marker' AND message IS NULL AND exception IS NULL AND fields IS NULL
            AND marker_reason IN ('attempt_records', 'attempt_bytes') AND script_execution_id IS NULL
            AND sequence >= 4611686018427387904)
        OR (kind = 'script_marker' AND message IS NULL AND exception IS NULL AND fields IS NULL
            AND marker_reason IS NULL AND script_execution_id IS NOT NULL
            AND sequence >= 4611686018427387904)),
    CONSTRAINT step_log_lines_sizes CHECK (octet_length(coalesce(message, '')) <= 8192
        AND octet_length(coalesce(exception, '')) <= 8192 AND octet_length(coalesce(fields, '')) <= 2048)
);
CREATE INDEX step_log_lines_read_idx ON step_log_lines (tenant_id, run_id, stored_xid, sequence);
CREATE INDEX step_log_lines_stream_idx ON step_log_lines (tenant_id, stream_id, stored_xid, sequence);

-- Lines are never changed; a `DELETE` is the purge's.
CREATE FUNCTION step_log_lines_append_only() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    RAISE EXCEPTION 'a stored step-log line never changes'
        USING ERRCODE = 'restrict_violation', CONSTRAINT = 'step_log_lines_append_only';
END
$$;
REVOKE ALL ON FUNCTION step_log_lines_append_only() FROM PUBLIC;
CREATE TRIGGER step_log_lines_append_only BEFORE UPDATE ON step_log_lines
    FOR EACH ROW EXECUTE FUNCTION step_log_lines_append_only();
ALTER TABLE step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_append_only;

-- ---------------------------------------------------------------------------------------------------------------------
-- §3. The accounting of the class `step_log` (ADR-0027's Amendment items 3 and 4, as its Amendment of 2026-11-20 and
-- ADR-0097 change them).

ALTER TABLE quota_transaction_deltas ADD COLUMN step_log_delta bigint NOT NULL DEFAULT 0;
-- A step log has no terminal fallback to wire first, so it is enforced from this migration on (Amendment item 3).
ALTER TABLE quota_accounting_control ADD COLUMN step_log_enforcement_enabled boolean NOT NULL DEFAULT true;

-- `fixed_octets`: a constant charge per row whatever the value, 128 bytes (Amendment item 4, OD-2026-11-15-6), allowed
-- only on a `NOT NULL` column (`step_log_lines.level`), so that the manifest test compares it.
CREATE OR REPLACE FUNCTION sagat_quota_measure(
    value jsonb, measurement text, column_name text, value_is_sql_null boolean
)
RETURNS bigint LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, pg_temp AS $$
DECLARE encoded text;
BEGIN
    IF value_is_sql_null OR value -> column_name IS NULL THEN
        RETURN 0;
    END IF;
    IF measurement = 'octet_length_utf8' THEN
        RETURN octet_length(value ->> column_name);
    ELSIF measurement = 'octet_length_bytea' THEN
        encoded := value ->> column_name;
        IF left(encoded, 2) <> '\x' THEN
            RAISE EXCEPTION 'unexpected bytea encoding in quota measurement';
        END IF;
        RETURN (length(encoded) - 2) / 2;
    ELSIF measurement IN ('canonical_json_utf8', 'canonical_json_array_utf8') THEN
        RETURN octet_length((value -> column_name)::text);
    ELSIF measurement = 'fixed_octets' THEN
        RETURN 128;
    END IF;
    RAISE EXCEPTION 'unknown quota measurement %', measurement;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_measure(jsonb, text, text, boolean) FROM PUBLIC;

-- The step-log delta as a fifth argument with a default of 0: `CREATE OR REPLACE` with a new argument would create a
-- second function beside the four-argument one and make the present four-argument calls ambiguous.
DROP FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint);
CREATE FUNCTION sagat_quota_add_delta(tenant uuid, content bigint, reserved bigint, logs bigint,
                                      step_logs bigint DEFAULT 0)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    INSERT INTO public.quota_transaction_deltas
        (transaction_id, tenant_id, content_delta, content_reserved_delta, log_delta, step_log_delta)
    VALUES (pg_current_xact_id(), tenant, content, reserved, logs, step_logs)
    ON CONFLICT (transaction_id, tenant_id) DO UPDATE SET
        content_delta = public.quota_transaction_deltas.content_delta + EXCLUDED.content_delta,
        content_reserved_delta = public.quota_transaction_deltas.content_reserved_delta
                                 + EXCLUDED.content_reserved_delta,
        log_delta = public.quota_transaction_deltas.log_delta + EXCLUDED.log_delta,
        step_log_delta = public.quota_transaction_deltas.step_log_delta + EXCLUDED.step_log_delta
$$;
REVOKE ALL ON FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint, bigint) FROM PUBLIC;

-- The class `step_log` goes to its own delta; a class the function does not know raises (Amendment item 3).
CREATE OR REPLACE FUNCTION sagat_quota_row_delta() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    old_row jsonb := CASE WHEN TG_OP = 'INSERT' THEN NULL ELSE to_jsonb(OLD) END;
    new_row jsonb := CASE WHEN TG_OP = 'DELETE' THEN NULL ELSE to_jsonb(NEW) END;
    old_tenant uuid; new_tenant uuid;
    old_content bigint := 0; new_content bigint := 0;
    old_logs bigint := 0; new_logs bigint := 0;
    old_step_logs bigint := 0; new_step_logs bigint := 0;
    i integer := 0; measured bigint;
    old_is_null boolean; new_is_null boolean;
BEGIN
    IF old_row IS NOT NULL THEN old_tenant := (old_row ->> 'tenant_id')::uuid; END IF;
    IF new_row IS NOT NULL THEN new_tenant := (new_row ->> 'tenant_id')::uuid; END IF;
    WHILE i < TG_NARGS LOOP
        IF TG_ARGV[i] NOT IN ('content', 'log', 'step_log') THEN
            RAISE EXCEPTION 'unknown quota class %', TG_ARGV[i];
        END IF;
        -- State-only updates are common on workers and steps. Compare the already materialized
        -- JSON values first so they never enter SPI or enqueue a tenant usage delta.
        IF old_row IS NOT NULL AND new_row IS NOT NULL
           AND old_tenant IS NOT DISTINCT FROM new_tenant
           AND old_row -> TG_ARGV[i + 2] IS NOT DISTINCT FROM new_row -> TG_ARGV[i + 2]
           AND NOT (TG_ARGV[i + 1] = 'canonical_json_utf8'
                    AND old_row -> TG_ARGV[i + 2] = 'null'::jsonb) THEN
            i := i + 3;
            CONTINUE;
        END IF;
        old_is_null := old_row IS NULL OR old_row -> TG_ARGV[i + 2] = 'null'::jsonb;
        new_is_null := new_row IS NULL OR new_row -> TG_ARGV[i + 2] = 'null'::jsonb;
        -- Only a json/jsonb column can hold a JSON literal null that is distinct from SQL NULL.
        -- Dynamic record inspection is therefore confined to this ambiguous, uncommon value.
        IF old_row IS NOT NULL AND old_is_null
           AND TG_ARGV[i + 1] = 'canonical_json_utf8' THEN
            EXECUTE format('SELECT ($1).%I IS NULL', TG_ARGV[i + 2])
               INTO old_is_null USING OLD;
        END IF;
        IF new_row IS NOT NULL AND new_is_null
           AND TG_ARGV[i + 1] = 'canonical_json_utf8' THEN
            EXECUTE format('SELECT ($1).%I IS NULL', TG_ARGV[i + 2])
               INTO new_is_null USING NEW;
        END IF;
        IF old_row IS NOT NULL THEN
            measured := public.sagat_quota_measure(
                old_row, TG_ARGV[i + 1], TG_ARGV[i + 2], old_is_null);
            IF TG_ARGV[i] = 'content' THEN old_content := old_content + measured;
            ELSIF TG_ARGV[i] = 'log' THEN old_logs := old_logs + measured;
            ELSE old_step_logs := old_step_logs + measured; END IF;
        END IF;
        IF new_row IS NOT NULL THEN
            measured := public.sagat_quota_measure(
                new_row, TG_ARGV[i + 1], TG_ARGV[i + 2], new_is_null);
            IF TG_ARGV[i] = 'content' THEN new_content := new_content + measured;
            ELSIF TG_ARGV[i] = 'log' THEN new_logs := new_logs + measured;
            ELSE new_step_logs := new_step_logs + measured; END IF;
        END IF;
        i := i + 3;
    END LOOP;
    IF old_tenant IS NOT DISTINCT FROM new_tenant THEN
        IF new_content <> old_content OR new_logs <> old_logs OR new_step_logs <> old_step_logs THEN
            PERFORM public.sagat_quota_add_delta(
                new_tenant, new_content - old_content, 0, new_logs - old_logs, new_step_logs - old_step_logs);
        END IF;
    ELSE
        IF old_tenant IS NOT NULL THEN
            PERFORM public.sagat_quota_add_delta(old_tenant, -old_content, 0, -old_logs, -old_step_logs);
        END IF;
        IF new_tenant IS NOT NULL THEN
            PERFORM public.sagat_quota_add_delta(new_tenant, new_content, 0, new_logs, new_step_logs);
        END IF;
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_row_delta() FROM PUBLIC;

-- ---------------------------------------------------------------------------------------------------------------------
-- §3b. The exact quota leases (ADR-0097).

-- G: the room granted to the tenant's quota leases. S (`step_log_bytes`) may be negative while released bytes were
-- charged in quota leases; the usage U = S + Σ used is kept non-negative by every release (D041-112, D041-113).
ALTER TABLE tenant_retained_usage ADD COLUMN step_log_leased_bytes bigint NOT NULL DEFAULT 0;
ALTER TABLE tenant_retained_usage ADD CONSTRAINT tenant_retained_usage_step_log_leased_nonnegative
    CHECK (step_log_leased_bytes >= 0) NOT VALID;

-- One quota lease per tenant and server backend that charges for it (D041-117): `writer` is `pg_backend_pid()`. A
-- backend runs one transaction at a time, so no writer ever meets another's lock on its quota lease. No index besides
-- the primary key: every charge sets `charged_at`, which an index would make a non-HOT update.
-- No foreign key to the usage row (decision D041-143, amending ADR-0097 decision 1): PostgreSQL checks a foreign key
-- on an update that keeps the key when the row version was written by the updating transaction, so a second charge in
-- one transaction would take `FOR KEY SHARE` on the usage row and wait for a refill holding it. A quota lease is
-- inserted only by a refill or a top-up holding the tenant's usage row `FOR UPDATE`, so its row exists then and cannot
-- be deleted meanwhile; the cascade is the trigger `tenant_retained_usage_step_log_leases_cascade` below.
CREATE TABLE step_log_quota_leases (
    tenant_id  uuid        NOT NULL,
    writer     integer     NOT NULL,
    granted    bigint      NOT NULL DEFAULT 0,
    used       bigint      NOT NULL DEFAULT 0,
    charged_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, writer),
    CONSTRAINT step_log_quota_leases_used_within CHECK (0 <= used AND used <= granted)
) WITH (fillfactor = 50, autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);

-- The cascade of a tenant's usage row (deleted with its tenant) to its quota leases.
CREATE FUNCTION sagat_quota_step_log_leases_cascade() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    DELETE FROM public.step_log_quota_leases WHERE tenant_id = OLD.tenant_id;
    RETURN NULL;
END
$$;
CREATE TRIGGER tenant_retained_usage_step_log_leases_cascade AFTER DELETE ON tenant_retained_usage
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_step_log_leases_cascade();
ALTER TABLE tenant_retained_usage ENABLE ALWAYS TRIGGER tenant_retained_usage_step_log_leases_cascade;

-- Events counted inside SQL advance sequences, never a counter row (a hot row again); a sequence is not
-- transactional, so these count attempts, rolled-back ones included (data-model.md §7).
CREATE SEQUENCE step_log_quota_refills_refill_seq;
CREATE SEQUENCE step_log_quota_refills_reclaim_seq;
CREATE SEQUENCE step_log_quota_refills_idle_seq;
CREATE SEQUENCE step_log_quota_refills_ended_seq;

-- The top-up mark M (decision D041-124): a writer's lease with this much room is not topped up.
CREATE FUNCTION sagat_quota_step_log_top_up_mark() RETURNS bigint
LANGUAGE sql IMMUTABLE SET search_path = pg_catalog, pg_temp AS $$ SELECT 65536::bigint $$;

-- Q, the replica's wait slots (decisions D041-134, D041-137): the transaction-local setting the store sends, clamped
-- to 1 to 64, and 1 when it is unset, empty or not a number. The tenant's slots are P = ⌈Q / 2⌉.
CREATE FUNCTION sagat_quota_step_log_wait_slots() RETURNS integer
LANGUAGE sql STABLE SET search_path = pg_catalog, pg_temp AS $$
    SELECT CASE WHEN v ~ '^[0-9]{1,9}$' THEN least(64, greatest(1, v::integer)) ELSE 1 END
      FROM (SELECT coalesce(current_setting('sagat.step_log_wait_slots', true), '') AS v) setting
$$;

-- The fallback refill's budget b for each lock acquisition on the usage row (D041-119): 200 ms, the budget an ingest
-- batch already has on the same row. A function, not a session setting, so that no caller can lengthen it.
CREATE FUNCTION sagat_quota_step_log_refill_wait() RETURNS interval
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT interval '200 milliseconds'
$$;

-- L: the tenant's limit, as the finalizer works out the other byte limits (`quota_limits`, else `quota_ceilings`,
-- else 1 GiB, bounded by the ceiling). Every caller reads it after the step-log fence it holds or tried.
CREATE FUNCTION sagat_quota_step_log_limit(p_tenant uuid) RETURNS bigint
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT least(
        coalesce((SELECT max_value FROM public.quota_limits
                   WHERE tenant_id = p_tenant AND namespace_id IS NULL AND resource = 'tenant_step_log_bytes'),
                 (SELECT max_value FROM public.quota_ceilings
                   WHERE tenant_id = p_tenant AND resource = 'tenant_step_log_bytes'),
                 1073741824),
        coalesce((SELECT max_value FROM public.quota_ceilings
                   WHERE tenant_id = p_tenant AND resource = 'tenant_step_log_bytes'),
                 1073741824))
$$;

-- U = S + Σ used, read without a lock in one snapshot. `STABLE`, so that the quota view, which calls it inside the
-- one `SELECT` that reads the grant (decision D041-141), reads the usage in that statement's snapshot: a `VOLATILE`
-- function's queries take snapshots of their own.
CREATE FUNCTION sagat_quota_step_log_usage(p_tenant uuid) RETURNS bigint
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT (coalesce((SELECT u.step_log_bytes FROM public.tenant_retained_usage u WHERE u.tenant_id = p_tenant), 0)
          + coalesce((SELECT sum(l.used) FROM public.step_log_quota_leases l WHERE l.tenant_id = p_tenant), 0))::bigint
$$;

-- The ingest's and the marker's early check (Amendment item 3): the usage, the limit and whether it is enforced,
-- without a lock (the runtime role may not read `quota_accounting_control`).
CREATE FUNCTION sagat_quota_step_log_allowance(p_tenant uuid)
RETURNS TABLE (used bigint, max_value bigint, enforced boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT public.sagat_quota_step_log_usage(p_tenant), public.sagat_quota_step_log_limit(p_tenant),
           c.step_log_enforcement_enabled
      FROM public.quota_accounting_control c WHERE c.singleton
$$;

-- The refill (ADR-0097 decision 4): only when the writer's own quota lease is short of the charge, or S + G > L.
-- Lock order: the class fences (held by the charge) -> the usage row (tried at once; waited for only while holding a
-- tenant wait slot and a replica wait slot, each acquisition within b) -> the writer's own quota lease `NOWAIT`:
-- nothing waits after the usage row. No exception block, so no subtransaction.
CREATE FUNCTION sagat_quota_step_log_refill(p_tenant uuid, p_bytes bigint, p_charge boolean DEFAULT true)
RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    v_limit bigint; v_enforced boolean; v_s bigint; v_g bigint; v_free bigint; v_grant bigint; v_others bigint;
    v_granted bigint; v_used bigint; v_lease record; v_previous text; v_bound interval;
    v_slots integer; v_slot integer; v_replica text; v_taken boolean; v_absent boolean;
BEGIN
    IF p_bytes IS NULL OR p_bytes < 0 THEN
        RAISE EXCEPTION 'a step-log refill takes a non-negative amount'
            USING ERRCODE = 'invalid_parameter_value', CONSTRAINT = 'step_log_refill_amount';
    END IF;
    -- The usage row, tried without waiting.
    SELECT step_log_bytes, step_log_leased_bytes INTO v_s, v_g
      FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE SKIP LOCKED;
    IF NOT FOUND THEN
        -- Held, or absent (its insert can wait for a concurrent inserter): wait only while holding one of the
        -- tenant's P slots and one of the replica's Q slots on this replica, both tried and never waited for.
        v_absent := NOT EXISTS (SELECT 1 FROM public.tenant_retained_usage WHERE tenant_id = p_tenant);
        v_slots := public.sagat_quota_step_log_wait_slots();
        v_replica := current_setting('application_name');
        v_taken := false;
        FOR v_slot IN 1 .. (v_slots + 1) / 2 LOOP
            IF pg_try_advisory_xact_lock(hashtextextended(
                   'sagat:step_log_wait:t:' || p_tenant::text || ':' || v_replica || ':' || v_slot::text, 0)) THEN
                v_taken := true;
                EXIT;
            END IF;
        END LOOP;
        IF v_taken THEN
            v_taken := false;
            FOR v_slot IN 1 .. v_slots LOOP
                IF pg_try_advisory_xact_lock(hashtextextended(
                       'sagat:step_log_wait:r:' || v_replica || ':' || v_slot::text, 0)) THEN
                    v_taken := true;
                    EXIT;
                END IF;
            END LOOP;
        END IF;
        IF NOT v_taken THEN
            RAISE EXCEPTION 'step-log wait slots taken'
                USING ERRCODE = 'lock_not_available', CONSTRAINT = 'step_log_wait_slots_taken';
        END IF;
        -- Each lock acquisition on the usage row waits at most b, unless the caller's own timeout is lower (a
        -- `lock_timeout` of 0 means none and is never lower); the caller's value is restored after it.
        v_previous := current_setting('lock_timeout');
        v_bound := public.sagat_quota_step_log_refill_wait();
        IF v_previous = '0' OR v_previous::interval > v_bound THEN
            PERFORM set_config('lock_timeout', (extract(epoch FROM v_bound) * 1000)::bigint::text || 'ms', true);
        END IF;
        IF v_absent THEN
            INSERT INTO public.tenant_retained_usage (tenant_id) VALUES (p_tenant) ON CONFLICT (tenant_id) DO NOTHING;
        END IF;
        SELECT step_log_bytes, step_log_leased_bytes INTO v_s, v_g
          FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE;
        v_absent := NOT FOUND;
        PERFORM set_config('lock_timeout', v_previous, true);
        IF v_absent THEN
            RAISE EXCEPTION 'the tenant has no step-log usage row'
                USING ERRCODE = 'foreign_key_violation', CONSTRAINT = 'step_log_usage_row_missing';
        END IF;
    END IF;
    PERFORM nextval('public.step_log_quota_refills_refill_seq');
    -- The limit, read here after the fence the charge holds, never passed by a caller.
    SELECT public.sagat_quota_step_log_limit(p_tenant), c.step_log_enforcement_enabled INTO v_limit, v_enforced
      FROM public.quota_accounting_control c WHERE c.singleton;
    IF v_limit - v_s - v_g < greatest(p_bytes, 1048576) THEN
        -- Near or past the limit: every other quota lease no open transaction holds gives its room back.
        FOR v_lease IN SELECT writer, granted, used FROM public.step_log_quota_leases
                        WHERE tenant_id = p_tenant AND writer <> pg_backend_pid() AND granted > 0
                        ORDER BY writer FOR UPDATE SKIP LOCKED LOOP
            v_s := v_s + v_lease.used;
            v_g := v_g - v_lease.granted;
            UPDATE public.step_log_quota_leases SET granted = 0, used = 0
             WHERE tenant_id = p_tenant AND writer = v_lease.writer;
            PERFORM nextval('public.step_log_quota_refills_reclaim_seq');
        END LOOP;
    END IF;
    -- The writer's own quota lease, `NOWAIT`: only a holder of the usage row, or a limit change holding the fence
    -- exclusively, could hold it, so nothing waits after the usage row.
    SELECT granted, used INTO v_granted, v_used FROM public.step_log_quota_leases
     WHERE tenant_id = p_tenant AND writer = pg_backend_pid() FOR UPDATE NOWAIT;
    IF NOT FOUND THEN
        INSERT INTO public.step_log_quota_leases (tenant_id, writer) VALUES (p_tenant, pg_backend_pid());
        v_granted := 0;
        v_used := 0;
    END IF;
    -- Fold the writer's quota lease whole, then grant it g = max(n, min(1 MiB, ⌊F / (2(A + 1))⌋)), F the free room
    -- and A the tenant's other quota leases holding a grant (I4: S + G ≤ L after the grant).
    v_s := v_s + v_used;
    v_g := v_g - v_granted;
    v_free := v_limit - v_s - v_g;
    SELECT count(*) INTO v_others FROM public.step_log_quota_leases
     WHERE tenant_id = p_tenant AND writer <> pg_backend_pid() AND granted > 0;
    v_grant := greatest(p_bytes, least(1048576, greatest(v_free, 0) / (2 * (v_others + 1))));
    IF (v_enforced AND v_grant > v_free) OR v_grant = 0 THEN
        -- Held: the room missing is in quota leases open transactions hold (their room read without a lock).
        IF v_grant > 0 AND v_grant <= v_free + coalesce((SELECT sum(granted - used) FROM public.step_log_quota_leases
                                          WHERE tenant_id = p_tenant AND writer <> pg_backend_pid()), 0) THEN
            RAISE EXCEPTION 'tenant step-log quota held by open transactions'
                USING ERRCODE = 'program_limit_exceeded', CONSTRAINT = 'tenant_step_log_quota_held';
        END IF;
        RAISE EXCEPTION 'tenant step-log quota exceeded'
            USING ERRCODE = 'program_limit_exceeded', CONSTRAINT = 'tenant_step_log_quota_exceeded';
    END IF;
    UPDATE public.step_log_quota_leases
       SET granted = v_grant, used = CASE WHEN p_charge THEN p_bytes ELSE 0 END, charged_at = clock_timestamp()
     WHERE tenant_id = p_tenant AND writer = pg_backend_pid();
    UPDATE public.tenant_retained_usage SET step_log_bytes = v_s, step_log_leased_bytes = v_g + v_grant
     WHERE tenant_id = p_tenant;
END
$$;

-- The charge (ADR-0097 decision 3), once per insert statement and tenant, with the bytes the statement trigger measured:
-- the three class fences shared in ADR-0027's order (one select list, evaluated left to right), the limit, S and G
-- without a lock, and, when S + G ≤ L or the quota is not enforced, the writer's own quota lease with room, locked
-- `SKIP LOCKED` (only a reclaim, holding the usage row, can hold it). It never locks the usage row and never waits for
-- a quota lease; otherwise the refill decides.
CREATE FUNCTION sagat_quota_step_log_charge(p_tenant uuid, p_bytes bigint) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE v_limit bigint; v_s bigint; v_g bigint; v_enforced boolean; v_lease integer;
BEGIN
    IF p_bytes <= 0 THEN
        RETURN;
    END IF;
    PERFORM pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_content_bytes:' || p_tenant::text, 0)),
            pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_log_bytes:' || p_tenant::text, 0)),
            pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0));
    v_limit := public.sagat_quota_step_log_limit(p_tenant);
    SELECT u.step_log_bytes, u.step_log_leased_bytes, c.step_log_enforcement_enabled INTO v_s, v_g, v_enforced
      FROM public.quota_accounting_control c
      LEFT JOIN public.tenant_retained_usage u ON u.tenant_id = p_tenant
     WHERE c.singleton;
    IF NOT v_enforced OR coalesce(v_s, 0) + coalesce(v_g, 0) <= v_limit THEN
        UPDATE public.step_log_quota_leases SET used = used + p_bytes, charged_at = clock_timestamp()
         WHERE tenant_id = p_tenant AND writer = (
               SELECT writer FROM public.step_log_quota_leases
                WHERE tenant_id = p_tenant AND writer = pg_backend_pid() AND granted - used >= p_bytes
                  FOR UPDATE SKIP LOCKED)
        RETURNING writer INTO v_lease;
        IF v_lease IS NOT NULL THEN
            RETURN;
        END IF;
    END IF;
    PERFORM public.sagat_quota_step_log_refill(p_tenant, p_bytes, true);
END
$$;

-- The statement trigger of the lines (ADR-0027's Amendment of the Amendment): per tenant, over the statement's
-- transition table, what `sagat_quota_measure` measures (the same function the per-row release applies), charged once,
-- in tenant order. No delta row and no deferred flush on insert.
CREATE FUNCTION sagat_quota_step_log_insert_trigger() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE r record;
BEGIN
    FOR r IN SELECT i.tenant_id,
                    sum(public.sagat_quota_measure(j.v, 'octet_length_utf8', 'exception', i.exception IS NULL)
                        + public.sagat_quota_measure(j.v, 'octet_length_utf8', 'fields', i.fields IS NULL)
                        + public.sagat_quota_measure(j.v, 'fixed_octets', 'level', i.level IS NULL)
                        + public.sagat_quota_measure(j.v, 'octet_length_utf8', 'message', i.message IS NULL)
                       )::bigint AS bytes
               FROM inserted i
               CROSS JOIN LATERAL (SELECT jsonb_build_object('exception', i.exception, 'fields', i.fields,
                                                             'level', i.level, 'message', i.message) AS v) j
              GROUP BY i.tenant_id ORDER BY i.tenant_id LOOP
        PERFORM public.sagat_quota_step_log_charge(r.tenant_id, r.bytes);
    END LOOP;
    RETURN NULL;
END
$$;

-- The carry's reservation (ADR-0097 decision 3, D041-113, D041-114): the writer's own quota lease with room for
-- `p_bytes`, locked for the transaction and nothing written, after the step-log fence is tried shared (holding a quota
-- lease always means holding that fence). `room` is that quota lease's room, read under the lock when it locked it,
-- else without a lock, and null when the writer has no quota lease of the tenant.
CREATE FUNCTION sagat_quota_step_log_reserve(p_tenant uuid, p_bytes bigint)
RETURNS TABLE (reserved boolean, room bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE v_s bigint; v_g bigint; v_enforced boolean; v_room bigint;
BEGIN
    IF p_bytes > 0 AND pg_try_advisory_xact_lock_shared(
           hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0)) THEN
        SELECT u.step_log_bytes, u.step_log_leased_bytes, c.step_log_enforcement_enabled INTO v_s, v_g, v_enforced
          FROM public.quota_accounting_control c
          LEFT JOIN public.tenant_retained_usage u ON u.tenant_id = p_tenant
         WHERE c.singleton;
        IF NOT v_enforced OR coalesce(v_s, 0) + coalesce(v_g, 0) <= public.sagat_quota_step_log_limit(p_tenant) THEN
            SELECT l.granted - l.used INTO v_room FROM public.step_log_quota_leases l
             WHERE l.tenant_id = p_tenant AND l.writer = pg_backend_pid() AND l.granted - l.used >= p_bytes
               FOR UPDATE SKIP LOCKED;
            IF FOUND THEN
                RETURN QUERY SELECT true, v_room;
                RETURN;
            END IF;
        END IF;
    END IF;
    RETURN QUERY SELECT false, (SELECT l.granted - l.used FROM public.step_log_quota_leases l
                                 WHERE l.tenant_id = p_tenant AND l.writer = pg_backend_pid());
END
$$;

-- The top-up after a carrying report (ADR-0097 decision 3b, decisions D041-124, D041-134, OD-2026-11-20-18): one
-- statement in its own transaction, under no run lock. It takes no amount, never waits, takes no wait slot, reclaims no
-- other quota lease and never raises for its answers: `contended` (the step-log fence held), `room` (room ≥ M already,
-- or a share not above the current room), `busy` (the usage row held or absent), `limit` (a share of 0) or `made` (the
-- writer's quota lease folded and granted exactly its share, with no charge). `room` is the writer's room after it.
CREATE FUNCTION sagat_quota_step_log_top_up(p_tenant uuid)
RETURNS TABLE (outcome text, room bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE v_s bigint; v_g bigint; v_limit bigint; v_room bigint; v_granted bigint; v_used bigint; v_others bigint;
        v_share bigint; v_leased boolean;
BEGIN
    SELECT l.granted - l.used INTO v_room FROM public.step_log_quota_leases l
     WHERE l.tenant_id = p_tenant AND l.writer = pg_backend_pid();
    IF NOT pg_try_advisory_xact_lock_shared(
           hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0)) THEN
        RETURN QUERY SELECT 'contended'::text, coalesce(v_room, 0);
        RETURN;
    END IF;
    IF coalesce(v_room, 0) >= public.sagat_quota_step_log_top_up_mark() THEN
        RETURN QUERY SELECT 'room'::text, v_room;
        RETURN;
    END IF;
    SELECT step_log_bytes, step_log_leased_bytes INTO v_s, v_g
      FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE SKIP LOCKED;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'busy'::text, coalesce(v_room, 0);
        RETURN;
    END IF;
    v_limit := public.sagat_quota_step_log_limit(p_tenant);
    -- The writer's own quota lease, `NOWAIT`: this transaction holds the usage row and the fence shared, so no one
    -- else can hold it.
    SELECT granted, used INTO v_granted, v_used FROM public.step_log_quota_leases
     WHERE tenant_id = p_tenant AND writer = pg_backend_pid() FOR UPDATE NOWAIT;
    v_leased := FOUND;
    IF NOT v_leased THEN
        v_granted := 0;
        v_used := 0;
    END IF;
    SELECT count(*) INTO v_others FROM public.step_log_quota_leases
     WHERE tenant_id = p_tenant AND writer <> pg_backend_pid() AND granted > 0;
    v_share := least(1048576, greatest(v_limit - v_s - v_g, 0) / (2 * (v_others + 1)));
    IF v_share = 0 THEN
        RETURN QUERY SELECT 'limit'::text, v_granted - v_used;
        RETURN;
    END IF;
    IF v_share <= v_granted - v_used THEN
        RETURN QUERY SELECT 'room'::text, v_granted - v_used;
        RETURN;
    END IF;
    IF NOT v_leased THEN
        INSERT INTO public.step_log_quota_leases (tenant_id, writer, granted, used)
        VALUES (p_tenant, pg_backend_pid(), v_share, 0);
    ELSE
        UPDATE public.step_log_quota_leases SET granted = v_share, used = 0, charged_at = clock_timestamp()
         WHERE tenant_id = p_tenant AND writer = pg_backend_pid();
    END IF;
    UPDATE public.tenant_retained_usage
       SET step_log_bytes = v_s + v_used, step_log_leased_bytes = v_g - v_granted + v_share
     WHERE tenant_id = p_tenant;
    RETURN QUERY SELECT 'made'::text, v_share;
END
$$;

-- The tenants the purge pass serves (decision D041-120): ids only, at most `p_max`, of tenants with a quota lease not
-- charged for 10 minutes (the pools' idle timeout) or whose backend has ended. `VOLATILE`: it reads the clock and
-- `pg_stat_activity`, which change within a statement.
CREATE FUNCTION sagat_quota_step_log_idle_tenants(p_max integer) RETURNS SETOF uuid
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT DISTINCT l.tenant_id FROM public.step_log_quota_leases l
     WHERE l.charged_at < clock_timestamp() - interval '10 minutes'
        OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity a WHERE a.pid = l.writer)
     LIMIT greatest(p_max, 0)
$$;

-- The idle and ended-backend reclaim (decisions D041-117, D041-119, D041-120), one tenant a transaction: nothing idle
-- answers `(0, false)` before any lock; the step-log fence is tried shared (held: `(0, true)`, the tenant is skipped this
-- pass), then under the usage row every such quota lease, `SKIP LOCKED`, is folded and deleted.
CREATE FUNCTION sagat_quota_step_log_reclaim_idle(p_tenant uuid)
RETURNS TABLE (reclaimed integer, skipped boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE v_lease record; v_count integer := 0;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.step_log_quota_leases l
                    WHERE l.tenant_id = p_tenant
                      AND (l.charged_at < clock_timestamp() - interval '10 minutes'
                           OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity a WHERE a.pid = l.writer))) THEN
        RETURN QUERY SELECT 0, false;
        RETURN;
    END IF;
    IF NOT pg_try_advisory_xact_lock_shared(
           hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0)) THEN
        RETURN QUERY SELECT 0, true;
        RETURN;
    END IF;
    PERFORM 1 FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE;
    FOR v_lease IN SELECT l.writer, l.granted, l.used,
                          NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity a WHERE a.pid = l.writer) AS ended
                     FROM public.step_log_quota_leases l
                    WHERE l.tenant_id = p_tenant
                      AND (l.charged_at < clock_timestamp() - interval '10 minutes'
                           OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity a WHERE a.pid = l.writer))
                    ORDER BY l.writer FOR UPDATE OF l SKIP LOCKED LOOP
        UPDATE public.tenant_retained_usage
           SET step_log_bytes = step_log_bytes + v_lease.used,
               step_log_leased_bytes = step_log_leased_bytes - v_lease.granted
         WHERE tenant_id = p_tenant;
        DELETE FROM public.step_log_quota_leases WHERE tenant_id = p_tenant AND writer = v_lease.writer;
        IF v_lease.ended THEN
            PERFORM nextval('public.step_log_quota_refills_ended_seq');
        ELSE
            PERFORM nextval('public.step_log_quota_refills_idle_seq');
        END IF;
        v_count := v_count + 1;
    END LOOP;
    RETURN QUERY SELECT v_count, false;
END
$$;

-- A change of the tenant's step-log limit or ceiling reclaims every quota lease (ADR-0097 decision 6), whoever wrote the
-- row: the step-log fence exclusively (at once when the editor holds it, so no transaction holds a quota lease), the
-- quota leases `NOWAIT`, then the usage row.
CREATE FUNCTION sagat_quota_step_log_reclaim() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE v_tenant uuid; v_resource text; v_used bigint; v_leases integer;
BEGIN
    IF TG_OP = 'DELETE' THEN
        v_tenant := OLD.tenant_id;
        v_resource := OLD.resource;
    ELSE
        v_tenant := NEW.tenant_id;
        v_resource := NEW.resource;
    END IF;
    IF v_resource <> 'tenant_step_log_bytes'
       AND (TG_OP <> 'UPDATE' OR OLD.resource <> 'tenant_step_log_bytes') THEN
        RETURN NULL;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('quota-policy:tenant_step_log_bytes:' || v_tenant::text, 0));
    PERFORM 1 FROM public.step_log_quota_leases WHERE tenant_id = v_tenant ORDER BY writer FOR UPDATE NOWAIT;
    PERFORM 1 FROM public.tenant_retained_usage WHERE tenant_id = v_tenant FOR UPDATE;
    SELECT coalesce(sum(used), 0)::bigint, count(*) FILTER (WHERE granted > 0)::integer INTO v_used, v_leases
      FROM public.step_log_quota_leases WHERE tenant_id = v_tenant;
    UPDATE public.tenant_retained_usage
       SET step_log_bytes = step_log_bytes + v_used, step_log_leased_bytes = 0
     WHERE tenant_id = v_tenant;
    UPDATE public.step_log_quota_leases SET granted = 0, used = 0 WHERE tenant_id = v_tenant AND granted > 0;
    FOR i IN 1 .. v_leases LOOP
        PERFORM nextval('public.step_log_quota_refills_reclaim_seq');
    END LOOP;
    RETURN NULL;
END
$$;
CREATE TRIGGER quota_limits_step_log_reclaim AFTER INSERT OR UPDATE OR DELETE ON quota_limits
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_step_log_reclaim();
ALTER TABLE quota_limits ENABLE ALWAYS TRIGGER quota_limits_step_log_reclaim;
CREATE TRIGGER quota_ceilings_step_log_reclaim AFTER INSERT OR UPDATE OR DELETE ON quota_ceilings
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_step_log_reclaim();
ALTER TABLE quota_ceilings ENABLE ALWAYS TRIGGER quota_ceilings_step_log_reclaim;

-- The counters SQL keeps (data-model.md §7), raw and monotonic, with the quota-lease row count; the replica that holds
-- shard 0 publishes them (task T041-10).
CREATE FUNCTION sagat_quota_step_log_counters()
RETURNS TABLE (refills bigint, reclaims bigint, idle bigint, ended bigint, lease_rows bigint)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT (SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM public.step_log_quota_refills_refill_seq),
           (SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM public.step_log_quota_refills_reclaim_seq),
           (SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM public.step_log_quota_refills_idle_seq),
           (SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM public.step_log_quota_refills_ended_seq),
           (SELECT count(*) FROM public.step_log_quota_leases)
$$;

-- The finalizer (ADR-0027) with the release branch of the class `step_log` (ADR-0097 decision 5): a step-log delta is
-- only ever negative (the per-row `AFTER DELETE` trigger of the lines; inserts charge quota leases), and its branch,
-- after the fences and under the usage row, reads Σ used and refuses an underflow of U = S + Σ used. It takes no quota
-- lease. A positive step-log delta would be a charge outside a quota lease and is refused.
CREATE OR REPLACE FUNCTION sagat_quota_flush_deltas() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE d record; next_content bigint; next_reserved bigint; next_logs bigint; next_step_logs bigint;
        leased_used bigint; content_limit bigint; log_limit bigint;
        enforce_content boolean; enforce_logs boolean;
BEGIN
    SELECT content_enforcement_enabled, log_enforcement_enabled
      INTO enforce_content, enforce_logs
      FROM public.quota_accounting_control WHERE singleton;
    FOR d IN SELECT * FROM public.quota_transaction_deltas
              WHERE transaction_id = pg_current_xact_id()
                AND (content_delta <> 0 OR content_reserved_delta <> 0 OR log_delta <> 0 OR step_log_delta <> 0)
              ORDER BY tenant_id LOOP
        IF d.step_log_delta > 0 THEN
            RAISE EXCEPTION 'a step-log charge outside a quota lease'
                USING ERRCODE = 'data_exception', CONSTRAINT = 'step_log_delta_unleased';
        END IF;
        IF (enforce_content AND d.content_delta + d.content_reserved_delta > 0)
           OR (enforce_logs AND d.log_delta > 0) THEN
            -- Content policy is first even for log-only writes because log finalization can share a
            -- transaction with content admission. Policy fences always precede the usage-row lock.
            PERFORM pg_advisory_xact_lock_shared(hashtextextended(
                'quota-policy:tenant_content_bytes:' || d.tenant_id::text, 0));
        END IF;
        IF enforce_content AND d.content_delta + d.content_reserved_delta > 0 THEN
            SELECT least(
                coalesce((SELECT max_value FROM public.quota_limits
                           WHERE tenant_id=d.tenant_id AND namespace_id IS NULL
                             AND resource='tenant_content_bytes'),
                         (SELECT max_value FROM public.quota_ceilings
                           WHERE tenant_id=d.tenant_id AND resource='tenant_content_bytes'),
                         10737418240),
                coalesce((SELECT max_value FROM public.quota_ceilings
                           WHERE tenant_id=d.tenant_id AND resource='tenant_content_bytes'),
                         10737418240)) INTO content_limit;
        END IF;
        IF enforce_logs AND d.log_delta > 0 THEN
            PERFORM pg_advisory_xact_lock_shared(hashtextextended(
                'quota-policy:tenant_log_bytes:' || d.tenant_id::text, 0));
            SELECT least(
                coalesce((SELECT max_value FROM public.quota_limits
                           WHERE tenant_id=d.tenant_id AND namespace_id IS NULL
                             AND resource='tenant_log_bytes'),
                         (SELECT max_value FROM public.quota_ceilings
                           WHERE tenant_id=d.tenant_id AND resource='tenant_log_bytes'),
                         1073741824),
                coalesce((SELECT max_value FROM public.quota_ceilings
                           WHERE tenant_id=d.tenant_id AND resource='tenant_log_bytes'),
                         1073741824)) INTO log_limit;
        END IF;
        IF d.step_log_delta < 0 THEN
            -- The step-log fence after the content and log fences, before the usage row (ADR-0027's order).
            PERFORM pg_advisory_xact_lock_shared(hashtextextended(
                'quota-policy:tenant_step_log_bytes:' || d.tenant_id::text, 0));
        END IF;
        INSERT INTO public.tenant_retained_usage
            (tenant_id, content_bytes, content_reserved_bytes, log_bytes)
        VALUES (d.tenant_id, 0, 0, 0) ON CONFLICT (tenant_id) DO NOTHING;
        SELECT content_bytes + d.content_delta,
               content_reserved_bytes + d.content_reserved_delta, log_bytes + d.log_delta,
               step_log_bytes + d.step_log_delta
          INTO next_content, next_reserved, next_logs, next_step_logs
          FROM public.tenant_retained_usage WHERE tenant_id = d.tenant_id FOR UPDATE;
        IF next_content < 0 OR next_reserved < 0 OR next_logs < 0 THEN
            RAISE EXCEPTION 'retained usage underflow for tenant' USING ERRCODE = 'data_exception';
        END IF;
        IF d.step_log_delta < 0 THEN
            -- The committed charges still in quota leases, read after the lock: U never goes below zero (I3).
            SELECT coalesce(sum(used), 0)::bigint INTO leased_used
              FROM public.step_log_quota_leases WHERE tenant_id = d.tenant_id;
            IF next_step_logs + leased_used < 0 THEN
                RAISE EXCEPTION 'retained step-log usage underflow for tenant'
                    USING ERRCODE = 'data_exception', CONSTRAINT = 'step_log_usage_underflow';
            END IF;
        END IF;
        IF enforce_content AND d.content_delta + d.content_reserved_delta > 0 THEN
            IF next_content + next_reserved > content_limit THEN
                RAISE EXCEPTION 'tenant content quota exceeded'
                    USING ERRCODE = 'program_limit_exceeded',
                          CONSTRAINT = 'tenant_content_quota_exceeded';
            END IF;
        END IF;
        IF enforce_logs AND d.log_delta > 0 AND next_logs > log_limit THEN
            RAISE EXCEPTION 'tenant log quota exceeded'
                USING ERRCODE = 'program_limit_exceeded',
                      CONSTRAINT = 'tenant_log_quota_exceeded';
        END IF;
        UPDATE public.tenant_retained_usage SET content_bytes=next_content,
            content_reserved_bytes=next_reserved, log_bytes=next_logs, step_log_bytes=next_step_logs
          WHERE tenant_id=d.tenant_id;
    END LOOP;
    DELETE FROM public.quota_transaction_deltas
     WHERE transaction_id = pg_current_xact_id();
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_flush_deltas() FROM PUBLIC;

-- The quota triggers of the lines: the statement-level charge on insert, and the per-row release and its deferred
-- finalizer on delete, so that the purge releases exactly what it deletes. No update trigger: lines never change.
CREATE TRIGGER step_log_lines_quota_insert AFTER INSERT ON step_log_lines
    REFERENCING NEW TABLE AS inserted FOR EACH STATEMENT EXECUTE FUNCTION sagat_quota_step_log_insert_trigger();
ALTER TABLE step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_quota_insert;
CREATE TRIGGER step_log_lines_quota_delete AFTER DELETE ON step_log_lines FOR EACH ROW
    EXECUTE FUNCTION sagat_quota_row_delta('step_log', 'octet_length_utf8', 'exception',
                                           'step_log', 'octet_length_utf8', 'fields',
                                           'step_log', 'fixed_octets', 'level',
                                           'step_log', 'octet_length_utf8', 'message');
ALTER TABLE step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_quota_delete;
CREATE CONSTRAINT TRIGGER step_log_lines_quota_flush AFTER DELETE ON step_log_lines
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_quota_flush;

-- ---------------------------------------------------------------------------------------------------------------------
-- §4. Grants. PostgreSQL grants `EXECUTE` to `PUBLIC` on a new function, so every one is revoked from it.

REVOKE ALL ON FUNCTION sagat_quota_step_log_top_up_mark(), sagat_quota_step_log_wait_slots(),
    sagat_quota_step_log_refill_wait(), sagat_quota_step_log_limit(uuid), sagat_quota_step_log_usage(uuid),
    sagat_quota_step_log_allowance(uuid), sagat_quota_step_log_refill(uuid, bigint, boolean),
    sagat_quota_step_log_charge(uuid, bigint), sagat_quota_step_log_insert_trigger(),
    sagat_quota_step_log_reserve(uuid, bigint), sagat_quota_step_log_top_up(uuid),
    sagat_quota_step_log_idle_tenants(integer), sagat_quota_step_log_reclaim_idle(uuid),
    sagat_quota_step_log_reclaim(), sagat_quota_step_log_counters(), sagat_quota_step_log_leases_cascade() FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        GRANT SELECT, INSERT, DELETE ON step_log_lines TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON step_log_streams TO sagat_app;
        GRANT USAGE ON SEQUENCE step_log_streams_stream_id_seq TO sagat_app;
        REVOKE ALL ON step_log_quota_leases FROM sagat_app;
        REVOKE ALL ON SEQUENCE step_log_quota_refills_refill_seq, step_log_quota_refills_reclaim_seq,
            step_log_quota_refills_idle_seq, step_log_quota_refills_ended_seq FROM sagat_app;
        REVOKE ALL ON FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint, bigint),
            sagat_quota_step_log_refill_wait(), sagat_quota_step_log_limit(uuid),
            sagat_quota_step_log_refill(uuid, bigint, boolean), sagat_quota_step_log_charge(uuid, bigint),
            sagat_quota_step_log_insert_trigger(), sagat_quota_step_log_reclaim(), sagat_quota_step_log_leases_cascade(),
            step_log_streams_guard(), step_log_lines_append_only() FROM sagat_app;
        GRANT EXECUTE ON FUNCTION sagat_quota_step_log_allowance(uuid), sagat_quota_step_log_reserve(uuid, bigint),
            sagat_quota_step_log_top_up(uuid), sagat_quota_step_log_usage(uuid), sagat_quota_step_log_counters(),
            sagat_quota_step_log_idle_tenants(integer), sagat_quota_step_log_reclaim_idle(uuid),
            sagat_quota_step_log_top_up_mark(), sagat_quota_step_log_wait_slots() TO sagat_app;
    END IF;
END
$grants$;
