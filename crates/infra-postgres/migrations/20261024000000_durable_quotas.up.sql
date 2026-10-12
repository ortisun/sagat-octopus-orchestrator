-- ADR-0027 / FR-009-006: durable retained-byte accounting and authoritative rate/retention policy state.
-- Generated measurement trigger declarations correspond one-for-one with
-- `../storage-classification.json`; the integration coverage test rejects drift.

ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_resource_valid;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_tenant_scoped;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_tenant_scoped CHECK (
    namespace_id IS NULL OR resource IN
        ('active_runs', 'workflows', 'secrets', 'event_endpoints', 'webhook_subscriptions',
         'broker_subscriptions', 'event_connectors', 'scripts', 'script_jobs',
         'activity_concurrency', 'api_requests_per_second', 'events_per_second',
         'event_retention_seconds', 'script_retention_days'));
ALTER TABLE quota_limits DROP CONSTRAINT quota_limits_value_range;
ALTER TABLE quota_limits ADD CONSTRAINT quota_limits_value_range CHECK (
    CASE resource
      WHEN 'tenant_content_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'tenant_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'api_requests_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'events_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'event_retention_seconds' THEN max_value = 0 OR max_value BETWEEN 60 AND 2592000
      WHEN 'script_retention_days' THEN max_value BETWEEN 0 AND 365
      ELSE max_value BETWEEN 0 AND 1000000000
    END);

ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_resource_valid;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_resource_valid CHECK (resource IN
    ('active_runs', 'tenant_active_runs', 'workflows', 'worker_sessions', 'secrets',
     'event_endpoints', 'webhook_subscriptions', 'broker_subscriptions', 'event_connectors',
     'scripts', 'script_jobs', 'activity_concurrency',
     'tenant_content_bytes', 'tenant_log_bytes', 'api_requests_per_second',
     'events_per_second', 'event_retention_seconds', 'script_retention_days'));
ALTER TABLE quota_ceilings DROP CONSTRAINT quota_ceilings_value_range;
ALTER TABLE quota_ceilings ADD CONSTRAINT quota_ceilings_value_range CHECK (
    CASE resource
      WHEN 'tenant_content_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'tenant_log_bytes' THEN max_value BETWEEN 0 AND 1125899906842624
      WHEN 'api_requests_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'events_per_second' THEN max_value BETWEEN 0 AND 1000000
      WHEN 'event_retention_seconds' THEN max_value = 0 OR max_value BETWEEN 60 AND 2592000
      WHEN 'script_retention_days' THEN max_value BETWEEN 0 AND 365
      ELSE max_value BETWEEN 0 AND 1000000000
    END);

CREATE TABLE tenant_retained_usage (
    tenant_id               uuid   PRIMARY KEY REFERENCES tenants (id) ON DELETE CASCADE,
    content_bytes           bigint NOT NULL DEFAULT 0 CHECK (content_bytes >= 0),
    content_reserved_bytes  bigint NOT NULL DEFAULT 0 CHECK (content_reserved_bytes >= 0),
    log_bytes               bigint NOT NULL DEFAULT 0 CHECK (log_bytes >= 0)
);

-- Private transaction-local net deltas. Runtime roles receive no table privilege.
CREATE TABLE quota_transaction_deltas (
    transaction_id          xid8   NOT NULL,
    tenant_id               uuid   NOT NULL,
    content_delta           bigint NOT NULL DEFAULT 0,
    content_reserved_delta  bigint NOT NULL DEFAULT 0,
    log_delta               bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (transaction_id, tenant_id)
);

-- Content rejection remains disabled until every asynchronous terminal fallback and blob caller is
-- wired. Only a later reviewed migration may flip this owner-only row; there is no session bypass.
CREATE TABLE quota_accounting_control (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    content_enforcement_enabled boolean NOT NULL DEFAULT false,
    log_enforcement_enabled boolean NOT NULL DEFAULT false
);
INSERT INTO quota_accounting_control DEFAULT VALUES;

-- Every tenant has one usage row. Provisioning through an ALWAYS trigger lets the runtime keep
-- direct DML on the private accounting table revoked.
CREATE FUNCTION sagat_quota_provision_tenant_usage() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    INSERT INTO public.tenant_retained_usage (tenant_id) VALUES (NEW.id)
    ON CONFLICT (tenant_id) DO NOTHING;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_provision_tenant_usage() FROM PUBLIC;
CREATE TRIGGER tenants_quota_usage AFTER INSERT ON tenants
    FOR EACH ROW EXECUTE FUNCTION sagat_quota_provision_tenant_usage();
ALTER TABLE tenants ENABLE ALWAYS TRIGGER tenants_quota_usage;

CREATE TABLE quota_rate_buckets (
    tenant_id     uuid        NOT NULL,
    namespace_id  uuid        NOT NULL,
    resource      text        NOT NULL,
    source_kind   text        NOT NULL,
    source_id     uuid        NOT NULL,
    microtokens   bigint      NOT NULL CHECK (microtokens >= 0),
    updated_at    timestamptz NOT NULL,
    PRIMARY KEY (tenant_id, namespace_id, resource, source_kind, source_id),
    CONSTRAINT quota_rate_buckets_namespace_fk FOREIGN KEY (tenant_id, namespace_id)
        REFERENCES namespaces (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT quota_rate_buckets_resource_valid CHECK
        (resource IN ('api_requests_per_second', 'events_per_second')),
    CONSTRAINT quota_rate_buckets_source_valid CHECK
        ((source_kind = 'NAMESPACE' AND source_id = '00000000-0000-0000-0000-000000000000')
         OR (resource = 'events_per_second' AND source_kind IN ('ENDPOINT', 'CONNECTOR')
             AND source_id <> '00000000-0000-0000-0000-000000000000'))
);

-- The shipped upload ceiling has always been 5 GiB. Legacy uncommitted rows did not persist the
-- cap selected for their upload, so they conservatively retain that historical hard maximum.
-- Custom embedders that exceeded the shipped ceiling must reconcile before this migration.
ALTER TABLE blobs ADD COLUMN reservation_bytes bigint NOT NULL DEFAULT 5368709120
    CHECK (reservation_bytes >= 0 AND reservation_bytes <= 5368709120);
UPDATE blobs SET reservation_bytes = 0 WHERE uploaded_at IS NOT NULL;

CREATE FUNCTION sagat_blob_reservation_state() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND OLD.uploaded_at IS NULL AND NEW.uploaded_at IS NULL
       AND NEW.reservation_bytes <> OLD.reservation_bytes THEN
        RAISE EXCEPTION 'a blob upload reservation is immutable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'blobs_reservation_immutable';
    END IF;
    IF NEW.uploaded_at IS NOT NULL
       AND (TG_OP = 'INSERT' OR OLD.uploaded_at IS NULL)
       AND NEW.size_bytes > NEW.reservation_bytes THEN
        RAISE EXCEPTION 'a finalized blob exceeds its upload reservation'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'blobs_within_reservation';
    END IF;
    IF TG_OP = 'UPDATE' AND OLD.uploaded_at IS NOT NULL
       AND ((NEW.uploaded_at IS NULL) IS DISTINCT FROM (OLD.uploaded_at IS NULL)
            OR NEW.size_bytes IS DISTINCT FROM OLD.size_bytes
            OR NEW.sha256 IS DISTINCT FROM OLD.sha256) THEN
        RAISE EXCEPTION 'a finalized blob measurement is immutable'
            USING ERRCODE = 'check_violation',
                  CONSTRAINT = 'blobs_finalized_measurement_immutable';
    END IF;
    IF NEW.uploaded_at IS NOT NULL THEN
        NEW.reservation_bytes := 0;
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION sagat_blob_reservation_state() FROM PUBLIC;
CREATE TRIGGER blobs_reservation_state BEFORE INSERT OR UPDATE ON blobs
    FOR EACH ROW EXECUTE FUNCTION sagat_blob_reservation_state();
ALTER TABLE blobs ENABLE ALWAYS TRIGGER blobs_reservation_state;

CREATE FUNCTION sagat_quota_measure(
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

CREATE FUNCTION sagat_quota_row_delta() RETURNS trigger
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

CREATE FUNCTION sagat_quota_blob_delta() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE old_content bigint := 0; new_content bigint := 0;
        old_reserved bigint := 0; new_reserved bigint := 0;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        IF OLD.uploaded_at IS NOT NULL THEN old_content := OLD.size_bytes;
        ELSE old_reserved := OLD.reservation_bytes; END IF;
        PERFORM public.sagat_quota_add_delta(OLD.tenant_id, -old_content, -old_reserved, 0);
    END IF;
    IF TG_OP <> 'DELETE' THEN
        IF NEW.uploaded_at IS NOT NULL THEN new_content := NEW.size_bytes;
        ELSE new_reserved := NEW.reservation_bytes; END IF;
        PERFORM public.sagat_quota_add_delta(NEW.tenant_id, new_content, new_reserved, 0);
    END IF;
    RETURN NULL;
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_blob_delta() FROM PUBLIC;

-- Runtime log finalizers need an atomic read of private accounting state, but never permission to
-- mutate counters or inspect transaction deltas directly. The row lock is held through the caller's
-- transaction and the generated finalizer cross-checks the actual stored delta at commit.
CREATE FUNCTION sagat_quota_lock_log_usage(tenant uuid) RETURNS bigint
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE stored bigint; pending bigint;
BEGIN
    SELECT log_bytes INTO STRICT stored FROM public.tenant_retained_usage
     WHERE tenant_id = tenant FOR UPDATE;
    SELECT coalesce(log_delta, 0) INTO pending FROM public.quota_transaction_deltas
     WHERE transaction_id = pg_current_xact_id() AND tenant_id = tenant;
    RETURN stored + coalesce(pending, 0);
END
$$;
REVOKE ALL ON FUNCTION sagat_quota_lock_log_usage(uuid) FROM PUBLIC;

CREATE FUNCTION sagat_quota_flush_deltas() RETURNS trigger
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

-- Exclude writers for the exact one-time backfill. The generated trigger set below takes over only
-- after these locks and the backfill complete.
LOCK TABLE blobs, connector_dead_letters, connector_offsets, egress_policies, event_connectors, event_endpoints, inbox_events, memberships, namespaces, payloads, publication_decisions, publication_requests, runs, script_artifacts, script_executions, script_test_runs, script_versions, scripts, secret_access, secret_versions, secrets, steps, webhook_subscriptions, worker_bindings, worker_drains, workflow_triggers, workflow_versions, workflows, workspaces IN SHARE ROW EXCLUSIVE MODE;

INSERT INTO tenant_retained_usage (tenant_id) SELECT id FROM tenants ON CONFLICT DO NOTHING;

WITH measured AS (
    SELECT tenant_id, sum((octet_length(coalesce(body, ''::bytea)) + octet_length(coalesce(headers::text, '')) + octet_length(coalesce(position, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM connector_dead_letters GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(stream, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM connector_offsets GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(to_jsonb(ports)::text, '')) + octet_length(coalesce(to_jsonb(rules)::text, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM egress_policies GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(default_event_type, '')) + octet_length(coalesce(secret_name, '')) + octet_length(coalesce(settings::text, '')) + octet_length(coalesce(slug, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((octet_length(coalesce(last_error, '')))::bigint)::bigint AS log_bytes FROM event_connectors GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(default_event_type, '')) + octet_length(coalesce(mtls_certificate_sha256, '')) + octet_length(coalesce(oidc_profile, '')) + octet_length(coalesce(oidc_subject, '')) + octet_length(coalesce(secret_name, '')) + octet_length(coalesce(slug, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM event_endpoints GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(correlation_key, '')) + octet_length(coalesce(event_id, '')) + octet_length(coalesce(event_type, '')) + octet_length(coalesce(payload, ''::bytea)) + octet_length(coalesce(source, '')) + octet_length(coalesce(subject, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((octet_length(coalesce(last_error, '')))::bigint)::bigint AS log_bytes FROM inbox_events GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(subject, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM memberships GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(slug, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM namespaces GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(content, ''::bytea)))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM payloads GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(reason, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM publication_decisions GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(release_notes, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM publication_requests GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(correlation_id, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM runs GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(content, ''::bytea)) + octet_length(coalesce(sbom, ''::bytea)))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM script_artifacts GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((0)::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((octet_length(coalesce(log, '')))::bigint)::bigint AS log_bytes FROM script_executions GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(inferred_input_schema, ''::bytea)) + octet_length(coalesce(inferred_output_schema, ''::bytea)) + octet_length(coalesce(input, ''::bytea)) + octet_length(coalesce(input_schema, ''::bytea)) + octet_length(coalesce(lockfile, '')) + octet_length(coalesce(output_schema, ''::bytea)) + octet_length(coalesce(policy, '')) + octet_length(coalesce(result, ''::bytea)) + octet_length(coalesce(source, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((octet_length(coalesce(log, '')))::bigint)::bigint AS log_bytes FROM script_test_runs GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(input_schema, ''::bytea)) + octet_length(coalesce(lockfile, '')) + octet_length(coalesce(output_schema, ''::bytea)) + octet_length(coalesce(policy, '')) + octet_length(coalesce(source, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((octet_length(coalesce(build_log, '')) + octet_length(coalesce(failure, '')))::bigint)::bigint AS log_bytes FROM script_versions GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(input_schema, ''::bytea)) + octet_length(coalesce(lockfile, '')) + octet_length(coalesce(name, '')) + octet_length(coalesce(output_schema, ''::bytea)) + octet_length(coalesce(policy, '')) + octet_length(coalesce(source, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM scripts GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(subject, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM secret_access GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(ciphertext, ''::bytea)))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM secret_versions GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(name, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM secrets GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(child_input, ''::bytea)) + octet_length(coalesce(correlation_key, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM steps GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(destination_settings::text, '')) + octet_length(coalesce(to_jsonb(kinds)::text, '')) + octet_length(coalesce(name, '')) + octet_length(coalesce(secret_name, '')) + octet_length(coalesce(url, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM webhook_subscriptions GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(pattern, '')) + octet_length(coalesce(subject, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM worker_bindings GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(reason, '')) + octet_length(coalesce(subject, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM worker_drains GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(cron, '')) + octet_length(coalesce(filter, '')) + octet_length(coalesce(match_source, '')) + octet_length(coalesce(match_type, '')) + octet_length(coalesce(timezone, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM workflow_triggers GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(document, ''::bytea)) + octet_length(coalesce(release_notes, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM workflow_versions GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(draft::text, '')) + octet_length(coalesce(name, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM workflows GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum((octet_length(coalesce(slug, '')))::bigint)::bigint AS content_bytes, 0::bigint AS reserved_bytes, sum((0)::bigint)::bigint AS log_bytes FROM workspaces GROUP BY tenant_id
UNION ALL
    SELECT tenant_id, sum(CASE WHEN uploaded_at IS NOT NULL THEN size_bytes ELSE 0 END)::bigint, sum(CASE WHEN uploaded_at IS NULL THEN reservation_bytes ELSE 0 END)::bigint, 0::bigint FROM blobs GROUP BY tenant_id
), totals AS (
    SELECT tenant_id, coalesce(sum(content_bytes),0)::bigint content_bytes, coalesce(sum(reserved_bytes),0)::bigint reserved_bytes, coalesce(sum(log_bytes),0)::bigint log_bytes FROM measured GROUP BY tenant_id
)
UPDATE tenant_retained_usage u SET content_bytes=t.content_bytes, content_reserved_bytes=t.reserved_bytes, log_bytes=t.log_bytes FROM totals t WHERE u.tenant_id=t.tenant_id;

CREATE TRIGGER connector_dead_letters_quota_delta AFTER INSERT OR UPDATE OR DELETE ON connector_dead_letters FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'body', 'content', 'canonical_json_utf8', 'headers', 'content', 'octet_length_utf8', 'position');
ALTER TABLE connector_dead_letters ENABLE ALWAYS TRIGGER connector_dead_letters_quota_delta;
CREATE CONSTRAINT TRIGGER connector_dead_letters_quota_flush AFTER INSERT OR UPDATE OR DELETE ON connector_dead_letters DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE connector_dead_letters ENABLE ALWAYS TRIGGER connector_dead_letters_quota_flush;
CREATE TRIGGER connector_offsets_quota_delta AFTER INSERT OR UPDATE OR DELETE ON connector_offsets FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'stream');
ALTER TABLE connector_offsets ENABLE ALWAYS TRIGGER connector_offsets_quota_delta;
CREATE CONSTRAINT TRIGGER connector_offsets_quota_flush AFTER INSERT OR UPDATE OR DELETE ON connector_offsets DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE connector_offsets ENABLE ALWAYS TRIGGER connector_offsets_quota_flush;
CREATE TRIGGER egress_policies_quota_delta AFTER INSERT OR UPDATE OR DELETE ON egress_policies FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'canonical_json_array_utf8', 'ports', 'content', 'canonical_json_array_utf8', 'rules');
ALTER TABLE egress_policies ENABLE ALWAYS TRIGGER egress_policies_quota_delta;
CREATE CONSTRAINT TRIGGER egress_policies_quota_flush AFTER INSERT OR UPDATE OR DELETE ON egress_policies DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE egress_policies ENABLE ALWAYS TRIGGER egress_policies_quota_flush;
CREATE TRIGGER event_connectors_quota_delta AFTER INSERT OR UPDATE OR DELETE ON event_connectors FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'default_event_type', 'log', 'octet_length_utf8', 'last_error', 'content', 'octet_length_utf8', 'secret_name', 'content', 'canonical_json_utf8', 'settings', 'content', 'octet_length_utf8', 'slug');
ALTER TABLE event_connectors ENABLE ALWAYS TRIGGER event_connectors_quota_delta;
CREATE CONSTRAINT TRIGGER event_connectors_quota_flush AFTER INSERT OR UPDATE OR DELETE ON event_connectors DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE event_connectors ENABLE ALWAYS TRIGGER event_connectors_quota_flush;
CREATE TRIGGER event_endpoints_quota_delta AFTER INSERT OR UPDATE OR DELETE ON event_endpoints FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'default_event_type', 'content', 'octet_length_utf8', 'mtls_certificate_sha256', 'content', 'octet_length_utf8', 'oidc_profile', 'content', 'octet_length_utf8', 'oidc_subject', 'content', 'octet_length_utf8', 'secret_name', 'content', 'octet_length_utf8', 'slug');
ALTER TABLE event_endpoints ENABLE ALWAYS TRIGGER event_endpoints_quota_delta;
CREATE CONSTRAINT TRIGGER event_endpoints_quota_flush AFTER INSERT OR UPDATE OR DELETE ON event_endpoints DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE event_endpoints ENABLE ALWAYS TRIGGER event_endpoints_quota_flush;
CREATE TRIGGER inbox_events_quota_delta AFTER INSERT OR UPDATE OR DELETE ON inbox_events FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'correlation_key', 'content', 'octet_length_utf8', 'event_id', 'content', 'octet_length_utf8', 'event_type', 'log', 'octet_length_utf8', 'last_error', 'content', 'octet_length_bytea', 'payload', 'content', 'octet_length_utf8', 'source', 'content', 'octet_length_utf8', 'subject');
ALTER TABLE inbox_events ENABLE ALWAYS TRIGGER inbox_events_quota_delta;
CREATE CONSTRAINT TRIGGER inbox_events_quota_flush AFTER INSERT OR UPDATE OR DELETE ON inbox_events DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE inbox_events ENABLE ALWAYS TRIGGER inbox_events_quota_flush;
CREATE TRIGGER memberships_quota_delta AFTER INSERT OR UPDATE OR DELETE ON memberships FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'subject');
ALTER TABLE memberships ENABLE ALWAYS TRIGGER memberships_quota_delta;
CREATE CONSTRAINT TRIGGER memberships_quota_flush AFTER INSERT OR UPDATE OR DELETE ON memberships DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE memberships ENABLE ALWAYS TRIGGER memberships_quota_flush;
CREATE TRIGGER namespaces_quota_delta AFTER INSERT OR UPDATE OR DELETE ON namespaces FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'slug');
ALTER TABLE namespaces ENABLE ALWAYS TRIGGER namespaces_quota_delta;
CREATE CONSTRAINT TRIGGER namespaces_quota_flush AFTER INSERT OR UPDATE OR DELETE ON namespaces DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE namespaces ENABLE ALWAYS TRIGGER namespaces_quota_flush;
CREATE TRIGGER payloads_quota_delta AFTER INSERT OR UPDATE OR DELETE ON payloads FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'content');
ALTER TABLE payloads ENABLE ALWAYS TRIGGER payloads_quota_delta;
CREATE CONSTRAINT TRIGGER payloads_quota_flush AFTER INSERT OR UPDATE OR DELETE ON payloads DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE payloads ENABLE ALWAYS TRIGGER payloads_quota_flush;
CREATE TRIGGER publication_decisions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON publication_decisions FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'reason');
ALTER TABLE publication_decisions ENABLE ALWAYS TRIGGER publication_decisions_quota_delta;
CREATE CONSTRAINT TRIGGER publication_decisions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON publication_decisions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE publication_decisions ENABLE ALWAYS TRIGGER publication_decisions_quota_flush;
CREATE TRIGGER publication_requests_quota_delta AFTER INSERT OR UPDATE OR DELETE ON publication_requests FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'release_notes');
ALTER TABLE publication_requests ENABLE ALWAYS TRIGGER publication_requests_quota_delta;
CREATE CONSTRAINT TRIGGER publication_requests_quota_flush AFTER INSERT OR UPDATE OR DELETE ON publication_requests DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE publication_requests ENABLE ALWAYS TRIGGER publication_requests_quota_flush;
CREATE TRIGGER runs_quota_delta AFTER INSERT OR UPDATE OR DELETE ON runs FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'correlation_id');
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_quota_delta;
CREATE CONSTRAINT TRIGGER runs_quota_flush AFTER INSERT OR UPDATE OR DELETE ON runs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_quota_flush;
CREATE TRIGGER script_artifacts_quota_delta AFTER INSERT OR UPDATE OR DELETE ON script_artifacts FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'content', 'content', 'octet_length_bytea', 'sbom');
ALTER TABLE script_artifacts ENABLE ALWAYS TRIGGER script_artifacts_quota_delta;
CREATE CONSTRAINT TRIGGER script_artifacts_quota_flush AFTER INSERT OR UPDATE OR DELETE ON script_artifacts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE script_artifacts ENABLE ALWAYS TRIGGER script_artifacts_quota_flush;
CREATE TRIGGER script_executions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON script_executions FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('log', 'octet_length_utf8', 'log');
ALTER TABLE script_executions ENABLE ALWAYS TRIGGER script_executions_quota_delta;
CREATE CONSTRAINT TRIGGER script_executions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON script_executions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE script_executions ENABLE ALWAYS TRIGGER script_executions_quota_flush;
CREATE TRIGGER script_test_runs_quota_delta AFTER INSERT OR UPDATE OR DELETE ON script_test_runs FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'inferred_input_schema', 'content', 'octet_length_bytea', 'inferred_output_schema', 'content', 'octet_length_bytea', 'input', 'content', 'octet_length_bytea', 'input_schema', 'content', 'octet_length_utf8', 'lockfile', 'log', 'octet_length_utf8', 'log', 'content', 'octet_length_bytea', 'output_schema', 'content', 'octet_length_utf8', 'policy', 'content', 'octet_length_bytea', 'result', 'content', 'octet_length_utf8', 'source');
ALTER TABLE script_test_runs ENABLE ALWAYS TRIGGER script_test_runs_quota_delta;
CREATE CONSTRAINT TRIGGER script_test_runs_quota_flush AFTER INSERT OR UPDATE OR DELETE ON script_test_runs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE script_test_runs ENABLE ALWAYS TRIGGER script_test_runs_quota_flush;
CREATE TRIGGER script_versions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON script_versions FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('log', 'octet_length_utf8', 'build_log', 'log', 'octet_length_utf8', 'failure', 'content', 'octet_length_bytea', 'input_schema', 'content', 'octet_length_utf8', 'lockfile', 'content', 'octet_length_bytea', 'output_schema', 'content', 'octet_length_utf8', 'policy', 'content', 'octet_length_utf8', 'source');
ALTER TABLE script_versions ENABLE ALWAYS TRIGGER script_versions_quota_delta;
CREATE CONSTRAINT TRIGGER script_versions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON script_versions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE script_versions ENABLE ALWAYS TRIGGER script_versions_quota_flush;
CREATE TRIGGER scripts_quota_delta AFTER INSERT OR UPDATE OR DELETE ON scripts FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'input_schema', 'content', 'octet_length_utf8', 'lockfile', 'content', 'octet_length_utf8', 'name', 'content', 'octet_length_bytea', 'output_schema', 'content', 'octet_length_utf8', 'policy', 'content', 'octet_length_utf8', 'source');
ALTER TABLE scripts ENABLE ALWAYS TRIGGER scripts_quota_delta;
CREATE CONSTRAINT TRIGGER scripts_quota_flush AFTER INSERT OR UPDATE OR DELETE ON scripts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE scripts ENABLE ALWAYS TRIGGER scripts_quota_flush;
CREATE TRIGGER secret_access_quota_delta AFTER INSERT OR UPDATE OR DELETE ON secret_access FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'subject');
ALTER TABLE secret_access ENABLE ALWAYS TRIGGER secret_access_quota_delta;
CREATE CONSTRAINT TRIGGER secret_access_quota_flush AFTER INSERT OR UPDATE OR DELETE ON secret_access DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE secret_access ENABLE ALWAYS TRIGGER secret_access_quota_flush;
CREATE TRIGGER secret_versions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON secret_versions FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'ciphertext');
ALTER TABLE secret_versions ENABLE ALWAYS TRIGGER secret_versions_quota_delta;
CREATE CONSTRAINT TRIGGER secret_versions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON secret_versions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE secret_versions ENABLE ALWAYS TRIGGER secret_versions_quota_flush;
CREATE TRIGGER secrets_quota_delta AFTER INSERT OR UPDATE OR DELETE ON secrets FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'name');
ALTER TABLE secrets ENABLE ALWAYS TRIGGER secrets_quota_delta;
CREATE CONSTRAINT TRIGGER secrets_quota_flush AFTER INSERT OR UPDATE OR DELETE ON secrets DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE secrets ENABLE ALWAYS TRIGGER secrets_quota_flush;
CREATE TRIGGER steps_quota_delta AFTER INSERT OR UPDATE OR DELETE ON steps FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'child_input', 'content', 'octet_length_utf8', 'correlation_key');
ALTER TABLE steps ENABLE ALWAYS TRIGGER steps_quota_delta;
CREATE CONSTRAINT TRIGGER steps_quota_flush AFTER INSERT OR UPDATE OR DELETE ON steps DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE steps ENABLE ALWAYS TRIGGER steps_quota_flush;
CREATE TRIGGER webhook_subscriptions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON webhook_subscriptions FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'canonical_json_utf8', 'destination_settings', 'content', 'canonical_json_array_utf8', 'kinds', 'content', 'octet_length_utf8', 'name', 'content', 'octet_length_utf8', 'secret_name', 'content', 'octet_length_utf8', 'url');
ALTER TABLE webhook_subscriptions ENABLE ALWAYS TRIGGER webhook_subscriptions_quota_delta;
CREATE CONSTRAINT TRIGGER webhook_subscriptions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON webhook_subscriptions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE webhook_subscriptions ENABLE ALWAYS TRIGGER webhook_subscriptions_quota_flush;
CREATE TRIGGER worker_bindings_quota_delta AFTER INSERT OR UPDATE OR DELETE ON worker_bindings FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'pattern', 'content', 'octet_length_utf8', 'subject');
ALTER TABLE worker_bindings ENABLE ALWAYS TRIGGER worker_bindings_quota_delta;
CREATE CONSTRAINT TRIGGER worker_bindings_quota_flush AFTER INSERT OR UPDATE OR DELETE ON worker_bindings DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE worker_bindings ENABLE ALWAYS TRIGGER worker_bindings_quota_flush;
CREATE TRIGGER worker_drains_quota_delta AFTER INSERT OR UPDATE OR DELETE ON worker_drains FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'reason', 'content', 'octet_length_utf8', 'subject');
ALTER TABLE worker_drains ENABLE ALWAYS TRIGGER worker_drains_quota_delta;
CREATE CONSTRAINT TRIGGER worker_drains_quota_flush AFTER INSERT OR UPDATE OR DELETE ON worker_drains DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE worker_drains ENABLE ALWAYS TRIGGER worker_drains_quota_flush;
CREATE TRIGGER workflow_triggers_quota_delta AFTER INSERT OR UPDATE OR DELETE ON workflow_triggers FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'cron', 'content', 'octet_length_utf8', 'filter', 'content', 'octet_length_utf8', 'match_source', 'content', 'octet_length_utf8', 'match_type', 'content', 'octet_length_utf8', 'timezone');
ALTER TABLE workflow_triggers ENABLE ALWAYS TRIGGER workflow_triggers_quota_delta;
CREATE CONSTRAINT TRIGGER workflow_triggers_quota_flush AFTER INSERT OR UPDATE OR DELETE ON workflow_triggers DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE workflow_triggers ENABLE ALWAYS TRIGGER workflow_triggers_quota_flush;
CREATE TRIGGER workflow_versions_quota_delta AFTER INSERT OR UPDATE OR DELETE ON workflow_versions FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_bytea', 'document', 'content', 'octet_length_utf8', 'release_notes');
ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_quota_delta;
CREATE CONSTRAINT TRIGGER workflow_versions_quota_flush AFTER INSERT OR UPDATE OR DELETE ON workflow_versions DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE workflow_versions ENABLE ALWAYS TRIGGER workflow_versions_quota_flush;
CREATE TRIGGER workflows_quota_delta AFTER INSERT OR UPDATE OR DELETE ON workflows FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'canonical_json_utf8', 'draft', 'content', 'octet_length_utf8', 'name');
ALTER TABLE workflows ENABLE ALWAYS TRIGGER workflows_quota_delta;
CREATE CONSTRAINT TRIGGER workflows_quota_flush AFTER INSERT OR UPDATE OR DELETE ON workflows DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE workflows ENABLE ALWAYS TRIGGER workflows_quota_flush;
CREATE TRIGGER workspaces_quota_delta AFTER INSERT OR UPDATE OR DELETE ON workspaces FOR EACH ROW EXECUTE FUNCTION sagat_quota_row_delta('content', 'octet_length_utf8', 'slug');
ALTER TABLE workspaces ENABLE ALWAYS TRIGGER workspaces_quota_delta;
CREATE CONSTRAINT TRIGGER workspaces_quota_flush AFTER INSERT OR UPDATE OR DELETE ON workspaces DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE workspaces ENABLE ALWAYS TRIGGER workspaces_quota_flush;
CREATE TRIGGER blobs_quota_delta AFTER INSERT OR UPDATE OR DELETE ON blobs FOR EACH ROW EXECUTE FUNCTION sagat_quota_blob_delta();
ALTER TABLE blobs ENABLE ALWAYS TRIGGER blobs_quota_delta;
CREATE CONSTRAINT TRIGGER blobs_quota_flush AFTER INSERT OR UPDATE OR DELETE ON blobs DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION sagat_quota_flush_deltas();
ALTER TABLE blobs ENABLE ALWAYS TRIGGER blobs_quota_flush;

DO $grants$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app') THEN
        REVOKE ALL ON tenant_retained_usage, quota_transaction_deltas, quota_accounting_control
            FROM sagat_app;
        GRANT SELECT ON tenant_retained_usage TO sagat_app;
        GRANT SELECT, INSERT, UPDATE ON quota_rate_buckets TO sagat_app;
        REVOKE ALL ON FUNCTION sagat_quota_measure(jsonb, text, text, boolean),
            sagat_quota_add_delta(uuid, bigint, bigint, bigint), sagat_quota_row_delta(),
            sagat_quota_blob_delta(), sagat_quota_flush_deltas(),
            sagat_blob_reservation_state(), sagat_quota_provision_tenant_usage() FROM sagat_app;
        GRANT EXECUTE ON FUNCTION sagat_quota_lock_log_usage(uuid) TO sagat_app;
    END IF;
END
$grants$;
