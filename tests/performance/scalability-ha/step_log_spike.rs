//! The step-log ingest spike of design 041 (T041-31): the SQL of the ingest transaction (T1 to T10 of
//! `specs/041-step-logs/data-model.md` §8) against a scratch schema `spike_041` that holds the tables of
//! data-model.md §2, in the benchmark's own disposable database. Nothing here reaches a migration.
//!
//! The line table carries the two triggers ADR-0027 generates, charging its text columns to the
//! existing `log` class, so that every commit takes the log policy fence and the tenant usage row as
//! the `step_log` class will (the class itself does not exist before T041-5). The step-log quota
//! resources do not exist before T041-4 either: their policy fences are taken by their keys and their
//! defaults are used. A session is matched as T3 says except for the minor, which is not 1.7 yet.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sagat_application::entropy::Token;
use sagat_application::worker::ClaimedTask;
use sqlx::{PgPool, Row};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::support::engine::Session;

/// The application name an ingest transaction sets, so the sampler can tell its backends apart.
pub const INGEST_APPLICATION: &str = "spike041";
/// 2^62, the first server sequence (markers).
const MARKER_SEQUENCE: i64 = 1 << 62;
/// The defaults of data-model.md §6, since the resources do not exist yet.
const ATTEMPT_RECORDS: i64 = 10_000;
const ATTEMPT_BYTES: i64 = 1024 * 1024;
const RETENTION_DAYS: i32 = 14;
const RESOURCES: [&str; 3] = [
    "step_log_attempt_records",
    "step_log_attempt_bytes",
    "step_log_retention_days",
];

/// The DDL of data-model.md §2 in the scratch schema, with the quota triggers on the line table.
pub async fn install(pool: &PgPool) {
    let statements = [
        "DROP SCHEMA IF EXISTS spike_041 CASCADE",
        "CREATE SCHEMA spike_041",
        r"CREATE TABLE spike_041.step_log_streams (
            tenant_id uuid NOT NULL,
            stream_id bigint GENERATED ALWAYS AS IDENTITY,
            public_id uuid NOT NULL,
            namespace_id uuid NOT NULL,
            run_id uuid NOT NULL,
            step_id uuid,
            node text NOT NULL,
            instance_path_nodes text[],
            instance_path_indices integer[],
            attempt integer NOT NULL,
            generation bigint,
            source text NOT NULL DEFAULT 'worker',
            records integer NOT NULL DEFAULT 0,
            bytes bigint NOT NULL DEFAULT 0,
            last_sequence bigint NOT NULL DEFAULT 0,
            ceiling_sequence bigint,
            next_marker_sequence bigint NOT NULL DEFAULT 4611686018427387904,
            dropped_by_worker bigint NOT NULL DEFAULT 0,
            dropped_by_server bigint NOT NULL DEFAULT 0,
            ceiling_reason text,
            truncated_records integer NOT NULL DEFAULT 0,
            retention_days smallint NOT NULL,
            final_at timestamptz,
            first_at timestamptz,
            last_at timestamptz NOT NULL,
            expires_at timestamptz NOT NULL,
            deleted_at timestamptz,
            removed_at timestamptz,
            purged_at timestamptz,
            PRIMARY KEY (tenant_id, stream_id),
            CONSTRAINT step_log_streams_public_uniq UNIQUE (tenant_id, public_id),
            CONSTRAINT step_log_streams_attempt_uniq UNIQUE (tenant_id, run_id, step_id, attempt, generation, source),
            CONSTRAINT step_log_streams_source_valid CHECK (source IN ('worker', 'script')),
            CONSTRAINT step_log_streams_counters_valid CHECK (records >= 0 AND bytes >= 0 AND last_sequence >= 0
                AND dropped_by_worker BETWEEN 0 AND 9007199254740992 AND dropped_by_server >= 0),
            CONSTRAINT step_log_streams_path_valid CHECK (instance_path_nodes IS NULL
                OR public.execution_instance_path_valid(instance_path_nodes, instance_path_indices)),
            CONSTRAINT step_log_streams_retention_valid CHECK (retention_days BETWEEN 1 AND 365)
        )",
        "ALTER TABLE spike_041.step_log_streams ADD COLUMN last_stored_xid xid8",
        "CREATE INDEX step_log_streams_run_idx ON spike_041.step_log_streams (tenant_id, run_id, stream_id)",
        "CREATE INDEX step_log_streams_expiry_idx ON spike_041.step_log_streams (expires_at) WHERE removed_at IS NULL",
        "CREATE INDEX step_log_streams_removal_idx ON spike_041.step_log_streams (tenant_id, stream_id) \
           WHERE removed_at IS NOT NULL AND purged_at IS NULL",
        r"CREATE FUNCTION spike_041.streams_guard() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
            IF NEW.tenant_id <> OLD.tenant_id OR NEW.stream_id <> OLD.stream_id OR NEW.public_id <> OLD.public_id
               OR NEW.run_id <> OLD.run_id OR NEW.attempt <> OLD.attempt
               OR NEW.retention_days <> OLD.retention_days OR NEW.last_sequence < OLD.last_sequence
               OR (OLD.final_at IS NOT NULL AND NEW.final_at IS NULL)
               OR (OLD.removed_at IS NOT NULL AND NEW.removed_at IS NULL) THEN
                RAISE EXCEPTION 'step log stream identity is immutable';
            END IF;
            RETURN NEW;
        END $$",
        "CREATE TRIGGER step_log_streams_guard BEFORE UPDATE ON spike_041.step_log_streams \
           FOR EACH ROW EXECUTE FUNCTION spike_041.streams_guard()",
        r"CREATE TABLE spike_041.step_log_lines (
            tenant_id uuid NOT NULL,
            stream_id bigint NOT NULL,
            sequence bigint NOT NULL,
            run_id uuid NOT NULL,
            stored_xid xid8 NOT NULL DEFAULT pg_current_xact_id(),
            stored_at timestamptz NOT NULL DEFAULT clock_timestamp(),
            worker_time timestamptz,
            level text NOT NULL,
            kind text NOT NULL,
            truncated boolean NOT NULL DEFAULT false,
            message text,
            exception text,
            fields text,
            marker_reason text,
            script_execution_id uuid,
            PRIMARY KEY (tenant_id, stream_id, sequence),
            CONSTRAINT step_log_lines_stream_fk FOREIGN KEY (tenant_id, stream_id)
                REFERENCES spike_041.step_log_streams (tenant_id, stream_id),
            CONSTRAINT step_log_lines_level_valid CHECK (level IN ('trace', 'debug', 'info', 'warn', 'error')),
            CONSTRAINT step_log_lines_sizes CHECK (octet_length(coalesce(message, '')) <= 8192
                AND octet_length(coalesce(exception, '')) <= 8192 AND octet_length(coalesce(fields, '')) <= 2048)
        )",
        "CREATE INDEX step_log_lines_read_idx ON spike_041.step_log_lines (tenant_id, run_id, stored_xid, sequence)",
        "CREATE INDEX step_log_lines_stream_idx ON spike_041.step_log_lines (tenant_id, stream_id, stored_xid, sequence)",
        "CREATE TRIGGER step_log_lines_quota_delta AFTER INSERT OR UPDATE OR DELETE ON spike_041.step_log_lines \
           FOR EACH ROW EXECUTE FUNCTION public.sagat_quota_row_delta(\
             'log', 'octet_length_utf8', 'message', 'log', 'octet_length_utf8', 'exception', \
             'log', 'octet_length_utf8', 'fields')",
        "ALTER TABLE spike_041.step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_quota_delta",
        "CREATE CONSTRAINT TRIGGER step_log_lines_quota_flush AFTER INSERT OR UPDATE OR DELETE \
           ON spike_041.step_log_lines DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
           EXECUTE FUNCTION public.sagat_quota_flush_deltas()",
        "ALTER TABLE spike_041.step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_quota_flush",
    ];
    for statement in statements {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("spike schema: {error}"));
    }
}

/// T041-35's second run (OD-2026-11-20-1): the scratch line table charged once per insert statement, its flush queued by
/// the transaction's delta row, and the carry as one function call (design 041 section 2a, data-model.md §3 and §8a).
#[cfg(feature = "perf-spike")]
pub async fn install_per_statement(pool: &PgPool) {
    let statements = [
        "DROP TRIGGER step_log_lines_quota_delta ON spike_041.step_log_lines",
        "DROP TRIGGER step_log_lines_quota_flush ON spike_041.step_log_lines",
        r"CREATE FUNCTION spike_041.lines_charge() RETURNS trigger LANGUAGE plpgsql
            SET search_path = pg_catalog, pg_temp AS $$
        DECLARE r record;
        BEGIN
            FOR r IN SELECT tenant_id,
                            sum(octet_length(coalesce(message, '')) + octet_length(coalesce(exception, ''))
                                + octet_length(coalesce(fields, '')) + 128)::bigint AS bytes
                       FROM inserted GROUP BY tenant_id LOOP
                PERFORM public.sagat_quota_add_delta(r.tenant_id, 0, 0, r.bytes);
            END LOOP;
            RETURN NULL;
        END $$",
        "CREATE TRIGGER step_log_lines_quota_insert AFTER INSERT ON spike_041.step_log_lines \
           REFERENCING NEW TABLE AS inserted FOR EACH STATEMENT EXECUTE FUNCTION spike_041.lines_charge()",
        "ALTER TABLE spike_041.step_log_lines ENABLE ALWAYS TRIGGER step_log_lines_quota_insert",
        "CREATE CONSTRAINT TRIGGER quota_transaction_deltas_spike_flush AFTER INSERT ON public.quota_transaction_deltas \
           DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.log_delta <> 0) \
           EXECUTE FUNCTION public.sagat_quota_flush_deltas()",
        "ALTER TABLE public.quota_transaction_deltas ENABLE ALWAYS TRIGGER quota_transaction_deltas_spike_flush",
        r"CREATE FUNCTION spike_041.step_log_carry(p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid,
                p_node text, p_attempt integer, p_generation bigint, p_keys text[], p_stream_key text,
                p_resources text[], p_messages text[], p_application text)
            RETURNS text LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, public, pg_temp AS $$
        DECLARE
            k text; r record;
            v_records bigint := 10000; v_bytes bigint := 1048576; v_retention integer := 14;
            v_records_scoped boolean := false; v_bytes_scoped boolean := false; v_retention_scoped boolean := false;
            v_usage bigint; v_stream bigint; v_last bigint; v_count bigint; v_sum bigint;
            v_final boolean; v_removed boolean; v_in_order boolean; v_inserted integer; v_size bigint;
        BEGIN
            PERFORM set_config('application_name', p_application, true);
            -- CA: the fences and the stream lock, tried, never waited for.
            FOREACH k IN ARRAY p_keys LOOP
                IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(k, 0)) THEN RETURN 'contended'; END IF;
            END LOOP;
            IF NOT pg_try_advisory_xact_lock(hashtextextended(p_stream_key, 0)) THEN RETURN 'contended'; END IF;
            -- CB: the policy, read after the locks, and the tenant's allowance.
            FOR r IN SELECT resource, namespace_id, max_value FROM public.quota_limits
                      WHERE tenant_id = p_tenant AND resource = ANY (p_resources)
                        AND (namespace_id IS NULL OR namespace_id = p_namespace)
                     UNION ALL SELECT resource, NULL::uuid, max_value FROM public.quota_ceilings
                      WHERE tenant_id = p_tenant AND resource = ANY (p_resources) LOOP
                IF r.resource = p_resources[1] AND (r.namespace_id IS NOT NULL OR NOT v_records_scoped) THEN
                    v_records := r.max_value; v_records_scoped := r.namespace_id IS NOT NULL;
                ELSIF r.resource = p_resources[2] AND (r.namespace_id IS NOT NULL OR NOT v_bytes_scoped) THEN
                    v_bytes := r.max_value; v_bytes_scoped := r.namespace_id IS NOT NULL;
                ELSIF r.resource = p_resources[3] AND (r.namespace_id IS NOT NULL OR NOT v_retention_scoped) THEN
                    v_retention := r.max_value::integer; v_retention_scoped := r.namespace_id IS NOT NULL;
                END IF;
            END LOOP;
            SELECT log_bytes INTO v_usage FROM public.tenant_retained_usage WHERE tenant_id = p_tenant;
            IF v_records = 0 THEN RETURN 'off'; END IF;
            -- CC: a new stream, never waiting on a unique check.
            INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node,
                    instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at)
            SELECT p_tenant, gen_random_uuid(), p_namespace, p_run, p_step, p_node, '{}'::text[], '{}'::integer[],
                   p_attempt, p_generation, v_retention, clock_timestamp(),
                   clock_timestamp() + make_interval(days => v_retention)
             WHERE NOT EXISTS (SELECT 1 FROM spike_041.step_log_streams WHERE tenant_id = p_tenant AND run_id = p_run
                                  AND step_id = p_step AND attempt = p_attempt AND generation = p_generation
                                  AND source = 'worker')
            ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO NOTHING
            RETURNING stream_id INTO v_stream;
            IF v_stream IS NOT NULL THEN
                v_last := 0; v_count := 0; v_sum := 0;
            ELSE
                -- CC': the existing stream, locked only if free.
                SELECT stream_id, last_sequence, records, bytes, final_at IS NOT NULL,
                       (removed_at IS NOT NULL OR expires_at <= clock_timestamp()),
                       (last_stored_xid IS NULL OR last_stored_xid < pg_current_xact_id())
                  INTO v_stream, v_last, v_count, v_sum, v_final, v_removed, v_in_order
                  FROM spike_041.step_log_streams
                 WHERE tenant_id = p_tenant AND run_id = p_run AND step_id = p_step AND attempt = p_attempt
                   AND generation = p_generation AND source = 'worker'
                   FOR UPDATE SKIP LOCKED;
                IF NOT FOUND THEN RETURN 'contended'; END IF;
                IF v_final OR v_removed OR NOT v_in_order THEN RETURN 'neither'; END IF;
            END IF;
            -- The plan, inline for the spike's two cases: every record kept when it fits the ceilings.
            IF v_count + cardinality(p_messages) > v_records THEN RETURN 'neither'; END IF;
            -- CD: the kept lines; the statement trigger charges them once.
            WITH ins AS (
                INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                SELECT p_tenant, v_stream, v_last + o, p_run, clock_timestamp(), 'info', 'line', m
                  FROM unnest(p_messages) WITH ORDINALITY AS u(m, o)
                ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING
                RETURNING octet_length(message) AS n)
            SELECT count(*)::integer, coalesce(sum(n), 0)::bigint INTO v_inserted, v_size FROM ins;
            IF v_sum + v_size > v_bytes THEN RAISE EXCEPTION 'spike carry over the attempt bytes'; END IF;
            -- CE: the counters, the final mark and the order guard.
            UPDATE spike_041.step_log_streams
               SET last_sequence = GREATEST(last_sequence, v_last + cardinality(p_messages)),
                   records = records + v_inserted, bytes = bytes + v_size,
                   first_at = coalesce(first_at, clock_timestamp()), last_at = clock_timestamp(),
                   expires_at = clock_timestamp() + make_interval(days => retention_days::integer),
                   final_at = clock_timestamp(),
                   last_stored_xid = CASE WHEN v_inserted > 0
                       THEN GREATEST(coalesce(last_stored_xid, '0'::xid8), pg_current_xact_id()) ELSE last_stored_xid END
             WHERE tenant_id = p_tenant AND stream_id = v_stream;
            RETURN 'written';
        END $$",
    ];
    for statement in statements {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("spike schema (second run): {error}"));
    }
}

/// T041-35's third run (ADR-0097, D041-108, D041-112): after `install_per_statement`, the step-log charge goes to the
/// tenant's exact leases (data-model.md §3b) instead of the transaction's delta row and the finalizer at commit: the
/// statement trigger calls `spike_041.step_log_charge` once per tenant, which takes the three class fences shared, reads
/// the limit and, without a lock, the folded usage and the leased bytes, and, unless they already pass the limit, adds the
/// bytes to a lease it can lock at once (`SKIP LOCKED`); otherwise `spike_041.step_log_refill` locks the tenant usage row
/// (`NOWAIT` when the transaction set `sagat.step_log_charge_nowait`, as a carry does), reclaims the idle leases near the
/// limit and grants one. `spike_041.step_log_release` is the release (a purge's), applied to the folded usage under the
/// usage row only, never waiting for a lease. `step_log_bytes` is the folded usage (it may be negative), the scratch
/// column `step_log_leased_bytes` the leased bytes.
#[cfg(feature = "perf-spike")]
#[allow(clippy::too_many_lines)]
pub async fn install_leases(pool: &PgPool) {
    let statements = [
        "DROP TRIGGER quota_transaction_deltas_spike_flush ON public.quota_transaction_deltas",
        "ALTER TABLE public.tenant_retained_usage DROP CONSTRAINT IF EXISTS tenant_retained_usage_step_log_nonnegative",
        "ALTER TABLE public.tenant_retained_usage ADD COLUMN step_log_leased_bytes bigint NOT NULL DEFAULT 0 \
           CHECK (step_log_leased_bytes >= 0)",
        // D041-117: one lease a writer, keyed by the server backend that charges (`pg_backend_pid()`): a backend runs
        // one transaction at a time, so no writer ever meets another's lock on its lease.
        "CREATE TABLE spike_041.step_log_quota_leases (tenant_id uuid NOT NULL \
           REFERENCES public.tenant_retained_usage (tenant_id) ON DELETE CASCADE, \
           writer integer NOT NULL, granted bigint NOT NULL DEFAULT 0, used bigint NOT NULL DEFAULT 0, \
           charged_at timestamptz NOT NULL DEFAULT clock_timestamp(), \
           PRIMARY KEY (tenant_id, writer), CHECK (0 <= used AND used <= granted)) WITH (fillfactor = 50)",
        // Refills that outwaited their bound (D041-117): counted outside the transaction they fail.
        "CREATE SEQUENCE spike_041.step_log_refill_wait_timeouts",
        r"CREATE FUNCTION spike_041.step_log_limit(p_tenant uuid) RETURNS bigint LANGUAGE sql STABLE
            SET search_path = pg_catalog, pg_temp AS $$
            SELECT least(
                coalesce((SELECT max_value FROM public.quota_limits
                           WHERE tenant_id = p_tenant AND namespace_id IS NULL AND resource = 'tenant_step_log_bytes'),
                         (SELECT max_value FROM public.quota_ceilings
                           WHERE tenant_id = p_tenant AND resource = 'tenant_step_log_bytes'),
                         1073741824),
                coalesce((SELECT max_value FROM public.quota_ceilings
                           WHERE tenant_id = p_tenant AND resource = 'tenant_step_log_bytes'),
                         1073741824))
        $$",
        // The usage S + Σ used, read without a lock in one snapshot (the allowance and the quota view).
        r"CREATE FUNCTION spike_041.step_log_usage(p_tenant uuid) RETURNS bigint LANGUAGE sql STABLE
            SET search_path = pg_catalog, pg_temp AS $$
            SELECT coalesce((SELECT step_log_bytes FROM public.tenant_retained_usage WHERE tenant_id = p_tenant), 0)
                 + coalesce((SELECT sum(used) FROM spike_041.step_log_quota_leases WHERE tenant_id = p_tenant), 0)::bigint
        $$",
        // The refill's bound b (D041-118): every other backend the server admits queued ahead on the usage row, each for
        // a finalizer's p99 hold of 4.5 ms (D041-106), plus one refill holder (the carry's fast-path p99 at 8 × 128,
        // 32.7 ms, run 44793160071) and its commit (4.5 ms), rounded up to whole seconds: 2 s at 400 connections.
        r"CREATE FUNCTION spike_041.step_log_refill_bound() RETURNS interval LANGUAGE sql STABLE
            SET search_path = pg_catalog, pg_temp AS $$
            SELECT make_interval(secs => ceil(((current_setting('max_connections')::int - 1) * 45 + 327 + 45) / 10000.0))
        $$",
        // The refill of the writer's own lease (D041-117). It waits for the usage row, at most the bound b
        // (`sagat.step_log_refill_wait` when a test sets it; a lower `lock_timeout` of the caller kept), and a wait past
        // it fails the transaction, counted, as `55P03` named `tenant_step_log_refill_wait`: never a mark-only answer.
        // Lock order: the step-log fence (shared, the caller's), the usage row, the writer's own lease; other writers'
        // leases only `SKIP LOCKED`.
        r"CREATE FUNCTION spike_041.step_log_refill(p_tenant uuid, p_bytes bigint) RETURNS void
            LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
        DECLARE v_limit bigint; v_s bigint; v_g bigint; v_free bigint; v_grant bigint; v_others bigint;
                v_granted bigint; v_used bigint; r record; i integer; v_prev text; v_bound text;
        BEGIN
            -- The limit, read here after the fence the charge holds, never passed by a caller.
            v_limit := spike_041.step_log_limit(p_tenant);
            v_prev := current_setting('lock_timeout');
            v_bound := coalesce(nullif(current_setting('sagat.step_log_refill_wait', true), ''),
                                (extract(epoch FROM spike_041.step_log_refill_bound()) * 1000)::bigint || 'ms');
            IF v_prev = '0' OR v_prev::interval > v_bound::interval THEN
                PERFORM set_config('lock_timeout', v_bound, true);
            END IF;
            -- The usage row, locked first; created only when it does not exist yet.
            BEGIN
                FOR i IN 1..2 LOOP
                    SELECT step_log_bytes, step_log_leased_bytes INTO v_s, v_g
                      FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE;
                    EXIT WHEN FOUND;
                    INSERT INTO public.tenant_retained_usage (tenant_id) VALUES (p_tenant) ON CONFLICT (tenant_id) DO NOTHING;
                END LOOP;
            EXCEPTION WHEN lock_not_available THEN
                PERFORM nextval('spike_041.step_log_refill_wait_timeouts');
                RAISE EXCEPTION 'step-log refill outwaited its bound' USING ERRCODE = 'lock_not_available',
                      CONSTRAINT = 'tenant_step_log_refill_wait';
            END;
            PERFORM set_config('lock_timeout', v_prev, true);
            IF v_s IS NULL THEN RAISE EXCEPTION 'tenant usage row missing' USING ERRCODE = 'data_exception'; END IF;
            IF v_limit - v_s - v_g < greatest(p_bytes, 1048576) THEN
                -- Near or past the limit: every lease no open transaction holds gives its room back.
                FOR r IN SELECT writer, granted, used FROM spike_041.step_log_quota_leases
                          WHERE tenant_id = p_tenant AND writer <> pg_backend_pid() ORDER BY writer
                          FOR UPDATE SKIP LOCKED LOOP
                    v_s := v_s + r.used; v_g := v_g - r.granted;
                    UPDATE spike_041.step_log_quota_leases SET granted = 0, used = 0
                     WHERE tenant_id = p_tenant AND writer = r.writer;
                END LOOP;
            END IF;
            -- The writer's own lease: no other transaction can hold it now (a reclaim holds the usage row first).
            SELECT granted, used INTO v_granted, v_used FROM spike_041.step_log_quota_leases
             WHERE tenant_id = p_tenant AND writer = pg_backend_pid() FOR UPDATE;
            IF NOT FOUND THEN
                INSERT INTO spike_041.step_log_quota_leases (tenant_id, writer) VALUES (p_tenant, pg_backend_pid());
                v_granted := 0; v_used := 0;
            END IF;
            -- Fold the writer's lease whole (its used bytes to the folded usage, its room back to the free room), then
            -- grant it max(n, min(1 MiB, F / (2(A + 1)))), A the tenant's other leases: a grant leaves half of F.
            v_s := v_s + v_used; v_g := v_g - v_granted;
            v_free := v_limit - v_s - v_g;
            SELECT count(*) INTO v_others FROM spike_041.step_log_quota_leases
             WHERE tenant_id = p_tenant AND writer <> pg_backend_pid();
            v_grant := greatest(p_bytes, least(1048576, greatest(v_free, 0) / (2 * (v_others + 1))));
            IF v_grant > v_free THEN
                -- Held: the room missing is in leases open transactions hold (their room, read without a lock).
                IF v_grant <= v_free + coalesce((SELECT sum(granted - used) FROM spike_041.step_log_quota_leases
                                                  WHERE tenant_id = p_tenant AND writer <> pg_backend_pid()), 0) THEN
                    RAISE EXCEPTION 'tenant step-log quota held' USING ERRCODE = 'program_limit_exceeded',
                          CONSTRAINT = 'tenant_step_log_quota_held';
                END IF;
                RAISE EXCEPTION 'tenant step-log quota exceeded' USING ERRCODE = 'program_limit_exceeded',
                      CONSTRAINT = 'tenant_step_log_quota_exceeded';
            END IF;
            UPDATE spike_041.step_log_quota_leases
               SET granted = v_grant, used = p_bytes, charged_at = clock_timestamp()
             WHERE tenant_id = p_tenant AND writer = pg_backend_pid();
            UPDATE public.tenant_retained_usage SET step_log_bytes = v_s, step_log_leased_bytes = v_g + v_grant
             WHERE tenant_id = p_tenant;
        END $$",
        // A lease idle longer than `p_idle` is folded back and deleted (D041-117; the purge pass, one tenant a
        // transaction), skipping a lease an open transaction holds.
        r"CREATE FUNCTION spike_041.step_log_reclaim_idle(p_tenant uuid, p_idle interval) RETURNS integer
            LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
        DECLARE r record; v_n integer := 0;
        BEGIN
            PERFORM 1 FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE;
            FOR r IN SELECT writer, granted, used FROM spike_041.step_log_quota_leases
                      WHERE tenant_id = p_tenant AND charged_at < clock_timestamp() - p_idle
                      ORDER BY writer FOR UPDATE SKIP LOCKED LOOP
                UPDATE public.tenant_retained_usage
                   SET step_log_bytes = step_log_bytes + r.used, step_log_leased_bytes = step_log_leased_bytes - r.granted
                 WHERE tenant_id = p_tenant;
                DELETE FROM spike_041.step_log_quota_leases WHERE tenant_id = p_tenant AND writer = r.writer;
                v_n := v_n + 1;
            END LOOP;
            RETURN v_n;
        END $$",
        // The carry's fast path (D041-113): a lease with room for `p_bytes` locked for this transaction (no write), or
        // false; read after the fences the carry holds. The charge then finds that lease (its own lock is not skipped).
        r"CREATE FUNCTION spike_041.step_log_reserve(p_tenant uuid, p_bytes bigint) RETURNS boolean
            LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
        DECLARE v_s bigint; v_g bigint; v_lease integer;
        BEGIN
            -- Holding a lease always means holding the step-log fence (the reclaim's NOWAIT relies on it): tried here
            -- shared, at once when the carry already holds it (D041-114).
            IF p_bytes <= 0 OR NOT pg_try_advisory_xact_lock_shared(
                   hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0)) THEN
                RETURN false;
            END IF;
            SELECT step_log_bytes, step_log_leased_bytes INTO v_s, v_g
              FROM public.tenant_retained_usage WHERE tenant_id = p_tenant;
            IF coalesce(v_s, 0) + coalesce(v_g, 0) > spike_041.step_log_limit(p_tenant) THEN RETURN false; END IF;
            -- The writer's own lease (D041-117), tried: only a reclaim can hold it, and then the slow path decides.
            SELECT writer INTO v_lease FROM spike_041.step_log_quota_leases
             WHERE tenant_id = p_tenant AND writer = pg_backend_pid() AND granted - used >= p_bytes
               FOR UPDATE SKIP LOCKED;
            RETURN v_lease IS NOT NULL;
        END $$",
        r"CREATE FUNCTION spike_041.step_log_charge(p_tenant uuid, p_bytes bigint) RETURNS void
            LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
        DECLARE v_limit bigint; v_s bigint; v_g bigint; v_lease integer;
        BEGIN
            IF p_bytes <= 0 THEN RETURN; END IF;
            -- The class fences in ADR-0027's order (the select list is evaluated left to right).
            PERFORM pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_content_bytes:' || p_tenant::text, 0)),
                    pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_log_bytes:' || p_tenant::text, 0)),
                    pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0));
            v_limit := spike_041.step_log_limit(p_tenant);
            SELECT step_log_bytes, step_log_leased_bytes INTO v_s, v_g
              FROM public.tenant_retained_usage WHERE tenant_id = p_tenant;
            IF coalesce(v_s, 0) + coalesce(v_g, 0) <= v_limit THEN
                -- The writer's own lease with room (D041-117), locked at once or skipped (a reclaim holds it).
                UPDATE spike_041.step_log_quota_leases SET used = used + p_bytes, charged_at = clock_timestamp()
                 WHERE tenant_id = p_tenant AND writer = (
                       SELECT writer FROM spike_041.step_log_quota_leases
                        WHERE tenant_id = p_tenant AND writer = pg_backend_pid() AND granted - used >= p_bytes
                          FOR UPDATE SKIP LOCKED)
                RETURNING writer INTO v_lease;
                IF v_lease IS NOT NULL THEN RETURN; END IF;
            END IF;
            -- The writer's lease is short, or the room is past a lowered limit: the refill reclaims and decides.
            PERFORM spike_041.step_log_refill(p_tenant, p_bytes);
        END $$",
        r"CREATE FUNCTION spike_041.step_log_release(p_tenant uuid, p_bytes bigint) RETURNS void
            LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp AS $$
        DECLARE v_s bigint; v_used bigint;
        BEGIN
            PERFORM pg_advisory_xact_lock_shared(hashtextextended('quota-policy:tenant_step_log_bytes:' || p_tenant::text, 0));
            SELECT step_log_bytes INTO STRICT v_s FROM public.tenant_retained_usage WHERE tenant_id = p_tenant FOR UPDATE;
            -- The committed charges still in leases, read after the lock: the usage never goes below zero.
            SELECT coalesce(sum(used), 0)::bigint INTO v_used FROM spike_041.step_log_quota_leases WHERE tenant_id = p_tenant;
            IF v_s - p_bytes + v_used < 0 THEN
                RAISE EXCEPTION 'retained step-log usage underflow for tenant' USING ERRCODE = 'data_exception';
            END IF;
            UPDATE public.tenant_retained_usage SET step_log_bytes = v_s - p_bytes WHERE tenant_id = p_tenant;
        END $$",
        // A change of the tenant's step-log limit or ceiling reclaims every lease (ADR-0097 decision 6), by a trigger that
        // takes the step-log fence exclusively itself, so no lease is held by a charge meanwhile.
        r"CREATE FUNCTION spike_041.step_log_reclaim() RETURNS trigger LANGUAGE plpgsql
            SET search_path = pg_catalog, pg_temp AS $$
        DECLARE v_tenant uuid := CASE WHEN TG_OP = 'DELETE' THEN OLD.tenant_id ELSE NEW.tenant_id END; v_used bigint;
        BEGIN
            IF (CASE WHEN TG_OP = 'DELETE' THEN OLD.resource ELSE NEW.resource END) <> 'tenant_step_log_bytes' THEN
                RETURN NULL;
            END IF;
            -- The trigger takes the step-log fence exclusively itself (re-entrant when the editor holds it), so no charge
            -- holds a lease meanwhile; then the usage row, then the leases, locked before they are summed.
            PERFORM pg_advisory_xact_lock(hashtextextended('quota-policy:tenant_step_log_bytes:' || v_tenant::text, 0));
            PERFORM 1 FROM spike_041.step_log_quota_leases WHERE tenant_id = v_tenant ORDER BY writer FOR UPDATE NOWAIT;
            PERFORM 1 FROM public.tenant_retained_usage WHERE tenant_id = v_tenant FOR UPDATE;
            SELECT coalesce(sum(used), 0)::bigint INTO v_used FROM spike_041.step_log_quota_leases WHERE tenant_id = v_tenant;
            UPDATE public.tenant_retained_usage SET step_log_bytes = step_log_bytes + v_used, step_log_leased_bytes = 0
             WHERE tenant_id = v_tenant;
            UPDATE spike_041.step_log_quota_leases SET granted = 0, used = 0 WHERE tenant_id = v_tenant;
            RETURN NULL;
        END $$",
        "CREATE TRIGGER quota_limits_step_log_reclaim AFTER INSERT OR UPDATE OR DELETE ON public.quota_limits \
           FOR EACH ROW EXECUTE FUNCTION spike_041.step_log_reclaim()",
        "CREATE TRIGGER quota_ceilings_step_log_reclaim AFTER INSERT OR UPDATE OR DELETE ON public.quota_ceilings \
           FOR EACH ROW EXECUTE FUNCTION spike_041.step_log_reclaim()",
        r"CREATE OR REPLACE FUNCTION spike_041.lines_charge() RETURNS trigger LANGUAGE plpgsql
            SET search_path = pg_catalog, pg_temp AS $$
        DECLARE r record;
        BEGIN
            FOR r IN SELECT tenant_id,
                            sum(octet_length(coalesce(message, '')) + octet_length(coalesce(exception, ''))
                                + octet_length(coalesce(fields, '')) + 128)::bigint AS bytes
                       FROM inserted GROUP BY tenant_id ORDER BY tenant_id LOOP
                PERFORM spike_041.step_log_charge(r.tenant_id, r.bytes);
            END LOOP;
            RETURN NULL;
        END $$",
    ];
    for statement in statements {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("spike schema (third run): {error}"));
    }
}

/// T041-35's third run, the storage of D041-111 (after `install_per_statement`): the stream row loses its run index (a
/// run's streams are read through the prefix of the attempt key), its expiry index is keyed on the day of `expires_at`, a
/// stored generated column, so that an update that keeps the day changes no indexed column (HOT, with room left on each
/// page), and the carry writes a new stream once, with its counters, final mark, expiry and order guard, before its lines.
#[cfg(feature = "perf-spike")]
pub async fn install_third_run_storage(pool: &PgPool) {
    let statements = [
        "DROP INDEX spike_041.step_log_streams_expiry_idx",
        "ALTER TABLE spike_041.step_log_streams ADD COLUMN expires_day integer GENERATED ALWAYS AS \
           ((floor(extract(epoch FROM (expires_at - '1970-01-01 00:00:00+00'::timestamptz)) / 86400))::integer) STORED",
        "CREATE INDEX step_log_streams_expiry_idx ON spike_041.step_log_streams (expires_day) WHERE removed_at IS NULL",
        "ALTER TABLE spike_041.step_log_streams SET (fillfactor = 70)",
        // CC to CE: the writes of a carry, called on the fast path as they are, on the slow path inside a block.
        r"CREATE FUNCTION spike_041.step_log_carry_write(p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid,
                p_node text, p_attempt integer, p_generation bigint, p_messages text[], p_retention integer,
                p_records bigint, p_bytes bigint, p_kept integer, p_kept_bytes bigint)
            RETURNS text LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, public, pg_temp AS $$
        DECLARE v_stream bigint; v_last bigint; v_count bigint; v_sum bigint;
                v_final boolean; v_removed boolean; v_in_order boolean; v_inserted integer; v_size bigint;
        BEGIN
            -- CC: a new stream, written once with what the plan stores.
            INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node,
                    instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_sequence,
                    records, bytes, first_at, last_at, expires_at, final_at, last_stored_xid)
            SELECT p_tenant, gen_random_uuid(), p_namespace, p_run, p_step, p_node, '{}'::text[], '{}'::integer[],
                   p_attempt, p_generation, p_retention, p_kept, p_kept, p_kept_bytes,
                   CASE WHEN p_kept > 0 THEN clock_timestamp() END, clock_timestamp(),
                   clock_timestamp() + make_interval(days => p_retention), clock_timestamp(),
                   CASE WHEN p_kept > 0 THEN pg_current_xact_id() END
             WHERE NOT EXISTS (SELECT 1 FROM spike_041.step_log_streams WHERE tenant_id = p_tenant AND run_id = p_run
                                  AND step_id = p_step AND attempt = p_attempt AND generation = p_generation
                                  AND source = 'worker')
            ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO NOTHING
            RETURNING stream_id INTO v_stream;
            IF v_stream IS NOT NULL THEN
                INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                SELECT p_tenant, v_stream, o, p_run, clock_timestamp(), 'info', 'line', m
                  FROM unnest(p_messages) WITH ORDINALITY AS u(m, o);
                RETURN 'written';
            END IF;
            -- CC': the existing stream, locked only if free; then CD and CE.
            SELECT stream_id, last_sequence, records, bytes, final_at IS NOT NULL,
                   (removed_at IS NOT NULL OR expires_at <= clock_timestamp()),
                   (last_stored_xid IS NULL OR last_stored_xid < pg_current_xact_id())
              INTO v_stream, v_last, v_count, v_sum, v_final, v_removed, v_in_order
              FROM spike_041.step_log_streams
             WHERE tenant_id = p_tenant AND run_id = p_run AND step_id = p_step AND attempt = p_attempt
               AND generation = p_generation AND source = 'worker'
               FOR UPDATE SKIP LOCKED;
            IF NOT FOUND THEN RETURN 'contended'; END IF;
            IF v_final OR v_removed OR NOT v_in_order THEN RETURN 'neither'; END IF;
            IF v_count + cardinality(p_messages) > p_records THEN RETURN 'neither'; END IF;
            WITH ins AS (
                INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                SELECT p_tenant, v_stream, v_last + o, p_run, clock_timestamp(), 'info', 'line', m
                  FROM unnest(p_messages) WITH ORDINALITY AS u(m, o)
                ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING
                RETURNING octet_length(message) AS n)
            SELECT count(*)::integer, coalesce(sum(n), 0)::bigint INTO v_inserted, v_size FROM ins;
            IF v_sum + v_size > p_bytes THEN RAISE EXCEPTION 'spike carry over the attempt bytes'; END IF;
            UPDATE spike_041.step_log_streams
               SET last_sequence = GREATEST(last_sequence, v_last + cardinality(p_messages)),
                   records = records + v_inserted, bytes = bytes + v_size,
                   first_at = coalesce(first_at, clock_timestamp()), last_at = clock_timestamp(),
                   expires_at = clock_timestamp() + make_interval(days => retention_days::integer),
                   final_at = clock_timestamp(),
                   last_stored_xid = CASE WHEN v_inserted > 0
                       THEN GREATEST(coalesce(last_stored_xid, '0'::xid8), pg_current_xact_id()) ELSE last_stored_xid END
             WHERE tenant_id = p_tenant AND stream_id = v_stream;
            RETURN 'written';
        END $$",
        r"CREATE OR REPLACE FUNCTION spike_041.step_log_carry(p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid,
                p_node text, p_attempt integer, p_generation bigint, p_keys text[], p_stream_key text,
                p_resources text[], p_messages text[], p_application text)
            RETURNS text LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, public, pg_temp AS $$
        DECLARE
            k text; r record; v_state text; v_constraint text; v_stream bigint;
            v_records bigint := 10000; v_bytes bigint := 1048576; v_retention integer := 14;
            v_records_scoped boolean := false; v_bytes_scoped boolean := false; v_retention_scoped boolean := false;
            v_usage bigint; v_kept integer; v_kept_bytes bigint; v_charge bigint; v_cause text; v_outcome text;
        BEGIN
            PERFORM set_config('application_name', p_application, true);
            FOREACH k IN ARRAY p_keys LOOP
                IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(k, 0)) THEN RETURN 'contended'; END IF;
            END LOOP;
            IF NOT pg_try_advisory_xact_lock(hashtextextended(p_stream_key, 0)) THEN RETURN 'contended'; END IF;
            -- The tenant's quota class fences (L3), tried, so that CD's charge never waits for them (ADR-0097).
            FOREACH k IN ARRAY ARRAY['quota-policy:tenant_content_bytes:', 'quota-policy:tenant_log_bytes:',
                                     'quota-policy:tenant_step_log_bytes:'] LOOP
                IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(k || p_tenant::text, 0)) THEN
                    RETURN 'contended';
                END IF;
            END LOOP;
            FOR r IN SELECT resource, namespace_id, max_value FROM public.quota_limits
                      WHERE tenant_id = p_tenant AND resource = ANY (p_resources)
                        AND (namespace_id IS NULL OR namespace_id = p_namespace)
                     UNION ALL SELECT resource, NULL::uuid, max_value FROM public.quota_ceilings
                      WHERE tenant_id = p_tenant AND resource = ANY (p_resources) LOOP
                IF r.resource = p_resources[1] AND (r.namespace_id IS NOT NULL OR NOT v_records_scoped) THEN
                    v_records := r.max_value; v_records_scoped := r.namespace_id IS NOT NULL;
                ELSIF r.resource = p_resources[2] AND (r.namespace_id IS NOT NULL OR NOT v_bytes_scoped) THEN
                    v_bytes := r.max_value; v_bytes_scoped := r.namespace_id IS NOT NULL;
                ELSIF r.resource = p_resources[3] AND (r.namespace_id IS NOT NULL OR NOT v_retention_scoped) THEN
                    v_retention := r.max_value::integer; v_retention_scoped := r.namespace_id IS NOT NULL;
                END IF;
            END LOOP;
            v_usage := spike_041.step_log_usage(p_tenant);
            IF v_records = 0 THEN RETURN 'off'; END IF;
            -- The plan before any write: every record kept when it fits the attempt ceilings; its charge, measured as the
            -- statement trigger will measure it (text plus 128 a row).
            v_kept := cardinality(p_messages);
            SELECT coalesce(sum(octet_length(m)), 0)::bigint INTO v_kept_bytes FROM unnest(p_messages) AS u(m);
            IF v_kept > v_records OR v_kept_bytes > v_bytes THEN RETURN 'neither'; END IF;
            v_charge := v_kept_bytes + 128 * v_kept;
            -- The fast path (D041-113): a lease with room for the charge is locked for this transaction before any write,
            -- so CD's charge cannot be refused and the writes need no subtransaction.
            IF spike_041.step_log_reserve(p_tenant, v_charge) THEN
                RETURN spike_041.step_log_carry_write(p_tenant, p_namespace, p_run, p_step, p_node, p_attempt,
                    p_generation, p_messages, v_retention, v_records, v_bytes, v_kept, v_kept_bytes);
            END IF;
            -- The slow path: a refill is needed; its refusal or a held usage row is answered here, in the same call. The
            -- outcome names the path and the cause (T041-35's split): `written:refill`, `mark_only:quota` (the limit),
            -- `mark_only:held` (the room is in leases open transactions hold), `mark_only:busy` (the usage row was held).
            BEGIN
                v_outcome := spike_041.step_log_carry_write(p_tenant, p_namespace, p_run, p_step, p_node, p_attempt,
                    p_generation, p_messages, v_retention, v_records, v_bytes, v_kept, v_kept_bytes);
                RETURN CASE WHEN v_outcome = 'written' THEN 'written:refill' ELSE v_outcome END;
            EXCEPTION WHEN program_limit_exceeded THEN
                -- Only the quota's own refusals (D041-117): a refill that outwaited its bound is not caught, it fails the
                -- report, which its sender re-runs.
                GET STACKED DIAGNOSTICS v_state = RETURNED_SQLSTATE, v_constraint = CONSTRAINT_NAME;
                IF v_constraint NOT IN ('tenant_step_log_quota_exceeded', 'tenant_step_log_quota_held') THEN
                    RAISE;
                END IF;
                v_cause := CASE WHEN v_constraint = 'tenant_step_log_quota_held' THEN 'held' ELSE 'quota' END;
                -- Mark-only: the final mark and the records counted dropped (tenant_quota), nothing charged.
                INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node,
                        instance_path_nodes, instance_path_indices, attempt, generation, retention_days,
                        last_sequence, dropped_by_server, ceiling_reason, last_at, expires_at, final_at)
                SELECT p_tenant, gen_random_uuid(), p_namespace, p_run, p_step, p_node, '{}'::text[], '{}'::integer[],
                       p_attempt, p_generation, v_retention, v_kept, v_kept, 'tenant_quota', clock_timestamp(),
                       clock_timestamp() + make_interval(days => v_retention), clock_timestamp()
                 WHERE NOT EXISTS (SELECT 1 FROM spike_041.step_log_streams WHERE tenant_id = p_tenant
                                      AND run_id = p_run AND step_id = p_step AND attempt = p_attempt
                                      AND generation = p_generation AND source = 'worker')
                ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO NOTHING
                RETURNING stream_id INTO v_stream;
                IF v_stream IS NOT NULL THEN RETURN 'mark_only:' || v_cause; END IF;
                -- An existing stream (a long step's): the rolled-back block released its row; take it again, if free.
                UPDATE spike_041.step_log_streams
                   SET last_sequence = GREATEST(last_sequence, last_sequence + v_kept),
                       dropped_by_server = dropped_by_server + v_kept, ceiling_reason = 'tenant_quota',
                       last_at = clock_timestamp(), final_at = clock_timestamp()
                 WHERE (tenant_id, stream_id) = (
                       SELECT tenant_id, stream_id FROM spike_041.step_log_streams
                        WHERE tenant_id = p_tenant AND run_id = p_run AND step_id = p_step AND attempt = p_attempt
                          AND generation = p_generation AND source = 'worker' AND final_at IS NULL
                          AND removed_at IS NULL AND expires_at > clock_timestamp()
                          FOR UPDATE SKIP LOCKED)
                RETURNING stream_id INTO v_stream;
                RETURN CASE WHEN v_stream IS NOT NULL THEN 'mark_only:' ELSE 'contended:' END || v_cause;
            END;
        END $$",
    ];
    for statement in statements {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("spike schema (third run, storage): {error}"));
    }
}

#[cfg(all(test, feature = "perf-spike"))]
#[path = "step_log_leases_tests.rs"]
mod leases_tests;

/// The admission of one replica: the Logs class cap and the tenant's tickets. No call waits for either.
#[derive(Clone)]
pub struct Admission {
    class: Arc<Semaphore>,
    tickets: Arc<Semaphore>,
}

impl Admission {
    pub fn new(cap: usize, tickets: usize) -> Self {
        Self {
            class: Arc::new(Semaphore::new(cap.max(1))),
            tickets: Arc::new(Semaphore::new(tickets.max(1))),
        }
    }
}

/// What a batch was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    FinalOnly,
    FinalRecorded,
    Stop(&'static str),
    /// No ticket or class slot was free: answered at once, no statement ran.
    RetryTicket,
    /// A lock timeout rolled the transaction back.
    RetryLock,
    /// The finalizer raised the tenant quota at commit.
    RetryQuota,
}

impl Verdict {
    pub const fn is_retry(self) -> bool {
        matches!(self, Self::RetryTicket | Self::RetryLock | Self::RetryQuota)
    }
}

/// One record of a batch: the worker's sequence and the message.
pub struct Record {
    pub sequence: i64,
    pub message: String,
}

/// Counters of every answer, shared by the workers of a configuration.
#[derive(Default)]
pub struct Tally {
    pub stored: AtomicU64,
    pub final_only: AtomicU64,
    pub final_recorded: AtomicU64,
    pub stopped: AtomicU64,
    pub retry_ticket: AtomicU64,
    pub retry_ticket_final: AtomicU64,
    pub retry_lock: AtomicU64,
    pub retry_lock_final: AtomicU64,
    pub retry_quota: AtomicU64,
    pub calls: AtomicU64,
    pub final_calls: AtomicU64,
    pub flush_given_up: AtomicU64,
}

impl Tally {
    fn count(&self, verdict: Verdict, final_batch: bool) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if final_batch {
            self.final_calls.fetch_add(1, Ordering::Relaxed);
        }
        let counter = match verdict {
            Verdict::Continue => &self.stored,
            Verdict::FinalOnly => &self.final_only,
            Verdict::FinalRecorded => &self.final_recorded,
            Verdict::Stop(_) => &self.stopped,
            Verdict::RetryTicket if final_batch => &self.retry_ticket_final,
            Verdict::RetryTicket => &self.retry_ticket,
            Verdict::RetryLock if final_batch => &self.retry_lock_final,
            Verdict::RetryLock => &self.retry_lock,
            Verdict::RetryQuota => &self.retry_quota,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

/// The prototype ingest of one replica.
#[derive(Clone)]
pub struct Ingest {
    pub pool: PgPool,
    pub admission: Admission,
    pub tally: Arc<Tally>,
    /// Chunk rows (O041-021): one row per batch holding its records, instead of one row per line.
    pub chunk: bool,
    /// T041-35: T3 also takes the attempt's stream lock and T9 raises `last_stored_xid` (design section 2a).
    pub stream_lock: bool,
}

/// What the ingest learnt of the stream, for the plan of T5 and T6.
struct StreamRow {
    stream_id: i64,
    records: i64,
    bytes: i64,
    last_sequence: i64,
    final_recorded: bool,
    removed: bool,
}

impl Ingest {
    /// One batch through admission and the transaction. Returns the verdict and, when a transaction
    /// ran to its end (committed or rolled back on a verdict), its service time.
    pub async fn append(
        &self,
        session: &Session,
        task: &ClaimedTask,
        records: &[Record],
        final_batch: bool,
        highest: i64,
    ) -> (Verdict, Option<Duration>) {
        // A batch is a call of its own: yield before taking a slot, so the slot the previous batch of this
        // worker released can go to another worker, as it would after a network round trip.
        tokio::task::yield_now().await;
        let Ok(_class) = self.admission.class.try_acquire() else {
            self.tally.count(Verdict::RetryTicket, final_batch);
            return (Verdict::RetryTicket, None);
        };
        let Ok(_ticket) = self.admission.tickets.try_acquire() else {
            self.tally.count(Verdict::RetryTicket, final_batch);
            return (Verdict::RetryTicket, None);
        };
        let started = Instant::now();
        let verdict = match self
            .transaction(session, task, records, final_batch, highest)
            .await
        {
            Ok(verdict) => verdict,
            Err(error) => match error.as_database_error() {
                Some(db) if db.code().as_deref() == Some("55P03") => Verdict::RetryLock,
                Some(db) if db.constraint() == Some("tenant_log_quota_exceeded") => {
                    Verdict::RetryQuota
                }
                _ => panic!("spike ingest failed: {error}"),
            },
        };
        let elapsed = started.elapsed();
        self.tally.count(verdict, final_batch);
        // Every transaction that ran counts in S, the ones a lock timeout or the quota rolled back included.
        (
            verdict,
            (verdict != Verdict::RetryTicket).then_some(elapsed),
        )
    }

    #[allow(clippy::too_many_lines)]
    async fn transaction(
        &self,
        session: &Session,
        task: &ClaimedTask,
        records: &[Record],
        final_batch: bool,
        highest: i64,
    ) -> Result<Verdict, sqlx::Error> {
        let tenant = session.principal.tenant_id().as_uuid();
        let subject = session.principal.subject().as_str().to_owned();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "SELECT set_config('lock_timeout', '200ms', true), set_config('application_name', $1, true)",
        )
        .bind(INGEST_APPLICATION)
        .execute(&mut *tx)
        .await?;
        // T1: the transaction's id before the lease is read.
        let _xid: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
            .fetch_one(&mut *tx)
            .await?;
        // T2: the lease, without a lock, and the stop rule of the report.
        let (nodes, indices): (Vec<String>, Vec<i32>) = task
            .instance
            .path()
            .segments()
            .iter()
            .map(|segment| {
                (
                    segment.scope_node().as_str().to_owned(),
                    i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
                )
            })
            .unzip();
        let lease = sqlx::query(
            "SELECT s.namespace_id, s.activity, s.cancel_requested, r.finished_at IS NOT NULL AS finished, \
                    r.stop_reason IS NOT NULL AS stopping \
               FROM steps s JOIN runs r ON r.tenant_id = s.tenant_id AND r.id = s.run_id \
              WHERE s.tenant_id = $1 AND s.step_id = $2 AND s.run_id = $3 AND s.node = $4 \
                AND s.instance_path_nodes = $5 AND s.instance_path_indices = $6 \
                AND s.status IN ('LEASED', 'RUNNING') AND s.attempt = $7 AND s.generation = $8 \
                AND s.lease_token_hash = $9 AND s.lease_expires_at > clock_timestamp()",
        )
        .bind(tenant)
        .bind(task.step_id)
        .bind(task.run_id.as_uuid())
        .bind(task.instance.node().as_str())
        .bind(&nodes)
        .bind(&indices)
        .bind(i32::try_from(task.fence.attempt).unwrap_or(i32::MAX))
        .bind(i64::try_from(task.fence.generation).unwrap_or(i64::MAX))
        .bind(Token::hash_of(task.lease_token.expose()))
        .fetch_optional(&mut *tx)
        .await?;
        let Some(lease) = lease else {
            tx.rollback().await?;
            return Ok(Verdict::Stop("lease"));
        };
        let namespace: Uuid = lease.try_get("namespace_id")?;
        let activity: Option<String> = lease.try_get("activity")?;
        let cancel: bool = lease.try_get("cancel_requested")?;
        let finished: bool = lease.try_get("finished")?;
        let stopping: bool = lease.try_get("stopping")?;
        if cancel || finished || (stopping && !task.instance.node().is_derived()) {
            tx.rollback().await?;
            return Ok(Verdict::Stop("lease"));
        }
        // T3: the session, then the six policy fences in order, then the three values.
        let session_ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM worker_sessions WHERE tenant_id = $1 AND id = $2 \
               AND namespace_id = $3 AND subject = $4 AND token_hash = $5 \
               AND expires_at > clock_timestamp() AND accepted_protocol_minor >= 0)",
        )
        .bind(tenant)
        .bind(session.worker_id)
        .bind(namespace)
        .bind(&subject)
        .bind(Token::hash_of(&session.token))
        .fetch_one(&mut *tx)
        .await?;
        let keys: Vec<String> = RESOURCES
            .iter()
            .flat_map(|resource| {
                [
                    format!("quota-policy:{resource}:{tenant}"),
                    format!("quota-policy:{resource}:{tenant}:{namespace}"),
                ]
            })
            .collect();
        if self.stream_lock {
            sqlx::query(
                "SELECT (SELECT count(pg_advisory_xact_lock_shared(hashtextextended(k, 0))) \
                           FROM (SELECT k FROM unnest($1::text[]) WITH ORDINALITY AS f(k, n) ORDER BY n) ordered), \
                        pg_advisory_xact_lock(hashtextextended($2, 0))",
            )
            .bind(&keys)
            .bind(stream_lock_key(
                tenant,
                task.run_id.as_uuid(),
                task.step_id,
                i32::try_from(task.fence.attempt).unwrap_or(i32::MAX),
                i64::try_from(task.fence.generation).unwrap_or(i64::MAX),
            ))
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "SELECT count(pg_advisory_xact_lock_shared(hashtextextended(k, 0))) \
                   FROM (SELECT k FROM unnest($1::text[]) WITH ORDINALITY AS f(k, n) ORDER BY n) ordered",
            )
            .bind(&keys)
            .execute(&mut *tx)
            .await?;
        }
        let stored: Vec<(String, Option<Uuid>, i64)> = sqlx::query_as(
            "SELECT resource, namespace_id, max_value FROM quota_limits \
              WHERE tenant_id = $1 AND resource = ANY($2) AND (namespace_id IS NULL OR namespace_id = $3) \
             UNION ALL SELECT resource, NULL, max_value FROM quota_ceilings \
              WHERE tenant_id = $1 AND resource = ANY($2)",
        )
        .bind(tenant)
        .bind(RESOURCES.map(str::to_owned).to_vec())
        .bind(namespace)
        .fetch_all(&mut *tx)
        .await?;
        let (attempt_records, attempt_bytes) = (
            value_of(&stored, RESOURCES[0], ATTEMPT_RECORDS),
            value_of(&stored, RESOURCES[1], ATTEMPT_BYTES),
        );
        let retention = i32::try_from(value_of(&stored, RESOURCES[2], i64::from(RETENTION_DAYS)))
            .unwrap_or(RETENTION_DAYS);
        // T4: the stream row, created or locked to commit.
        let row = sqlx::query(
            "INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
                 instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, clock_timestamp(), \
                     clock_timestamp() + make_interval(days => $11)) \
             ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) \
             DO UPDATE SET tenant_id = spike_041.step_log_streams.tenant_id \
             RETURNING stream_id, records, bytes, last_sequence, final_at IS NOT NULL AS final_recorded, \
                       (removed_at IS NOT NULL OR expires_at <= clock_timestamp()) AS removed",
        )
        .bind(tenant)
        .bind(Uuid::new_v4())
        .bind(namespace)
        .bind(task.run_id.as_uuid())
        .bind(task.step_id)
        .bind(task.instance.node().as_str())
        .bind(&nodes)
        .bind(&indices)
        .bind(i32::try_from(task.fence.attempt).unwrap_or(i32::MAX))
        .bind(i64::try_from(task.fence.generation).unwrap_or(i64::MAX))
        .bind(retention)
        .fetch_one(&mut *tx)
        .await?;
        let stream = StreamRow {
            stream_id: row.try_get("stream_id")?,
            records: i64::from(row.try_get::<i32, _>("records")?),
            bytes: row.try_get("bytes")?,
            last_sequence: row.try_get("last_sequence")?,
            final_recorded: row.try_get("final_recorded")?,
            removed: row.try_get("removed")?,
        };
        let noted = if !session_ok {
            Some(Verdict::Stop("session"))
        } else if stream.removed {
            Some(Verdict::Stop("removed"))
        } else if stream.final_recorded {
            Some(Verdict::FinalRecorded)
        } else {
            None
        };
        // T5 and T6: duplicates dropped, gaps counted, the attempt allowance from counters, then the
        // tenant's from an unlocked read.
        let fresh: Vec<&Record> = records
            .iter()
            .filter(|record| record.sequence > stream.last_sequence)
            .collect();
        let mut kept = Vec::new();
        let (mut count, mut bytes, mut over) = (stream.records, stream.bytes, false);
        for record in &fresh {
            let size = i64::try_from(record.message.len()).unwrap_or(i64::MAX);
            if over || count + 1 > attempt_records || bytes + size > attempt_bytes {
                over = true;
                continue;
            }
            count += 1;
            bytes += size;
            kept.push(*record);
        }
        let mark = fresh
            .iter()
            .map(|record| record.sequence)
            .max()
            .unwrap_or(0)
            .max(if final_batch { highest } else { 0 })
            .max(stream.last_sequence);
        let sent = i64::try_from(fresh.len()).unwrap_or(i64::MAX);
        let gap = (mark - stream.last_sequence - sent).max(0);
        let _usage: Option<i64> =
            sqlx::query_scalar("SELECT log_bytes FROM tenant_retained_usage WHERE tenant_id = $1")
                .bind(tenant)
                .fetch_optional(&mut *tx)
                .await?;
        // T7: the binding's patterns and the membership, without a lock.
        let patterns: Vec<String> = sqlx::query_scalar(
            "SELECT pattern FROM worker_bindings WHERE tenant_id = $1 AND namespace_id = $2 AND subject = $3",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(&subject)
        .fetch_all(&mut *tx)
        .await?;
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2",
        )
        .bind(tenant)
        .bind(&subject)
        .fetch_optional(&mut *tx)
        .await?;
        let covered = activity.is_some() && !patterns.is_empty();
        if !covered || role.is_none() {
            tx.rollback().await?;
            return Ok(Verdict::Stop("authority"));
        }
        if let Some(noted) = noted {
            tx.rollback().await?;
            return Ok(noted);
        }
        // T8: the kept records, at most once each.
        let joined;
        let (sequences, messages): (Vec<i64>, Vec<&str>) = if self.chunk && !kept.is_empty() {
            joined = kept
                .iter()
                .map(|record| record.message.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (vec![kept[0].sequence], vec![joined.as_str()])
        } else {
            (
                kept.iter().map(|record| record.sequence).collect(),
                kept.iter().map(|record| record.message.as_str()).collect(),
            )
        };
        let inserted: Vec<i32> = sqlx::query_scalar(
            "INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message) \
             SELECT $1, $2, s, $3, clock_timestamp(), 'info', 'line', m FROM unnest($4::bigint[], $5::text[]) AS u(s, m) \
             ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING RETURNING octet_length(message)",
        )
        .bind(tenant)
        .bind(stream.stream_id)
        .bind(task.run_id.as_uuid())
        .bind(&sequences)
        .bind(&messages)
        .fetch_all(&mut *tx)
        .await?;
        let stored_bytes: i64 = inserted.iter().map(|n| i64::from(*n)).sum();
        let stored_count = if self.chunk {
            i32::try_from(kept.len()).unwrap_or(i32::MAX)
        } else {
            i32::try_from(inserted.len()).unwrap_or(i32::MAX)
        };
        let dropped_by_server = sent - i64::try_from(kept.len()).unwrap_or(i64::MAX);
        // T9: the counters, from what the insert returned.
        sqlx::query(
            "UPDATE spike_041.step_log_streams SET last_sequence = GREATEST(last_sequence, $3), \
                    records = records + $4, bytes = bytes + $5, \
                    dropped_by_worker = LEAST(dropped_by_worker + $6, 9007199254740992), \
                    dropped_by_server = dropped_by_server + $7, \
                    ceiling_sequence = CASE WHEN $8 AND ceiling_sequence IS NULL THEN $3 ELSE ceiling_sequence END, \
                    ceiling_reason = CASE WHEN $8 AND ceiling_reason IS NULL THEN 'attempt_bytes' ELSE ceiling_reason END, \
                    first_at = coalesce(first_at, clock_timestamp()), last_at = clock_timestamp(), \
                    expires_at = clock_timestamp() + make_interval(days => retention_days::integer), \
                    final_at = CASE WHEN $9 THEN coalesce(final_at, clock_timestamp()) ELSE final_at END, \
                    last_stored_xid = CASE WHEN $10 AND $4 > 0 \
                        THEN GREATEST(coalesce(last_stored_xid, '0'::xid8), pg_current_xact_id()) ELSE last_stored_xid END \
              WHERE tenant_id = $1 AND stream_id = $2",
        )
        .bind(tenant)
        .bind(stream.stream_id)
        .bind(mark)
        .bind(stored_count)
        .bind(stored_bytes)
        .bind(gap)
        .bind(dropped_by_server)
        .bind(over)
        .bind(final_batch)
        .bind(self.stream_lock)
        .execute(&mut *tx)
        .await?;
        // T10: the commit runs the finalizer (the log fence and the tenant usage row).
        tx.commit().await?;
        let _ = MARKER_SEQUENCE;
        Ok(if over {
            Verdict::FinalOnly
        } else {
            Verdict::Continue
        })
    }
}

fn value_of(stored: &[(String, Option<Uuid>, i64)], resource: &str, default: i64) -> i64 {
    let scoped = stored
        .iter()
        .find(|(name, namespace, _)| name == resource && namespace.is_some());
    let tenant = stored
        .iter()
        .find(|(name, namespace, _)| name == resource && namespace.is_none());
    scoped.or(tenant).map_or(default, |(_, _, value)| *value)
}

/// `count` records of `bytes` bytes each, numbered from `from`.
pub fn records(from: i64, count: usize, bytes: usize) -> Vec<Record> {
    (0..count)
        .map(|n| {
            let sequence = from + i64::try_from(n).unwrap_or(0);
            let head = format!("line {sequence} of the spike ");
            let mut message = head.clone();
            while message.len() < bytes {
                message.push('x');
            }
            message.truncate(bytes.max(head.len()));
            Record { sequence, message }
        })
        .collect()
}

/// Sends a batch as an SDK does inside its final flush: a refused batch is sent again after 200 ms
/// until `deadline`; returns the last verdict and the service times of the transactions that ran.
#[allow(clippy::too_many_arguments)] // the batch and its fence, as the RPC carries them
pub async fn send_until(
    ingest: &Ingest,
    session: &Session,
    task: &ClaimedTask,
    records: &[Record],
    final_batch: bool,
    highest: i64,
    deadline: Instant,
    services: &mut Vec<Duration>,
) -> Option<Verdict> {
    loop {
        let (verdict, service) = ingest
            .append(session, task, records, final_batch, highest)
            .await;
        if let Some(service) = service {
            services.push(service);
        }
        if !verdict.is_retry() {
            return Some(verdict);
        }
        if Instant::now() + Duration::from_millis(200) >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

// ---------------------------------------------------------------------------------- the carry (T041-35)

/// The application name a carrying report's backend takes in CA, so the sampler can tell it apart.
pub const CARRY_APPLICATION: &str = "spike041carry";

/// The attempt's stream lock (design 041 section 2a, L1b): an advisory key of tenant, run, step, attempt and generation.
fn stream_lock_key(tenant: Uuid, run: Uuid, step: Uuid, attempt: i32, generation: i64) -> String {
    format!("step-log-stream:{tenant}:{run}:{step}:{attempt}:{generation}")
}

/// What CB reads: the three resources' rows as arrays (names, namespaces, values) and the tenant's usage.
#[cfg(feature = "perf-spike")]
type PolicyRead = (
    Option<Vec<String>>,
    Option<Vec<Option<Uuid>>>,
    Option<Vec<i64>>,
    Option<i64>,
);

/// The carry hook's counters at the end of a row (T041-35). `calls` = `written` + the three mark-only causes +
/// `contended` + `neither`; `written_refill` is part of `written`.
#[derive(Clone, Debug, Default)]
#[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
pub struct CarryCounts {
    pub calls: u64,
    pub written: u64,
    pub written_refill: u64,
    pub mark_only_quota: u64,
    pub mark_only_held: u64,
    pub mark_only_busy: u64,
    pub contended: u64,
    pub neither: u64,
    pub times: Vec<Duration>,
    /// Sorted times of the fast path's writes, the refill's writes and the mark-only answers.
    pub times_by_path: [Vec<Duration>; 3],
}

/// The carry of design 041 section 2a, statements CA to CE of data-model.md §8a, run by the report hook inside the
/// report transaction of a root attempt, before its commit. It plans inline the two cases the workloads meet: a new
/// stream, and a stream holding one batch, every record kept.
#[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
pub struct CarryHook {
    /// Records per carry.
    pub records: usize,
    pub bytes: usize,
    pub calls: AtomicU64,
    pub written: AtomicU64,
    pub contended: AtomicU64,
    pub neither: AtomicU64,
    /// The third shape's outcome split (T041-35, the coordinator's request of 2026-11-20): of `written`, the carries
    /// that took the slow path (a refill); and the mark-only answers by cause, which `neither` no longer counts.
    pub written_refill: AtomicU64,
    pub mark_only_quota: AtomicU64,
    pub mark_only_held: AtomicU64,
    pub mark_only_busy: AtomicU64,
    pub times: std::sync::Mutex<Vec<Duration>>,
    /// The carry's time by path: written on the fast path, written by a refill, mark-only (any cause).
    pub times_by_path: std::sync::Mutex<[Vec<Duration>; 3]>,
    /// T041-35's second run (`SAGAT_PERF_STEP_LOG_CARRY_RUN=2`): the carry as one call of `spike_041.step_log_carry`.
    pub one_call: bool,
    /// The hot-row diagnosis (`step_log_diag`): a hook that carries nothing and only times the report's `COMMIT`.
    pub probe: bool,
    /// The hot-row diagnosis (`SAGAT_PERF_STEP_LOG_DIAG=1`): the store times each report's `COMMIT` for this hook.
    pub timed: bool,
    /// Each timed report: its transaction id and when its `COMMIT` was sent and returned (Unix microseconds).
    pub committed: std::sync::Mutex<Vec<(i64, i64, i64)>>,
}

impl CarryHook {
    pub fn new(records: usize, bytes: usize) -> Self {
        Self {
            one_call: matches!(
                std::env::var("SAGAT_PERF_STEP_LOG_CARRY_RUN").as_deref(),
                Ok("2" | "3")
            ),
            records,
            bytes,
            calls: AtomicU64::new(0),
            written: AtomicU64::new(0),
            contended: AtomicU64::new(0),
            neither: AtomicU64::new(0),
            written_refill: AtomicU64::new(0),
            mark_only_quota: AtomicU64::new(0),
            mark_only_held: AtomicU64::new(0),
            mark_only_busy: AtomicU64::new(0),
            times: std::sync::Mutex::new(Vec::new()),
            times_by_path: std::sync::Mutex::new([Vec::new(), Vec::new(), Vec::new()]),
            probe: false,
            timed: crate::step_log_diag::enabled(),
            committed: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// The hot-row diagnosis's hook for a capture-off row: it carries nothing and only times the report's `COMMIT`.
    pub fn probe() -> Self {
        Self {
            probe: true,
            timed: true,
            ..Self::new(0, 0)
        }
    }

    #[cfg(feature = "perf-spike")]
    #[allow(clippy::too_many_lines)]
    async fn carry(
        &self,
        conn: &mut sqlx::PgConnection,
        a: &sagat_infra_postgres::report_hook::ReportedAttempt,
    ) -> Result<(), sqlx::Error> {
        if self.probe {
            return Ok(());
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        let started = Instant::now();
        if self.one_call {
            return self.carry_in_one_call(conn, a, started).await;
        }
        let keys: Vec<String> = RESOURCES
            .iter()
            .flat_map(|resource| {
                [
                    format!("quota-policy:{resource}:{}", a.tenant),
                    format!("quota-policy:{resource}:{}:{}", a.tenant, a.namespace),
                ]
            })
            .collect();
        // CA: the fences and the stream lock, tried, never waited for.
        let (_, fences, stream_lock): (String, Option<bool>, bool) = sqlx::query_as(
            "SELECT set_config('application_name', $3, true), \
                    (SELECT bool_and(pg_try_advisory_xact_lock_shared(hashtextextended(k, 0))) \
                       FROM (SELECT k FROM unnest($1::text[]) WITH ORDINALITY AS f(k, n) ORDER BY n) ordered), \
                    pg_try_advisory_xact_lock(hashtextextended($2, 0))",
        )
        .bind(&keys)
        .bind(stream_lock_key(a.tenant, a.run, a.step, a.attempt, a.generation))
        .bind(CARRY_APPLICATION)
        .fetch_one(&mut *conn)
        .await?;
        if fences != Some(true) || !stream_lock {
            self.contended.fetch_add(1, Ordering::Relaxed);
            self.record(started);
            return Ok(());
        }
        // CB: the three resources' rows and the tenant's allowance, read after the locks are held.
        let (names, scopes, values, _usage): PolicyRead = sqlx::query_as(
            "SELECT array_agg(resource), array_agg(namespace_id), array_agg(max_value), \
                    (SELECT log_bytes FROM tenant_retained_usage WHERE tenant_id = $1) \
               FROM (SELECT resource, namespace_id, max_value FROM quota_limits \
                      WHERE tenant_id = $1 AND resource = ANY($2) AND (namespace_id IS NULL OR namespace_id = $3) \
                     UNION ALL SELECT resource, NULL::uuid, max_value FROM quota_ceilings \
                      WHERE tenant_id = $1 AND resource = ANY($2)) q",
        )
        .bind(a.tenant)
        .bind(RESOURCES.map(str::to_owned).to_vec())
        .bind(a.namespace)
        .fetch_one(&mut *conn)
        .await?;
        let stored: Vec<(String, Option<Uuid>, i64)> = names
            .unwrap_or_default()
            .into_iter()
            .zip(scopes.unwrap_or_default())
            .zip(values.unwrap_or_default())
            .map(|((n, s), v)| (n, s, v))
            .collect();
        if value_of(&stored, RESOURCES[0], ATTEMPT_RECORDS) == 0 {
            self.neither.fetch_add(1, Ordering::Relaxed);
            self.record(started);
            return Ok(());
        }
        let retention = i32::try_from(value_of(&stored, RESOURCES[2], i64::from(RETENTION_DAYS)))
            .unwrap_or(RETENTION_DAYS);
        // CC: a new stream; the NOT EXISTS keeps the insert from reaching a unique check that could wait.
        let created: Option<i64> = sqlx::query_scalar(
            "INSERT INTO spike_041.step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
                 instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
             SELECT $1, $2, $3, $4, $5, $6, '{}'::text[], '{}'::integer[], $7, $8, $9, clock_timestamp(), \
                    clock_timestamp() + make_interval(days => $9) \
              WHERE NOT EXISTS (SELECT 1 FROM spike_041.step_log_streams WHERE tenant_id = $1 AND run_id = $4 \
                                   AND step_id = $5 AND attempt = $7 AND generation = $8 AND source = 'worker') \
             ON CONFLICT (tenant_id, run_id, step_id, attempt, generation, source) DO NOTHING \
             RETURNING stream_id",
        )
        .bind(a.tenant)
        .bind(Uuid::new_v4())
        .bind(a.namespace)
        .bind(a.run)
        .bind(a.step)
        .bind(&a.node)
        .bind(a.attempt)
        .bind(a.generation)
        .bind(retention)
        .fetch_optional(&mut *conn)
        .await?;
        let (stream_id, last_sequence) = if let Some(id) = created {
            (id, 0)
        } else {
            // CC′: the existing stream, locked only if free.
            let row = sqlx::query(
                "SELECT stream_id, last_sequence, final_at IS NOT NULL AS final_recorded, \
                        (removed_at IS NOT NULL OR expires_at <= clock_timestamp()) AS removed, \
                        (last_stored_xid IS NULL OR last_stored_xid < pg_current_xact_id()) AS in_order \
                   FROM spike_041.step_log_streams \
                  WHERE tenant_id = $1 AND run_id = $2 AND step_id = $3 AND attempt = $4 AND generation = $5 \
                    AND source = 'worker' \
                    FOR UPDATE SKIP LOCKED",
            )
            .bind(a.tenant)
            .bind(a.run)
            .bind(a.step)
            .bind(a.attempt)
            .bind(a.generation)
            .fetch_optional(&mut *conn)
            .await?;
            let Some(row) = row else {
                self.contended.fetch_add(1, Ordering::Relaxed);
                self.record(started);
                return Ok(());
            };
            let removed: bool = row.try_get("removed")?;
            let final_recorded: bool = row.try_get("final_recorded")?;
            let in_order: bool = row.try_get("in_order")?;
            if removed || final_recorded || !in_order {
                self.neither.fetch_add(1, Ordering::Relaxed);
                self.record(started);
                return Ok(());
            }
            (
                row.try_get("stream_id")?,
                row.try_get::<i64, _>("last_sequence")?,
            )
        };
        // The plan, inline: every record kept, numbered after the stream's last sequence.
        let batch = records(last_sequence + 1, self.records, self.bytes);
        let highest = last_sequence + i64::try_from(self.records).unwrap_or(0);
        // CD: the kept lines.
        let inserted: Vec<i32> = sqlx::query_scalar(
            "INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message) \
             SELECT $1, $2, s, $3, clock_timestamp(), 'info', 'line', m FROM unnest($4::bigint[], $5::text[]) AS u(s, m) \
             ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING RETURNING octet_length(message)",
        )
        .bind(a.tenant)
        .bind(stream_id)
        .bind(a.run)
        .bind(batch.iter().map(|r| r.sequence).collect::<Vec<_>>())
        .bind(batch.iter().map(|r| r.message.as_str()).collect::<Vec<_>>())
        .fetch_all(&mut *conn)
        .await?;
        let stored_bytes: i64 = inserted.iter().map(|n| i64::from(*n)).sum();
        // CE: the counters, the final mark and the order guard.
        sqlx::query(
            "UPDATE spike_041.step_log_streams SET last_sequence = GREATEST(last_sequence, $3), \
                    records = records + $4, bytes = bytes + $5, \
                    first_at = coalesce(first_at, clock_timestamp()), last_at = clock_timestamp(), \
                    expires_at = clock_timestamp() + make_interval(days => retention_days::integer), \
                    final_at = clock_timestamp(), \
                    last_stored_xid = CASE WHEN $4 > 0 \
                        THEN GREATEST(coalesce(last_stored_xid, '0'::xid8), pg_current_xact_id()) ELSE last_stored_xid END \
              WHERE tenant_id = $1 AND stream_id = $2",
        )
        .bind(a.tenant)
        .bind(stream_id)
        .bind(highest)
        .bind(i32::try_from(inserted.len()).unwrap_or(i32::MAX))
        .bind(stored_bytes)
        .execute(&mut *conn)
        .await?;
        self.written.fetch_add(1, Ordering::Relaxed);
        self.record(started);
        Ok(())
    }

    /// The second run's carry: one round trip, the function of `install_per_statement`.
    #[cfg(feature = "perf-spike")]
    async fn carry_in_one_call(
        &self,
        conn: &mut sqlx::PgConnection,
        a: &sagat_infra_postgres::report_hook::ReportedAttempt,
        started: Instant,
    ) -> Result<(), sqlx::Error> {
        let keys: Vec<String> = RESOURCES
            .iter()
            .flat_map(|resource| {
                [
                    format!("quota-policy:{resource}:{}", a.tenant),
                    format!("quota-policy:{resource}:{}:{}", a.tenant, a.namespace),
                ]
            })
            .collect();
        let messages: Vec<String> = records(1, self.records, self.bytes)
            .into_iter()
            .map(|record| record.message)
            .collect();
        let outcome: String = sqlx::query_scalar(
            "SELECT spike_041.step_log_carry($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(a.tenant)
        .bind(a.namespace)
        .bind(a.run)
        .bind(a.step)
        .bind(&a.node)
        .bind(a.attempt)
        .bind(a.generation)
        .bind(&keys)
        .bind(stream_lock_key(
            a.tenant,
            a.run,
            a.step,
            a.attempt,
            a.generation,
        ))
        .bind(RESOURCES.map(str::to_owned).to_vec())
        .bind(&messages)
        .bind(CARRY_APPLICATION)
        .fetch_one(&mut *conn)
        .await?;
        self.count(&outcome);
        let path = match outcome.as_str() {
            "written" => Some(0),
            "written:refill" => Some(1),
            o if o.starts_with("mark_only:") => Some(2),
            _ => None,
        };
        if let (Some(path), Ok(mut by_path)) = (path, self.times_by_path.lock()) {
            by_path[path].push(started.elapsed());
        }
        self.record(started);
        Ok(())
    }

    /// Counts one answer of `spike_041.step_log_carry`: `written`, `written:refill`, `mark_only:<cause>`,
    /// `contended[:<cause>]`, anything else `neither`.
    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    pub fn count(&self, outcome: &str) {
        let counter = match outcome {
            "written" => &self.written,
            "written:refill" => {
                self.written_refill.fetch_add(1, Ordering::Relaxed);
                &self.written
            }
            "mark_only:quota" => &self.mark_only_quota,
            "mark_only:held" => &self.mark_only_held,
            "mark_only:busy" => &self.mark_only_busy,
            o if o == "contended" || o.starts_with("contended:") => &self.contended,
            _ => &self.neither,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// The counters as the reports print them.
    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    pub fn counts(&self) -> CarryCounts {
        let mut times = self.times.lock().map(|t| t.clone()).unwrap_or_default();
        times.sort_unstable();
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);
        CarryCounts {
            calls: load(&self.calls),
            written: load(&self.written),
            written_refill: load(&self.written_refill),
            mark_only_quota: load(&self.mark_only_quota),
            mark_only_held: load(&self.mark_only_held),
            mark_only_busy: load(&self.mark_only_busy),
            contended: load(&self.contended),
            neither: load(&self.neither),
            times,
            times_by_path: self
                .times_by_path
                .lock()
                .map(|t| {
                    t.clone().map(|mut v| {
                        v.sort_unstable();
                        v
                    })
                })
                .unwrap_or_default(),
        }
    }

    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    fn record(&self, started: Instant) {
        if let Ok(mut times) = self.times.lock() {
            times.push(started.elapsed());
        }
    }
}

#[cfg(feature = "perf-spike")]
impl sagat_infra_postgres::report_hook::ReportHook for CarryHook {
    fn on_report_commit<'a>(
        &'a self,
        conn: &'a mut sqlx::PgConnection,
        attempt: &'a sagat_infra_postgres::report_hook::ReportedAttempt,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send + 'a>>
    {
        Box::pin(self.carry(conn, attempt))
    }

    fn times_commit(&self) -> bool {
        self.timed
    }

    fn on_committed(&self, xid: i64, sent_us: i64, returned_us: i64) {
        if let Ok(mut committed) = self.committed.lock() {
            committed.push((xid, sent_us, returned_us));
        }
    }
}

// ---------------------------------------------------------------------------------- the driver

/// The workflow a configuration runs.
#[derive(Clone, Copy)]
pub enum Shape {
    /// Runs of three chained activities, as `step_transitions_per_second`.
    Chain,
    /// Runs of one `forEach` over `items` whose body is one activity.
    Fanout { items: usize },
}

/// One configuration of the spike.
pub struct RowSpec {
    pub label: &'static str,
    pub api_replicas: usize,
    pub workers: usize,
    pub runs: usize,
    pub shape: Shape,
    /// `None`: the driver is off (the baseline); else the Logs class cap and the tenant tickets per replica.
    pub admission: Option<(usize, usize)>,
    /// Records per step and bytes per record in the first batch.
    pub lines: (usize, usize),
    /// Hold `BEGIN; SELECT pg_current_xact_id();` on another connection while the workers run.
    pub hold: bool,
    /// Send the lines in the final batch itself (one ingest per step), as an SDK does for a step
    /// shorter than its one-second flush, instead of a batch and then an empty final batch.
    pub single_final: bool,
    /// Chunk rows instead of one row per line (O041-021).
    pub chunk: bool,
    /// T041-35: the records each completion carries (the report hook is installed); with `admission`, each step
    /// first sends one non-final batch of `lines` and awaits its answer (workload (g)).
    pub carry: Option<usize>,
    /// T041-35 (f): the completion's output is `{}`, which stores nothing new.
    pub empty_output: bool,
}

pub struct RowResult {
    pub transitions: usize,
    pub per_second: f64,
    pub completes: Vec<Duration>,
    pub flushes: Vec<Duration>,
    pub services: Vec<Duration>,
    pub tally: Arc<Tally>,
    /// 10 ms samples of a non-ingest backend waiting on a lock in `COMMIT`, and of those whose blocker
    /// was an ingest backend.
    pub commit_waits: (u64, u64),
    pub deltas_bytes: (i64, i64),
    pub held_for: Option<Duration>,
    /// T041-35: the carry hook's counters and statement times, sorted. Read only by the carry spike, which needs
    /// `perf-spike`.
    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    pub carry: Option<CarryCounts>,
    /// 10 ms samples of a commit wait whose blocker was a carrying report.
    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    pub carry_waits: u64,
    /// The hot-row diagnosis of the row (`SAGAT_PERF_STEP_LOG_DIAG=1`), as `step_log_diag::Probe::finish` reports it.
    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    pub diag: Option<serde_json::Value>,
}

/// A 1 KiB output that differs per task, so every completion stores a payload not yet stored.
fn output(task: &ClaimedTask) -> Vec<u8> {
    let tag = task.task_id();
    let pad = "o".repeat(1024usize.saturating_sub(tag.len() + 16));
    serde_json::to_vec(&serde_json::json!({ "t": tag, "pad": pad })).expect("json")
}

/// Whether each row first fills its replicas' pools (D041-110). The diagnosis turns it off for the rows that reproduce
/// rounds 4 and 5 of D041-105.
pub static PREWARM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Opens every connection of `pool` before a row is measured (D041-110). A call of the benchmark then never waits for a
/// connection to be established: a replica has at most one call in flight per worker (each worker is sequential), 16 at
/// both sizes, against a pool of `API_POOL` = 20, and `start` has at most 16 calls in flight per pool of 16, so with full
/// pools an acquire never waits and the 2 s acquire timeout of `tests/support/engine.rs` only fails a row on a real
/// defect. Setup, not measured: each connection is retried until 120 s have passed.
pub async fn prewarm(pool: &PgPool) {
    if !PREWARM.load(Ordering::Relaxed) {
        return;
    }
    let max = pool.options().get_max_connections();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut held = Vec::new();
    while held.len() < usize::try_from(max).unwrap_or(0) {
        match pool.acquire().await {
            Ok(conn) => held.push(conn),
            Err(error) => assert!(Instant::now() < deadline, "prewarm a pool: {error}"),
        }
    }
    drop(held);
    assert_eq!(
        pool.size(),
        max,
        "a prewarmed pool holds all its connections"
    );
}

/// A call of the benchmark failed: prints what tells a pool wait (about the 2 s acquire timeout of
/// `tests/support/engine.rs`, the pool full) from a server-side cancellation or deadlock (in the server's log), then panics.
fn failure(op: &str, elapsed: Duration, pool: &PgPool, error: &dyn std::fmt::Debug) -> ! {
    eprintln!(
        "spike-failure-json {}",
        serde_json::json!({
            "op": op, "error": format!("{error:?}"), "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
            "pool_size": pool.size(), "pool_idle": pool.num_idle(),
            "pool_max": pool.options().get_max_connections(),
            "acquire_timeout_ms": pool.options().get_acquire_timeout().as_secs_f64() * 1000.0,
        })
    );
    panic!("{op}: {error:?}");
}

async fn start(
    w: &crate::support::engine::World,
    wf: sagat_domain::definition::WorkflowId,
    shape: Shape,
    runs: usize,
) {
    use futures_util::StreamExt as _;
    let mut apis = Vec::new();
    for _ in 0..4 {
        let replica = w.replica_with_pool(16).await;
        prewarm(&replica.execution_pool).await;
        apis.push(Arc::new(replica));
    }
    let tag = Uuid::new_v4().simple().to_string();
    let started: Vec<()> = futures_util::stream::iter(0..runs)
        .map(|i| {
            let api = Arc::clone(&apis[i % apis.len()]);
            let admin = w.admin.clone();
            let tag = tag.clone();
            async move {
                let input = match shape {
                    Shape::Chain => serde_json::json!({ "i": i }),
                    Shape::Fanout { items } => {
                        serde_json::json!({ "items": (0..items).collect::<Vec<_>>() })
                    }
                };
                let input = serde_json::to_vec(&input).expect("json");
                let t = Instant::now();
                api.runs
                    .start(
                        &admin,
                        &sagat_application::runs::StartRun {
                            namespace: "payments",
                            workflow: wf,
                            version: None,
                            input: &input,
                            correlation_id: None,
                            deadline: None,
                            key: &format!("spike-{tag}-{i:08}"),
                        },
                        &crate::support::engine::ctx(),
                    )
                    .await
                    .unwrap_or_else(|error| {
                        failure("start", t.elapsed(), &api.execution_pool, &error)
                    });
            }
        })
        .buffer_unordered(64)
        .collect()
        .await;
    assert_eq!(started.len(), runs);
    // Close the starting pools now, so that no row adds its connections to the next (D041-110).
    for api in &apis {
        api.execution_pool.close().await;
    }
}

/// Runs one configuration: workers poll, and for each task (driver on) send a batch of the
/// configured lines and an empty final batch within a 2 s flush, then complete with a 1 KiB output.
#[allow(clippy::too_many_lines)]
pub async fn run_row(
    w: &Arc<crate::support::engine::World>,
    wf: sagat_domain::definition::WorkflowId,
    spec: &RowSpec,
    activities: &[&str],
) -> RowResult {
    let fanout = matches!(spec.shape, Shape::Fanout { .. });
    let hook = spec
        .carry
        .map(|records| Arc::new(CarryHook::new(records, spec.lines.1)))
        .or_else(|| {
            (cfg!(feature = "perf-spike") && crate::step_log_diag::enabled())
                .then(|| Arc::new(CarryHook::probe()))
        });
    let mut apis = Vec::new();
    for _ in 0..spec.api_replicas {
        let mut replica = w.replica_with_pool(crate::API_POOL).await;
        prewarm(&replica.execution_pool).await;
        #[cfg(not(feature = "perf-spike"))]
        assert!(hook.is_none(), "a carry row needs `--features perf-spike`");
        #[cfg(feature = "perf-spike")]
        if let Some(hook) = &hook {
            let hook: Arc<dyn sagat_infra_postgres::report_hook::ReportHook> = hook.clone();
            replica.store = replica.store.clone().with_report_hook(hook);
            replica.workers = sagat_application::worker::WorkerService::new(
                replica.store.clone(),
                sagat_application::clock::SystemClock,
                sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
                sagat_application::entropy::SystemEntropy,
                sagat_application::worker::WorkerSettings::default(),
            );
        }
        if fanout {
            // Steps inside a collection need the scoped claim and report path (protocol 1.5).
            replica.store = replica.store.clone().with_scoped_worker_protocol();
            replica.workers = sagat_application::worker::WorkerService::new(
                replica.store.clone(),
                sagat_application::clock::SystemClock,
                sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
                sagat_application::entropy::SystemEntropy,
                sagat_application::worker::WorkerSettings::default(),
            );
        }
        apis.push(Arc::new(replica));
    }
    let tally = Arc::new(Tally::default());
    let ingests: Vec<Option<Ingest>> = apis
        .iter()
        .map(|api| {
            spec.admission.map(|(cap, tickets)| Ingest {
                pool: api.execution_pool.clone(),
                admission: Admission::new(cap, tickets),
                tally: Arc::clone(&tally),
                chunk: spec.chunk,
                stream_lock: spec.carry.is_some(),
            })
        })
        .collect();
    start(w, wf, spec.shape, spec.runs).await;
    let mut sessions = Vec::new();
    let mut principal = w.worker.clone();
    let tag = Uuid::new_v4().simple().to_string();
    for n in 0..spec.workers {
        if n % 32 == 0 {
            principal = w
                .member(
                    &format!("spike-{tag}-{}", n / 32),
                    sagat_domain::identity::Role::Worker,
                )
                .await;
        }
        if matches!(spec.shape, Shape::Fanout { .. }) {
            // A step inside a collection is leased only to a scoped worker (protocol 1.5).
            w.bind_everything(&principal).await;
            let registered = apis[0]
                .workers
                .register(
                    &principal,
                    "payments",
                    &format!("spike-scoped-{}", Uuid::new_v4().simple()),
                    &sagat_application::worker::Capabilities {
                        activities: activities.iter().map(|a| (*a).to_owned()).collect(),
                        languages: vec!["rust".into()],
                        pools: vec![],
                        max_inflight: 100,
                        protocol_version: "1.5".into(),
                    },
                )
                .await
                .expect("register a scoped worker");
            if n == 0 {
                eprintln!(
                    "spike fan-out: scoped worker accepted at {}",
                    registered.accepted_protocol_version
                );
            }
            sessions.push(Session {
                principal: principal.clone(),
                worker_id: registered.worker_id,
                token: registered.session_token.expose().to_owned(),
            });
        } else {
            sessions.push(w.session_as(&principal, activities, 100).await);
        }
    }
    // Iterations of a collection are opened by the scheduler's passes.
    let scheduler = if fanout {
        // Collections advance only under a placement that holds the shard fences.
        let placement = sagat_scheduler::Placement::new(format!("spike-{tag}").as_str());
        sagat_scheduler::Keeper::new(
            apis[0].store.clone(),
            placement.clone(),
            sagat_scheduler::KeeperSettings {
                ttl: Duration::from_secs(3_600),
            },
        )
        .round()
        .await
        .expect("acquire the shard fences");
        Some(Arc::new(
            sagat_scheduler::Scheduler::new(
                apis[0].store.clone(),
                sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
                sagat_application::entropy::SystemEntropy,
                sagat_scheduler::SchedulerSettings::default(),
            )
            .with_children(w.cache.clone())
            .with_placement(placement),
        ))
    } else {
        None
    };
    let ticker = scheduler.map(|scheduler| {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = tokio::spawn(async move {
            while !flag.load(Ordering::Relaxed) {
                let report = scheduler.tick().await;
                // Tick again at once while there is work, so the fan-out is not paced by the loop.
                if report.collections_advanced == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        });
        (stop, handle)
    });
    let deltas_before = deltas_size(&w.db.pool).await;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler = tokio::spawn({
        let (pool, stop) = (w.db.pool.clone(), Arc::clone(&stop));
        async move {
            let (mut waits, mut by_ingest, mut by_carry) = (0u64, 0u64, 0u64);
            while !stop.load(Ordering::Relaxed) {
                let row: Result<(i64, i64, i64), _> = sqlx::query_as(
                    "SELECT count(*), count(*) FILTER (WHERE EXISTS ( \
                         SELECT 1 FROM pg_stat_activity b WHERE b.pid = ANY (pg_blocking_pids(a.pid)) \
                            AND b.application_name = $1)), \
                            count(*) FILTER (WHERE EXISTS ( \
                         SELECT 1 FROM pg_stat_activity b WHERE b.pid = ANY (pg_blocking_pids(a.pid)) \
                            AND b.application_name = $2)) \
                       FROM pg_stat_activity a \
                      WHERE a.wait_event_type = 'Lock' AND a.backend_type = 'client backend' \
                        AND a.pid <> pg_backend_pid() AND a.application_name <> $1 AND a.query = 'COMMIT'",
                )
                .bind(INGEST_APPLICATION)
                .bind(CARRY_APPLICATION)
                .fetch_one(&pool)
                .await;
                if let Ok((n, m, c)) = row {
                    waits += u64::try_from(n).unwrap_or(0);
                    by_ingest += u64::try_from(m).unwrap_or(0);
                    by_carry += u64::try_from(c).unwrap_or(0);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            (waits, by_ingest, by_carry)
        }
    });
    let holder = if spec.hold {
        let mut conn = w.db.pool.acquire().await.expect("a connection to hold");
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .expect("begin");
        sqlx::query("SELECT pg_current_xact_id()")
            .execute(&mut *conn)
            .await
            .expect("an xid");
        Some((conn, Instant::now()))
    } else {
        None
    };
    let diag = if crate::step_log_diag::enabled() {
        Some(crate::step_log_diag::Probe::start(&w.db.pool, hook.clone()).await)
    } else {
        None
    };
    let started = Instant::now();
    let results =
        futures_util::future::join_all(sessions.iter().enumerate().map(|(n, session)| {
            let api = Arc::clone(&apis[n % spec.api_replicas]);
            let ingest = ingests[n % spec.api_replicas].clone();
            let lines = spec.lines;
            let single = spec.single_final;
            let carrying = spec.carry.is_some();
            let empty_output = spec.empty_output;
            let patience = if matches!(spec.shape, Shape::Fanout { .. }) {
                50
            } else {
                3
            };
            async move {
                let mut counters = crate::poll_client::PollCounters::default();
                let (mut completes, mut flushes, mut services) =
                    (Vec::new(), Vec::new(), Vec::new());
                let mut last_done = Instant::now();
                let mut empty = 0;
                while empty < patience {
                    let tasks = crate::poll_client::poll(&api, session, 16, &mut counters)
                        .await
                        .unwrap_or_else(|failure| panic!("poll: {:?}", failure.error))
                        .tasks;
                    if tasks.is_empty() {
                        empty += 1;
                        if patience > 3 {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        continue;
                    }
                    empty = 0;
                    for task in &tasks {
                        if carrying && let Some(ingest) = &ingest {
                            // (g): one batch before the report, its answer awaited, as a long step's SDK does.
                            let flush = Instant::now();
                            let batch = records(1, lines.0, lines.1);
                            let sent = send_until(
                                ingest,
                                session,
                                task,
                                &batch,
                                false,
                                0,
                                flush + Duration::from_secs(2),
                                &mut services,
                            )
                            .await;
                            if sent.is_none() {
                                ingest.tally.flush_given_up.fetch_add(1, Ordering::Relaxed);
                            }
                            flushes.push(flush.elapsed());
                        } else if let Some(ingest) = &ingest {
                            let flush = Instant::now();
                            let deadline = flush + Duration::from_secs(2);
                            let batch = records(1, lines.0, lines.1);
                            let highest = i64::try_from(lines.0).unwrap_or(0);
                            let first = if single {
                                None
                            } else {
                                send_until(
                                    ingest,
                                    session,
                                    task,
                                    &batch,
                                    false,
                                    0,
                                    deadline,
                                    &mut services,
                                )
                                .await
                            };
                            let last = if single {
                                send_until(
                                    ingest,
                                    session,
                                    task,
                                    &batch,
                                    true,
                                    highest,
                                    deadline,
                                    &mut services,
                                )
                                .await
                            } else {
                                match first {
                                    Some(_) => {
                                        send_until(
                                            ingest,
                                            session,
                                            task,
                                            &[],
                                            true,
                                            highest,
                                            deadline,
                                            &mut services,
                                        )
                                        .await
                                    }
                                    None => None,
                                }
                            };
                            if last.is_none() {
                                ingest.tally.flush_given_up.fetch_add(1, Ordering::Relaxed);
                            }
                            flushes.push(flush.elapsed());
                        }
                        let t = Instant::now();
                        api.workers
                            .complete(
                                &session.principal,
                                &crate::support::engine::task_ref(task),
                                &if empty_output {
                                    b"{}".to_vec()
                                } else {
                                    output(task)
                                },
                                &format!("done-{}", task.task_id()),
                            )
                            .await
                            .unwrap_or_else(|error| {
                                failure("complete", t.elapsed(), &api.execution_pool, &error)
                            });
                        completes.push(t.elapsed());
                        last_done = Instant::now();
                    }
                }
                (completes, flushes, services, last_done)
            }
        }))
        .await;
    stop.store(true, Ordering::Relaxed);
    if let Some((flag, handle)) = ticker {
        flag.store(true, Ordering::Relaxed);
        handle.await.expect("the ticker");
    }
    let held_for = if let Some((mut conn, since)) = holder {
        let held = since.elapsed();
        sqlx::query("ROLLBACK")
            .execute(&mut *conn)
            .await
            .expect("release the held transaction");
        Some(held)
    } else {
        None
    };
    let (waits, by_ingest, carry_waits) = sampler.await.expect("the sampler");
    let commit_waits = (waits, by_ingest);
    let deltas_after = deltas_size(&w.db.pool).await;
    for api in &apis {
        api.execution_pool.close().await;
    }
    let (mut completes, mut flushes, mut services) = (Vec::new(), Vec::new(), Vec::new());
    let mut finished = started;
    for (c, f, s, last) in results {
        completes.extend(c);
        flushes.extend(f);
        services.extend(s);
        finished = finished.max(last);
    }
    completes.sort_unstable();
    flushes.sort_unstable();
    services.sort_unstable();
    let transitions = completes.len();
    if transitions == 0 {
        let runs: Vec<(String, i64)> =
            sqlx::query_as("SELECT status, count(*) FROM runs WHERE workflow_id = $1 GROUP BY 1")
                .bind(wf.as_uuid())
                .fetch_all(&w.db.pool)
                .await
                .unwrap_or_default();
        let steps: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT s.node, s.status, count(*) FROM steps s JOIN runs r ON r.id = s.run_id \
              WHERE r.workflow_id = $1 GROUP BY 1, 2",
        )
        .bind(wf.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .unwrap_or_default();
        eprintln!(
            "spike {}: nothing completed; runs {runs:?}; steps {steps:?}",
            spec.label
        );
    }
    let expected = match spec.shape {
        Shape::Chain => spec.runs * 3,
        Shape::Fanout { items } => spec.runs * items,
    };
    assert_eq!(
        transitions, expected,
        "{}: every step completed",
        spec.label
    );
    let elapsed = finished - started;
    let diag = match diag {
        Some(probe) => Some(probe.finish(elapsed).await),
        None => None,
    };
    RowResult {
        transitions,
        per_second: transitions as f64 / elapsed.as_secs_f64().max(1e-9),
        completes,
        flushes,
        services,
        tally,
        commit_waits,
        deltas_bytes: (deltas_before, deltas_after),
        held_for,
        diag,
        carry: hook.filter(|hook| !hook.probe).map(|hook| hook.counts()),
        carry_waits,
    }
}

async fn deltas_size(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT pg_total_relation_size('public.quota_transaction_deltas')")
        .fetch_one(pool)
        .await
        .expect("the size of quota_transaction_deltas")
}

/// Workload (b): `attempts` leased attempts of one tenant on one replica each send full batches of
/// 500 records of `bytes` bytes until the attempt ceiling, at once. Returns the service times.
pub async fn full_batches(
    w: &Arc<crate::support::engine::World>,
    wf: sagat_domain::definition::WorkflowId,
    attempts: usize,
    tickets: usize,
    cap: usize,
    rounds: usize,
    bytes: usize,
) -> (Vec<Duration>, Arc<Tally>) {
    let api = Arc::new(w.replica_with_pool(crate::API_POOL).await);
    let tally = Arc::new(Tally::default());
    let ingest = Ingest {
        pool: api.execution_pool.clone(),
        admission: Admission::new(cap, tickets),
        tally: Arc::clone(&tally),
        chunk: false,
        stream_lock: false,
    };
    let session = w.session_as(&w.worker, &["act.big"], 1000).await;
    let mut services = Vec::new();
    for _ in 0..rounds {
        start(w, wf, Shape::Chain, attempts).await;
        let mut tasks = Vec::new();
        while tasks.len() < attempts {
            let mut counters = crate::poll_client::PollCounters::default();
            let polled = crate::poll_client::poll(&api, &session, 64, &mut counters)
                .await
                .unwrap_or_else(|failure| panic!("poll: {:?}", failure.error))
                .tasks;
            tasks.extend(polled);
        }
        let per: Vec<Vec<Duration>> = futures_util::future::join_all(tasks.iter().map(|task| {
            let (ingest, session) = (&ingest, &session);
            async move {
                let mut services = Vec::new();
                let mut next = 1;
                for _ in 0..16 {
                    let batch = records(next, 500, bytes);
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let verdict = send_until(
                        ingest,
                        session,
                        task,
                        &batch,
                        false,
                        0,
                        deadline,
                        &mut services,
                    )
                    .await;
                    next += 500;
                    if verdict != Some(Verdict::Continue) {
                        break;
                    }
                }
                services
            }
        }))
        .await;
        services.extend(per.into_iter().flatten());
        for task in &tasks {
            api.workers
                .complete(
                    &session.principal,
                    &crate::support::engine::task_ref(task),
                    b"{}",
                    &format!("done-{}", task.task_id()),
                )
                .await
                .expect("complete");
        }
    }
    services.sort_unstable();
    (services, tally)
}
