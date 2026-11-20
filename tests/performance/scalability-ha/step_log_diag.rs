//! The hot-row diagnosis of design 041's spike T041-35 (D041-105): the second run failed at 1 × 16 with commit waits
//! that grow with the carry. This module shows what a report's `COMMIT` waits on, and for how long the thing it waits
//! on is held, with `SAGAT_PERF_STEP_LOG_DIAG=1` only:
//!
//! - every 10 ms, the lock each backend waiting in `COMMIT` waits for (the relation of its tuple lock, or the type of
//!   its ungranted lock), and the wait event of each backend in `COMMIT` that is not waiting for a lock;
//! - the time at which each transaction locked the tenant usage row (an `AFTER UPDATE` trigger on
//!   `tenant_retained_usage`, which fires right after the finalizer's `SELECT … FOR UPDATE` and `UPDATE`), joined to when
//!   its `COMMIT` was sent and returned (the report hook times it);
//! - the WAL flushes and commits of the row (`pg_stat_io`, `pg_stat_database`, `pg_stat_wal`);
//! - the step-log charge moved: `none` (no charge, an ablation only) or `slot` (a prototype of a charge into one of 64
//!   per-tenant slots taken with `SKIP LOCKED`, never the usage row), beside `tenant` (the second run's charge).
//!
//! Nothing here reaches a migration; it runs against the scratch schema `spike_041` of `step_log_spike`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::step_log_spike::CarryHook;

/// Whether the hot-row diagnosis runs (`SAGAT_PERF_STEP_LOG_DIAG=1`).
pub fn enabled() -> bool {
    std::env::var("SAGAT_PERF_STEP_LOG_DIAG").as_deref() == Ok("1")
}

/// The diagnosis's scratch objects, after `step_log_spike::install_per_statement`.
#[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
pub async fn install(pool: &PgPool) {
    let statements = [
        "CREATE TABLE spike_041.charge_mode (singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton), \
           mode text NOT NULL CHECK (mode IN ('tenant', 'none', 'slot')))",
        "INSERT INTO spike_041.charge_mode (mode) VALUES ('tenant')",
        "CREATE TABLE spike_041.step_log_slots (tenant_id uuid NOT NULL, slot integer NOT NULL, \
           granted bigint NOT NULL, used bigint NOT NULL, PRIMARY KEY (tenant_id, slot), \
           CHECK (0 <= used AND used <= granted))",
        "CREATE SEQUENCE spike_041.slot_fallbacks",
        r"CREATE OR REPLACE FUNCTION spike_041.lines_charge() RETURNS trigger LANGUAGE plpgsql
            SET search_path = pg_catalog, pg_temp AS $$
        DECLARE r record; m text; n bigint;
        BEGIN
            SELECT mode INTO m FROM spike_041.charge_mode;
            IF m = 'none' THEN RETURN NULL; END IF;
            FOR r IN SELECT tenant_id,
                            sum(octet_length(coalesce(message, '')) + octet_length(coalesce(exception, ''))
                                + octet_length(coalesce(fields, '')) + 128)::bigint AS bytes
                       FROM inserted GROUP BY tenant_id LOOP
                IF m = 'slot' THEN
                    UPDATE spike_041.step_log_slots u SET used = u.used + r.bytes
                      FROM (SELECT slot FROM spike_041.step_log_slots
                             WHERE tenant_id = r.tenant_id AND granted - used >= r.bytes
                             ORDER BY (slot + pg_backend_pid()) % 64 LIMIT 1 FOR UPDATE SKIP LOCKED) c
                     WHERE u.tenant_id = r.tenant_id AND u.slot = c.slot;
                    GET DIAGNOSTICS n = ROW_COUNT;
                    IF n = 1 THEN CONTINUE; END IF;
                    PERFORM nextval('spike_041.slot_fallbacks');
                END IF;
                PERFORM public.sagat_quota_add_delta(r.tenant_id, 0, 0, r.bytes);
            END LOOP;
            RETURN NULL;
        END $$",
        "CREATE UNLOGGED TABLE spike_041.lock_times (xid bigint NOT NULL, locked_at_us bigint NOT NULL)",
        r"CREATE FUNCTION spike_041.usage_locked() RETURNS trigger LANGUAGE plpgsql
            SET search_path = pg_catalog, pg_temp AS $$
        BEGIN
            INSERT INTO spike_041.lock_times
            VALUES (pg_current_xact_id()::text::bigint, (extract(epoch FROM clock_timestamp()) * 1000000)::bigint);
            RETURN NULL;
        END $$",
        "CREATE TRIGGER tenant_retained_usage_spike_locked AFTER UPDATE ON public.tenant_retained_usage \
           FOR EACH ROW EXECUTE FUNCTION spike_041.usage_locked()",
        "ALTER TABLE public.tenant_retained_usage ENABLE ALWAYS TRIGGER tenant_retained_usage_spike_locked",
    ];
    for statement in statements {
        sqlx::query(statement)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("diagnosis schema: {error}"));
    }
}

/// The cost breakdown of the carry (D041-107): copies of the line table without its two secondary indexes, without its
/// foreign key, unlogged; a chunk table holding a carry's records in one row; and the carry function again, reading a
/// mode at its start (`full`, `locks`, `stream`, `noidx`, `nofk`, `chunk`, `unlogged`). Every mode reads the mode, so
/// each row pays that read alike. Run with the step-log charge `none`, so the storage alone is compared.
#[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
pub async fn install_cost(pool: &PgPool) {
    let statements = [
        "CREATE TABLE spike_041.carry_mode (singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton), \
           mode text NOT NULL CHECK (mode IN ('full', 'locks', 'stream', 'noidx', 'nofk', 'chunk', 'unlogged')))",
        "INSERT INTO spike_041.carry_mode (mode) VALUES ('full')",
        "CREATE TABLE spike_041.lines_noidx (LIKE spike_041.step_log_lines INCLUDING DEFAULTS INCLUDING CONSTRAINTS)",
        "ALTER TABLE spike_041.lines_noidx ADD PRIMARY KEY (tenant_id, stream_id, sequence), \
           ADD FOREIGN KEY (tenant_id, stream_id) REFERENCES spike_041.step_log_streams (tenant_id, stream_id)",
        "CREATE TABLE spike_041.lines_nofk (LIKE spike_041.step_log_lines INCLUDING DEFAULTS INCLUDING CONSTRAINTS \
           INCLUDING INDEXES)",
        "CREATE UNLOGGED TABLE spike_041.lines_unlogged (LIKE spike_041.step_log_lines INCLUDING DEFAULTS \
           INCLUDING CONSTRAINTS INCLUDING INDEXES)",
        "ALTER TABLE spike_041.lines_unlogged ADD FOREIGN KEY (tenant_id, stream_id) \
           REFERENCES spike_041.step_log_streams (tenant_id, stream_id)",
        r"CREATE TABLE spike_041.chunks (
            tenant_id uuid NOT NULL, stream_id bigint NOT NULL, first_sequence bigint NOT NULL,
            last_sequence bigint NOT NULL, run_id uuid NOT NULL,
            stored_xid xid8 NOT NULL DEFAULT pg_current_xact_id(), stored_at timestamptz NOT NULL DEFAULT clock_timestamp(),
            records integer NOT NULL, worker_times timestamptz[] NOT NULL, levels text[] NOT NULL, kinds text[] NOT NULL,
            messages text[] NOT NULL, exceptions text[], fields text[], truncated boolean[],
            PRIMARY KEY (tenant_id, stream_id, first_sequence),
            FOREIGN KEY (tenant_id, stream_id) REFERENCES spike_041.step_log_streams (tenant_id, stream_id))",
        "CREATE INDEX chunks_read_idx ON spike_041.chunks (tenant_id, run_id, stored_xid, first_sequence)",
        "CREATE INDEX chunks_stream_idx ON spike_041.chunks (tenant_id, stream_id, stored_xid, first_sequence)",
        r"CREATE OR REPLACE FUNCTION spike_041.step_log_carry(p_tenant uuid, p_namespace uuid, p_run uuid, p_step uuid,
                p_node text, p_attempt integer, p_generation bigint, p_keys text[], p_stream_key text,
                p_resources text[], p_messages text[], p_application text)
            RETURNS text LANGUAGE plpgsql VOLATILE SET search_path = pg_catalog, public, pg_temp AS $$
        DECLARE
            k text; r record; v_mode text;
            v_records bigint := 10000; v_bytes bigint := 1048576; v_retention integer := 14;
            v_records_scoped boolean := false; v_bytes_scoped boolean := false; v_retention_scoped boolean := false;
            v_usage bigint; v_stream bigint; v_last bigint; v_count bigint; v_sum bigint;
            v_final boolean; v_removed boolean; v_in_order boolean; v_inserted integer := 0; v_size bigint := 0;
        BEGIN
            SELECT mode INTO v_mode FROM spike_041.carry_mode;
            PERFORM set_config('application_name', p_application, true);
            FOREACH k IN ARRAY p_keys LOOP
                IF NOT pg_try_advisory_xact_lock_shared(hashtextextended(k, 0)) THEN RETURN 'contended'; END IF;
            END LOOP;
            IF NOT pg_try_advisory_xact_lock(hashtextextended(p_stream_key, 0)) THEN RETURN 'contended'; END IF;
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
            IF v_mode = 'locks' THEN RETURN 'written'; END IF;
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
            IF v_count + cardinality(p_messages) > v_records THEN RETURN 'neither'; END IF;
            IF v_mode = 'full' THEN
                WITH ins AS (
                    INSERT INTO spike_041.step_log_lines (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                    SELECT p_tenant, v_stream, v_last + o, p_run, clock_timestamp(), 'info', 'line', m
                      FROM unnest(p_messages) WITH ORDINALITY AS u(m, o)
                    ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING
                    RETURNING octet_length(message) AS n)
                SELECT count(*)::integer, coalesce(sum(n), 0)::bigint INTO v_inserted, v_size FROM ins;
            ELSIF v_mode = 'noidx' THEN
                WITH ins AS (
                    INSERT INTO spike_041.lines_noidx (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                    SELECT p_tenant, v_stream, v_last + o, p_run, clock_timestamp(), 'info', 'line', m
                      FROM unnest(p_messages) WITH ORDINALITY AS u(m, o)
                    ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING
                    RETURNING octet_length(message) AS n)
                SELECT count(*)::integer, coalesce(sum(n), 0)::bigint INTO v_inserted, v_size FROM ins;
            ELSIF v_mode = 'nofk' THEN
                WITH ins AS (
                    INSERT INTO spike_041.lines_nofk (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                    SELECT p_tenant, v_stream, v_last + o, p_run, clock_timestamp(), 'info', 'line', m
                      FROM unnest(p_messages) WITH ORDINALITY AS u(m, o)
                    ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING
                    RETURNING octet_length(message) AS n)
                SELECT count(*)::integer, coalesce(sum(n), 0)::bigint INTO v_inserted, v_size FROM ins;
            ELSIF v_mode = 'unlogged' THEN
                WITH ins AS (
                    INSERT INTO spike_041.lines_unlogged (tenant_id, stream_id, sequence, run_id, worker_time, level, kind, message)
                    SELECT p_tenant, v_stream, v_last + o, p_run, clock_timestamp(), 'info', 'line', m
                      FROM unnest(p_messages) WITH ORDINALITY AS u(m, o)
                    ON CONFLICT (tenant_id, stream_id, sequence) DO NOTHING
                    RETURNING octet_length(message) AS n)
                SELECT count(*)::integer, coalesce(sum(n), 0)::bigint INTO v_inserted, v_size FROM ins;
            ELSIF v_mode = 'chunk' THEN
                INSERT INTO spike_041.chunks (tenant_id, stream_id, first_sequence, last_sequence, run_id, records,
                        worker_times, levels, kinds, messages)
                SELECT p_tenant, v_stream, v_last + 1, v_last + cardinality(p_messages), p_run, cardinality(p_messages),
                       array_fill(clock_timestamp(), ARRAY[cardinality(p_messages)]),
                       array_fill('info'::text, ARRAY[cardinality(p_messages)]),
                       array_fill('line'::text, ARRAY[cardinality(p_messages)]), p_messages
                ON CONFLICT (tenant_id, stream_id, first_sequence) DO NOTHING;
                GET DIAGNOSTICS v_inserted = ROW_COUNT;
                IF v_inserted = 1 THEN
                    v_inserted := cardinality(p_messages);
                    SELECT coalesce(sum(octet_length(m)), 0)::bigint INTO v_size FROM unnest(p_messages) AS u(m);
                END IF;
            END IF;
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
            .unwrap_or_else(|error| panic!("cost schema: {error}"));
    }
}

/// The carry's mode for the next rows (`install_cost`).
#[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
pub async fn set_carry_mode(pool: &PgPool, mode: &str) {
    sqlx::query("UPDATE spike_041.carry_mode SET mode = $1")
        .bind(mode)
        .execute(pool)
        .await
        .expect("set the carry mode");
}

/// Where the next rows' step-log charge goes: `tenant` (the usage row at commit), `none` or `slot`.
#[cfg_attr(not(feature = "perf-spike"), allow(dead_code))]
pub async fn set_charge(pool: &PgPool, tenant: Uuid, mode: &str) {
    sqlx::query("UPDATE spike_041.charge_mode SET mode = $1")
        .bind(mode)
        .execute(pool)
        .await
        .expect("set the charge mode");
    if mode == "slot" {
        // 64 slots of 1 TiB each: the prototype measures where the lock is, not the refill.
        sqlx::query(
            "INSERT INTO spike_041.step_log_slots (tenant_id, slot, granted, used) \
             SELECT $1, g, 1099511627776, 0 FROM generate_series(0, 63) g \
             ON CONFLICT (tenant_id, slot) DO UPDATE SET used = 0",
        )
        .bind(tenant)
        .execute(pool)
        .await
        .expect("fill the slots");
    }
}

#[derive(Default, Clone, Copy)]
struct Stats {
    commits: Option<i64>,
    wal_bytes: Option<i64>,
    fsyncs: Option<i64>,
    fsync_ms: Option<f64>,
    writes: Option<i64>,
    write_ms: Option<f64>,
    fallbacks: Option<i64>,
}

/// WAL fsyncs, their milliseconds, WAL writes and their milliseconds (`pg_stat_io`, PostgreSQL 18).
type WalIo = (Option<i64>, Option<f64>, Option<i64>, Option<f64>);

async fn stats(pool: &PgPool) -> Stats {
    let base: Result<(Option<i64>, Option<i64>, Option<i64>), _> = sqlx::query_as(
        "SELECT (SELECT xact_commit FROM pg_stat_database WHERE datname = current_database()), \
                (SELECT wal_bytes::bigint FROM pg_stat_wal), \
                (SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM spike_041.slot_fallbacks)",
    )
    .fetch_one(pool)
    .await;
    let io: Result<WalIo, _> = sqlx::query_as(
        "SELECT sum(fsyncs)::bigint, sum(fsync_time)::float8, sum(writes)::bigint, sum(write_time)::float8 \
           FROM pg_stat_io WHERE object = 'wal'",
    )
    .fetch_one(pool)
    .await;
    let (commits, wal_bytes, fallbacks) = base.unwrap_or_default();
    let (fsyncs, fsync_ms, writes, write_ms) = io.unwrap_or_default();
    Stats {
        commits,
        wal_bytes,
        fsyncs,
        fsync_ms,
        writes,
        write_ms,
        fallbacks,
    }
}

type Histograms = (BTreeMap<String, u64>, BTreeMap<String, u64>, u64);

/// One row's diagnosis, from `start` to `finish`.
pub struct Probe {
    pool: PgPool,
    hook: Option<Arc<CarryHook>>,
    before: Stats,
    stop: Arc<AtomicBool>,
    sampler: tokio::task::JoinHandle<Histograms>,
}

impl Probe {
    pub async fn start(pool: &PgPool, hook: Option<Arc<CarryHook>>) -> Self {
        sqlx::query("TRUNCATE spike_041.lock_times")
            .execute(pool)
            .await
            .expect("clear the lock times");
        let before = stats(pool).await;
        let stop = Arc::new(AtomicBool::new(false));
        let sampler = tokio::spawn({
            let (pool, stop) = (pool.clone(), Arc::clone(&stop));
            async move {
                let (mut targets, mut states, mut samples) =
                    (BTreeMap::new(), BTreeMap::new(), 0u64);
                while !stop.load(Ordering::Relaxed) {
                    // What a backend waiting in COMMIT waits for: the relation of the tuple lock it holds or waits
                    // for (a row lock), else the type of its ungranted lock; and what each backend in COMMIT that
                    // does not wait for a lock is doing (the holder of a row lock is one of them).
                    let rows: Result<Vec<(String, String, i64)>, _> = sqlx::query_as(
                        "SELECT 'waits on', coalesce( \
                                (SELECT l.relation::regclass::text FROM pg_locks l \
                                  WHERE l.pid = a.pid AND l.locktype = 'tuple' LIMIT 1), \
                                (SELECT l.locktype FROM pg_locks l WHERE l.pid = a.pid AND NOT l.granted LIMIT 1), \
                                'none'), count(*) \
                           FROM pg_stat_activity a \
                          WHERE a.wait_event_type = 'Lock' AND a.backend_type = 'client backend' \
                            AND a.pid <> pg_backend_pid() AND a.query = 'COMMIT' GROUP BY 2 \
                         UNION ALL \
                         SELECT 'committing', coalesce(a.wait_event_type, 'CPU') || ':' \
                                || coalesce(a.wait_event, 'running'), count(*) \
                           FROM pg_stat_activity a \
                          WHERE a.backend_type = 'client backend' AND a.pid <> pg_backend_pid() \
                            AND a.query = 'COMMIT' AND a.state = 'active' \
                            AND a.wait_event_type IS DISTINCT FROM 'Lock' GROUP BY 2",
                    )
                    .fetch_all(&pool)
                    .await;
                    if let Ok(rows) = rows {
                        samples += 1;
                        for (kind, key, n) in rows {
                            let into = if kind == "waits on" {
                                &mut targets
                            } else {
                                &mut states
                            };
                            *into.entry(key).or_insert(0) += u64::try_from(n).unwrap_or(0);
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                (targets, states, samples)
            }
        });
        Self {
            pool: pool.clone(),
            hook,
            before,
            stop,
            sampler,
        }
    }

    /// Stops the sampler and reports the row's diagnosis.
    #[allow(clippy::too_many_lines)]
    pub async fn finish(self, elapsed: Duration) -> serde_json::Value {
        self.stop.store(true, Ordering::Relaxed);
        let (targets, states, samples) = self.sampler.await.expect("the diagnosis sampler");
        let after = stats(&self.pool).await;
        let locks: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT xid, locked_at_us FROM spike_041.lock_times ORDER BY locked_at_us",
        )
        .fetch_all(&self.pool)
        .await
        .expect("the lock times");
        let committed: HashMap<i64, (i64, i64)> = self
            .hook
            .as_ref()
            .and_then(|hook| hook.committed.lock().ok().map(|c| c.clone()))
            .unwrap_or_default()
            .into_iter()
            .map(|(xid, sent, returned)| (xid, (sent, returned)))
            .collect();
        let locked: HashMap<i64, i64> = locks.iter().copied().collect();
        // Per timed report that locked the usage row: from COMMIT sent to the lock (the deferred triggers before it
        // and the wait), and from the lock to COMMIT returned (an upper bound of the hold: the lock is released
        // before the client sees the answer).
        let (mut to_lock, mut hold_upper, mut commit_locking, mut commit_other) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (xid, (sent, returned)) in &committed {
            if let Some(at) = locked.get(xid) {
                to_lock.push(at - sent);
                hold_upper.push(returned - at);
                commit_locking.push(returned - sent);
            } else {
                commit_other.push(returned - sent);
            }
        }
        // Back to back: the next locker's COMMIT was sent before this one locked, so it was waiting, and it locked
        // when this one released: the gap is this one's hold, plus the hand-over. The busy time of the row is the sum
        // over lockers of the lesser of the hold's upper bound and the gap to the next locker.
        let (mut back_to_back, mut busy_us) = (Vec::new(), 0i64);
        for pair in locks.windows(2) {
            let ((this, at), (next, next_at)) = (pair[0], pair[1]);
            let gap = next_at - at;
            if let Some((next_sent, _)) = committed.get(&next)
                && *next_sent < at
            {
                back_to_back.push(gap);
            }
            let hold = committed
                .get(&this)
                .map_or(gap, |(_, returned)| (returned - at).min(gap));
            busy_us += hold.max(0);
        }
        let d = |a: Option<i64>, b: Option<i64>| a.zip(b).map(|(a, b)| b - a);
        let df = |a: Option<f64>, b: Option<f64>| a.zip(b).map(|(a, b)| b - a);
        let commits = d(self.before.commits, after.commits);
        let fsyncs = d(self.before.fsyncs, after.fsyncs);
        let fsync_ms = df(self.before.fsync_ms, after.fsync_ms);
        let writes = d(self.before.writes, after.writes);
        let write_ms = df(self.before.write_ms, after.write_ms);
        let wal_bytes = d(self.before.wal_bytes, after.wal_bytes);
        #[allow(clippy::cast_precision_loss)]
        let per = |a: Option<f64>, b: Option<i64>| {
            a.zip(b).and_then(|(a, b)| (b > 0).then(|| a / b as f64))
        };
        #[allow(clippy::cast_precision_loss)]
        let ratio = |a: Option<i64>, b: Option<i64>| per(a.map(|a| a as f64), b);
        json!({
            "samples": samples,
            "waits_on": targets,
            "committing": states,
            "lockers": locks.len(),
            "timed_reports": committed.len(),
            "timed_lockers": hold_upper.len(),
            "to_lock_us": percentiles(&mut to_lock),
            "hold_upper_us": percentiles(&mut hold_upper),
            "back_to_back_gap_us": percentiles(&mut back_to_back),
            "commit_locking_us": percentiles(&mut commit_locking),
            "commit_other_us": percentiles(&mut commit_other),
            "row_busy_fraction": busy_fraction(busy_us, elapsed),
            "commits": commits,
            "wal_fsyncs": fsyncs,
            "wal_fsync_ms": fsync_ms,
            "wal_fsync_ms_mean": per(fsync_ms, fsyncs),
            "wal_writes": writes,
            "wal_write_ms": write_ms,
            "commits_per_fsync": ratio(commits, fsyncs),
            "wal_bytes_per_commit": ratio(wal_bytes, commits),
            "slot_fallbacks": d(self.before.fallbacks, after.fallbacks),
        })
    }
}

/// p50, p90, p99 and the count of `values` (sorted in place).
fn percentiles(values: &mut [i64]) -> serde_json::Value {
    values.sort_unstable();
    let at = |q: f64| -> Option<i64> {
        if values.is_empty() {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let i = ((values.len() - 1) as f64 * q).round() as usize;
        values.get(i).copied()
    };
    json!({ "n": values.len(), "p50": at(0.50), "p90": at(0.90), "p99": at(0.99) })
}

/// The share of `elapsed` in which the usage row was held.
#[allow(clippy::cast_precision_loss)]
fn busy_fraction(busy_us: i64, elapsed: Duration) -> Option<f64> {
    let elapsed = elapsed.as_micros() as f64;
    (elapsed > 0.0).then(|| busy_us.max(0) as f64 / elapsed)
}
