-- Rollback is an operator/quiesced-service operation. A stored step-log line is a record an older reader cannot read:
-- the rollback refuses while one exists. With none, the usage U = S + Σ used is 0, so folding the quota leases' used
-- bytes into S leaves S = 0, which the rollback of 20270201000000_step_log_quotas requires; then everything this
-- migration added is dropped and the four functions it replaced get their bodies of 20261024000000_durable_quotas back.
LOCK TABLE step_log_lines, step_log_streams, step_log_quota_leases, tenant_retained_usage, quota_limits, quota_ceilings
    IN ACCESS EXCLUSIVE MODE;
DO $guard$
BEGIN
    IF EXISTS (SELECT 1 FROM step_log_lines) THEN
        RAISE EXCEPTION 'step-log lines are retained; keep a compatible reader'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;
END $guard$;
UPDATE tenant_retained_usage u
   SET step_log_bytes = u.step_log_bytes + l.used, step_log_leased_bytes = 0
  FROM (SELECT tenant_id, sum(used)::bigint AS used FROM step_log_quota_leases GROUP BY tenant_id) l
 WHERE u.tenant_id = l.tenant_id;

DROP TRIGGER quota_limits_step_log_reclaim ON quota_limits;
DROP TRIGGER quota_ceilings_step_log_reclaim ON quota_ceilings;
DROP TRIGGER tenant_retained_usage_step_log_leases_cascade ON tenant_retained_usage;
DROP TABLE step_log_lines;
DROP TABLE step_log_streams;
DROP TABLE step_log_quota_leases;
DROP FUNCTION sagat_quota_step_log_insert_trigger(), sagat_quota_step_log_charge(uuid, bigint),
    sagat_quota_step_log_refill(uuid, bigint, boolean), sagat_quota_step_log_reserve(uuid, bigint),
    sagat_quota_step_log_top_up(uuid), sagat_quota_step_log_idle_tenants(integer),
    sagat_quota_step_log_reclaim_idle(uuid), sagat_quota_step_log_reclaim(), sagat_quota_step_log_counters(),
    sagat_quota_step_log_allowance(uuid), sagat_quota_step_log_usage(uuid), sagat_quota_step_log_limit(uuid),
    sagat_quota_step_log_refill_wait(), sagat_quota_step_log_wait_slots(), sagat_quota_step_log_top_up_mark(),
    sagat_quota_step_log_leases_cascade(), step_log_streams_guard(), step_log_lines_append_only();
DROP SEQUENCE step_log_quota_refills_refill_seq, step_log_quota_refills_reclaim_seq, step_log_quota_refills_idle_seq,
    step_log_quota_refills_ended_seq;
ALTER TABLE tenant_retained_usage DROP COLUMN step_log_leased_bytes;

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
    END IF;
    RAISE EXCEPTION 'unknown quota measurement %', measurement;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_measure(jsonb, text, text, boolean) FROM PUBLIC;

DROP FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint, bigint);
CREATE FUNCTION sagat_quota_add_delta(tenant uuid, content bigint, reserved bigint, logs bigint)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    INSERT INTO public.quota_transaction_deltas
        (transaction_id, tenant_id, content_delta, content_reserved_delta, log_delta)
    VALUES (pg_current_xact_id(), tenant, content, reserved, logs)
    ON CONFLICT (transaction_id, tenant_id) DO UPDATE SET
        content_delta = public.quota_transaction_deltas.content_delta + EXCLUDED.content_delta,
        content_reserved_delta = public.quota_transaction_deltas.content_reserved_delta
                                 + EXCLUDED.content_reserved_delta,
        log_delta = public.quota_transaction_deltas.log_delta + EXCLUDED.log_delta
$$;
REVOKE ALL ON FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint) FROM PUBLIC;

CREATE OR REPLACE FUNCTION sagat_quota_row_delta() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    old_row jsonb := CASE WHEN TG_OP = 'INSERT' THEN NULL ELSE to_jsonb(OLD) END;
    new_row jsonb := CASE WHEN TG_OP = 'DELETE' THEN NULL ELSE to_jsonb(NEW) END;
    old_tenant uuid; new_tenant uuid;
    old_content bigint := 0; new_content bigint := 0;
    old_logs bigint := 0; new_logs bigint := 0;
    i integer := 0; measured bigint;
    old_is_null boolean; new_is_null boolean;
BEGIN
    IF old_row IS NOT NULL THEN old_tenant := (old_row ->> 'tenant_id')::uuid; END IF;
    IF new_row IS NOT NULL THEN new_tenant := (new_row ->> 'tenant_id')::uuid; END IF;
    WHILE i < TG_NARGS LOOP
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
            ELSE old_logs := old_logs + measured; END IF;
        END IF;
        IF new_row IS NOT NULL THEN
            measured := public.sagat_quota_measure(
                new_row, TG_ARGV[i + 1], TG_ARGV[i + 2], new_is_null);
            IF TG_ARGV[i] = 'content' THEN new_content := new_content + measured;
            ELSE new_logs := new_logs + measured; END IF;
        END IF;
        i := i + 3;
    END LOOP;
    IF old_tenant IS NOT DISTINCT FROM new_tenant THEN
        IF new_content <> old_content OR new_logs <> old_logs THEN
            PERFORM public.sagat_quota_add_delta(
                new_tenant, new_content - old_content, 0, new_logs - old_logs);
        END IF;
    ELSE
        IF old_tenant IS NOT NULL THEN
            PERFORM public.sagat_quota_add_delta(old_tenant, -old_content, 0, -old_logs);
        END IF;
        IF new_tenant IS NOT NULL THEN
            PERFORM public.sagat_quota_add_delta(new_tenant, new_content, 0, new_logs);
        END IF;
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_row_delta() FROM PUBLIC;

CREATE OR REPLACE FUNCTION sagat_quota_flush_deltas() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE d record; next_content bigint; next_reserved bigint; next_logs bigint;
        content_limit bigint; log_limit bigint;
        enforce_content boolean; enforce_logs boolean;
BEGIN
    SELECT content_enforcement_enabled, log_enforcement_enabled
      INTO enforce_content, enforce_logs
      FROM public.quota_accounting_control WHERE singleton;
    FOR d IN SELECT * FROM public.quota_transaction_deltas
              WHERE transaction_id = pg_current_xact_id()
                AND (content_delta <> 0 OR content_reserved_delta <> 0 OR log_delta <> 0)
              ORDER BY tenant_id LOOP
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
        INSERT INTO public.tenant_retained_usage
            (tenant_id, content_bytes, content_reserved_bytes, log_bytes)
        VALUES (d.tenant_id, 0, 0, 0) ON CONFLICT (tenant_id) DO NOTHING;
        SELECT content_bytes + d.content_delta,
               content_reserved_bytes + d.content_reserved_delta, log_bytes + d.log_delta
          INTO next_content, next_reserved, next_logs
          FROM public.tenant_retained_usage WHERE tenant_id = d.tenant_id FOR UPDATE;
        IF next_content < 0 OR next_reserved < 0 OR next_logs < 0 THEN
            RAISE EXCEPTION 'retained usage underflow for tenant' USING ERRCODE = 'data_exception';
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
            content_reserved_bytes=next_reserved, log_bytes=next_logs
          WHERE tenant_id=d.tenant_id;
    END LOOP;
    DELETE FROM public.quota_transaction_deltas
     WHERE transaction_id = pg_current_xact_id();
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_flush_deltas() FROM PUBLIC;

ALTER TABLE quota_transaction_deltas DROP COLUMN step_log_delta;
ALTER TABLE quota_accounting_control DROP COLUMN step_log_enforcement_enabled;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON FUNCTION sagat_quota_add_delta(uuid, bigint, bigint, bigint) FROM sagat_app;
    END IF;
END
$grants$;
