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
    pub times: std::sync::Mutex<Vec<Duration>>,
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
            one_call: std::env::var("SAGAT_PERF_STEP_LOG_CARRY_RUN").as_deref() == Ok("2"),
            records,
            bytes,
            calls: AtomicU64::new(0),
            written: AtomicU64::new(0),
            contended: AtomicU64::new(0),
            neither: AtomicU64::new(0),
            times: std::sync::Mutex::new(Vec::new()),
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
        match outcome.as_str() {
            "written" => self.written.fetch_add(1, Ordering::Relaxed),
            "contended" => self.contended.fetch_add(1, Ordering::Relaxed),
            _ => self.neither.fetch_add(1, Ordering::Relaxed),
        };
        self.record(started);
        Ok(())
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
    /// T041-35: the carry hook's counters (calls, written, contended, neither) and statement times, sorted. Read
    /// only by the carry spike, which needs `perf-spike`.
    #[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
    pub carry: Option<(u64, u64, u64, u64, Vec<Duration>)>,
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

async fn start(
    w: &crate::support::engine::World,
    wf: sagat_domain::definition::WorkflowId,
    shape: Shape,
    runs: usize,
) {
    use futures_util::StreamExt as _;
    let mut apis = Vec::new();
    for _ in 0..4 {
        apis.push(Arc::new(w.replica_with_pool(16).await));
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
                    .expect("start");
            }
        })
        .buffer_unordered(64)
        .collect()
        .await;
    assert_eq!(started.len(), runs);
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
                            .unwrap_or_else(|error| panic!("complete: {error:?}"));
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
        carry: hook.filter(|hook| !hook.probe).map(|hook| {
            let mut times = hook.times.lock().map(|t| t.clone()).unwrap_or_default();
            times.sort_unstable();
            (
                hook.calls.load(Ordering::Relaxed),
                hook.written.load(Ordering::Relaxed),
                hook.contended.load(Ordering::Relaxed),
                hook.neither.load(Ordering::Relaxed),
                times,
            )
        }),
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
