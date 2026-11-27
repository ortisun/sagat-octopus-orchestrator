//! Design 041, task T041-5: the step-log tables and the exact quota leases of ADR-0097, against a real PostgreSQL.
//!
//! Each test stores lines with a plain `INSERT` into `step_log_lines` (as an ingest batch does; its statement trigger
//! charges the writer's quota lease) or calls a function of data-model.md §3b directly, except those of `ingest`,
//! which call the ingest of task T041-6 itself. The carry's cases are task T041-36's. Races are made with held transactions, `pg_stat_activity` and `pg_locks`, never with sleeps.
//!
//! Writers connect as the runtime role when `SAGAT_TEST_AS_RUNTIME_ROLE` is set, so that every insert, top-up,
//! reservation and reclaim a test makes is one the runtime role may make; the ledger, the held usage rows and the
//! changes of a limit are the owner's, as an operator's would be.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection as _, PgConnection, PgPool};
use uuid::Uuid;

use crate::support::{self, TestDb, bounded};

mod accounting;
mod ingest;
mod leases;
mod reclaims;
mod schema;
mod top_up;
mod wait_slots;

/// The bound of each staged test.
pub(super) const LIMIT: Duration = Duration::from_secs(60);
/// The refill's share cap and the near-limit mark (ADR-0097 decision 4).
pub(super) const MIB: i64 = 1_048_576;
/// The per-row charge of OD-2026-11-15-6 (`fixed_octets` on `level`).
pub(super) const ROW: i64 = 128;
/// The largest message a line may hold (`step_log_lines_sizes`).
const MAX_MESSAGE: i64 = 8_192;
/// The top-up mark *M* (decision D041-124).
pub(super) const MARK: i64 = 65_536;

/// A migrated database with one tenant whose step-log limit is set, and a stream of it.
pub(super) struct Fx {
    pub db: TestDb,
    pub tenant: Uuid,
    pub stream: i64,
    next: AtomicI64,
}

/// What the tenant's accounting holds, read by the owner: S, G, Σ granted, Σ used and the number of quota leases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ledger {
    pub s: i64,
    pub g: i64,
    pub granted: i64,
    pub used: i64,
    pub leases: i64,
}

impl Ledger {
    /// U = S + Σ used.
    pub const fn usage(self) -> i64 {
        self.s + self.used
    }
}

/// A writer: a connection of its own, outside any pool (a quota lease belongs to the server backend that charges, D041-117).
pub(super) struct Writer {
    pub conn: PgConnection,
    pub pid: i32,
}

impl Fx {
    /// A fresh database, a tenant whose step-log limit and ceiling are `limit`, and one stream of it.
    pub async fn new(limit: i64) -> Self {
        let db = support::migrated_db().await;
        let tenant = tenant(&db.pool).await;
        set_limit(&db.pool, tenant, limit)
            .await
            .expect("the step-log limit");
        let stream = stream(&db.pool, tenant).await;
        Self {
            db,
            tenant,
            stream,
            next: AtomicI64::new(1),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.db.pool
    }

    /// Another tenant of the same database with its own limit, and a stream of it.
    pub async fn another_tenant(&self, limit: i64) -> (Uuid, i64) {
        let tenant = tenant(&self.db.pool).await;
        set_limit(&self.db.pool, tenant, limit)
            .await
            .expect("the step-log limit");
        (tenant, stream(&self.db.pool, tenant).await)
    }

    /// The connection options of a writer: the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE`, else the owner.
    pub fn writer_options(&self) -> PgConnectOptions {
        let url = if support::as_runtime_role() {
            self.db
                .url_for(support::RUNTIME_ROLE, support::RUNTIME_ROLE_PASSWORD)
        } else {
            self.db.url.clone()
        };
        url.parse().expect("the writer's options")
    }

    pub async fn writer(&self) -> Writer {
        self.writer_with(self.writer_options()).await
    }

    /// A writer whose connections are named `application_name`, as a replica's pool names them.
    pub async fn writer_named(&self, application_name: &str) -> Writer {
        self.writer_with(self.writer_options().application_name(application_name))
            .await
    }

    async fn writer_with(&self, options: PgConnectOptions) -> Writer {
        let mut conn = PgConnection::connect_with(&options)
            .await
            .expect("a writer");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut conn)
            .await
            .expect("the writer's backend");
        Writer { conn, pid }
    }

    /// The first of `count` new sequences (unique in the database, so any stream takes them).
    pub fn sequences(&self, count: i64) -> i64 {
        self.next.fetch_add(count, Ordering::SeqCst)
    }

    /// Lines on `stream` whose charge, their text plus 128 bytes a row, is exactly `bytes` (at least 128), in one insert
    /// statement, so one charge of the statement trigger.
    pub async fn charge(
        &self,
        conn: &mut PgConnection,
        tenant: Uuid,
        stream: i64,
        bytes: i64,
    ) -> Result<(), sqlx::Error> {
        let lengths = message_lengths(bytes);
        let first = self.sequences(i64::try_from(lengths.len()).expect("few rows"));
        insert_messages(conn, tenant, stream, first, &lengths).await
    }

    /// [`Self::charge`] for the fixture's tenant, in a transaction of its own, committed.
    pub async fn charge_committed(
        &self,
        conn: &mut PgConnection,
        bytes: i64,
    ) -> Result<(), sqlx::Error> {
        let mut tx = conn.begin().await?;
        self.charge(&mut tx, self.tenant, self.stream, bytes)
            .await?;
        tx.commit().await
    }

    pub async fn ledger(&self, tenant: Uuid) -> Ledger {
        ledger(&self.db.pool, tenant).await
    }

    /// ADR-0097's invariants on the committed state: G = Σ granted, 0 ≤ U ≤ S + G ≤ L.
    pub async fn assert_exact(&self, tenant: Uuid, limit: i64) {
        let l = self.ledger(tenant).await;
        assert_eq!(l.g, l.granted, "G is the sum of the grants: {l:?}");
        assert!(l.usage() >= 0, "the usage is negative: {l:?}");
        assert!(l.usage() <= l.s + l.g, "the usage passed S + G: {l:?}");
        assert!(l.s + l.g <= limit, "S + G passed the limit {limit}: {l:?}");
    }

    /// Replaces the fallback refill's budget with 300 s, so that a test's waiter waits until the test lets it go.
    pub async fn without_the_refill_bound(&self) {
        sqlx::query(
            "CREATE OR REPLACE FUNCTION sagat_quota_step_log_refill_wait() RETURNS interval LANGUAGE sql STABLE \
               SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$ SELECT interval '300 seconds' $$",
        )
        .execute(&self.db.pool)
        .await
        .expect("the test's bound");
    }
}

/// A tenant (its usage row is provisioned by `tenants_quota_usage`).
pub(super) async fn tenant(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, now())")
        .bind(id)
        .bind(format!("t{}", &id.simple().to_string()[..12]))
        .execute(pool)
        .await
        .expect("a tenant");
    id
}

/// Sets the tenant's step-log ceiling and limit to `limit`, as an operator's two writes; each fires the reclaim trigger.
pub(super) async fn set_limit(pool: &PgPool, tenant: Uuid, limit: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'tenant_step_log_bytes', $2, 'test') \
         ON CONFLICT (tenant_id, resource) DO UPDATE SET max_value = EXCLUDED.max_value",
    )
    .bind(tenant)
    .bind(limit)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM quota_limits WHERE tenant_id = $1 AND namespace_id IS NULL AND resource = 'tenant_step_log_bytes'",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_step_log_bytes', $2, 'test')",
    )
    .bind(tenant)
    .bind(limit)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// A worker stream of the tenant, committed by the owner.
pub(super) async fn stream(pool: &PgPool, tenant: Uuid) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
           instance_path_nodes, instance_path_indices, attempt, generation, retention_days, last_at, expires_at) \
         VALUES ($1, gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), 'a', '{}', '{}', \
           1, 0, 14, clock_timestamp(), clock_timestamp() + interval '14 days') RETURNING stream_id",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("a stream")
}

/// The message lengths of lines whose charge is exactly `bytes`: k = ⌈bytes / 8,320⌉ rows (8,192 bytes of text and 128
/// of row charge at most each), and bytes − 128 k of text spread over them. Every k ≥ 2 has 128 k ≤ 8,320 (k − 1) + 1,
/// and k = 1 needs bytes ≥ 128, so any charge of at least 128 bytes is reachable.
pub(super) fn message_lengths(bytes: i64) -> Vec<i64> {
    assert!(
        bytes >= ROW,
        "a charge is at least one row's 128 bytes: {bytes}"
    );
    let rows = (bytes + MAX_MESSAGE + ROW - 1) / (MAX_MESSAGE + ROW);
    let text = bytes - ROW * rows;
    assert!(
        (0..=MAX_MESSAGE * rows).contains(&text),
        "{bytes} bytes in {rows} rows"
    );
    (0..rows)
        .map(|i| text / rows + i64::from(i < text % rows))
        .collect()
}

/// One insert statement of `lengths.len()` `info` lines of ASCII text from sequence `first`.
pub(super) async fn insert_messages(
    conn: &mut PgConnection,
    tenant: Uuid,
    stream: i64,
    first: i64,
    lengths: &[i64],
) -> Result<(), sqlx::Error> {
    let sequences: Vec<i64> = (0..lengths.len())
        .map(|i| first + i64::try_from(i).expect("few rows"))
        .collect();
    let lengths: Vec<i32> = lengths
        .iter()
        .map(|n| i32::try_from(*n).expect("a message length"))
        .collect();
    sqlx::query(
        "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message) \
         SELECT $1, $2, t.s, gen_random_uuid(), 'info', 'line', repeat('x', t.n) \
           FROM unnest($3::bigint[], $4::integer[]) AS t(s, n)",
    )
    .bind(tenant)
    .bind(stream)
    .bind(sequences)
    .bind(lengths)
    .execute(conn)
    .await
    .map(|_| ())
}

pub(super) async fn ledger(pool: &PgPool, tenant: Uuid) -> Ledger {
    let (s, g, granted, used, leases): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT u.step_log_bytes, u.step_log_leased_bytes, \
                coalesce((SELECT sum(granted) FROM step_log_quota_leases WHERE tenant_id = $1), 0)::bigint, \
                coalesce((SELECT sum(used) FROM step_log_quota_leases WHERE tenant_id = $1), 0)::bigint, \
                (SELECT count(*) FROM step_log_quota_leases WHERE tenant_id = $1) \
           FROM tenant_retained_usage u WHERE u.tenant_id = $1",
    )
    .bind(tenant)
    .fetch_one(pool)
    .await
    .expect("the ledger");
    Ledger {
        s,
        g,
        granted,
        used,
        leases,
    }
}

/// The writer's quota lease of `tenant`: (granted, used), or `None`.
pub(super) async fn lease_of(pool: &PgPool, tenant: Uuid, pid: i32) -> Option<(i64, i64)> {
    sqlx::query_as(
        "SELECT granted, used FROM step_log_quota_leases WHERE tenant_id = $1 AND writer = $2",
    )
    .bind(tenant)
    .bind(pid)
    .fetch_optional(pool)
    .await
    .expect("the writer's quota lease")
}

/// Waits until `pid` waits on a heavyweight lock behind `holder`, or panics after the bound. The chain of blockers is
/// followed: PostgreSQL queues the second and later waiters for a row on the tuple lock of the first waiter, so
/// `pg_blocking_pids` of a later waiter names the first waiter, and only the first waiter's names the row's holder.
pub(super) async fn waits_on(pool: &PgPool, pid: i32, holder: i32) {
    bounded::within("the backend waits on the holder's lock", LIMIT, async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "WITH RECURSIVE chain (blocker) AS ( \
                     SELECT unnest(pg_blocking_pids($1)) \
                     UNION SELECT unnest(pg_blocking_pids(c.blocker)) FROM chain c) \
                 SELECT coalesce(a.wait_event_type = 'Lock' AND $2 IN (SELECT blocker FROM chain), false) \
                   FROM pg_stat_activity a WHERE a.pid = $1",
            )
            .bind(pid)
            .bind(holder)
            .fetch_one(pool)
            .await
            .expect("pg_stat_activity");
            if waiting {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// Waits until some backend of this database waits on an advisory lock (a fence), or panics after the bound.
pub(super) async fn a_backend_waits_on_a_fence(pool: &PgPool) {
    bounded::within("a backend waits on a fence", LIMIT, async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND wait_event = 'advisory' \
                   AND datname = current_database()",
            )
            .fetch_one(pool)
            .await
            .expect("pg_stat_activity");
            if waiting > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// The owner's transaction holding the tenant's usage row, as every real holder does (a refill, a top-up, a release, an
/// idle reclaim, a change of the limit): it locked it `FOR UPDATE` and updated it. An update alone takes only
/// `FOR NO KEY UPDATE`, which a foreign key's `FOR KEY SHARE` passes, so it would hide a wait behind a real holder.
pub(super) async fn hold_the_usage_row(
    pool: &PgPool,
    tenant: Uuid,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut tx = pool.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .expect("the holder's backend");
    sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .expect("lock the usage row");
    let updated = sqlx::query(
        "UPDATE tenant_retained_usage SET step_log_bytes = step_log_bytes WHERE tenant_id = $1",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("hold the usage row")
    .rows_affected();
    assert_eq!(updated, 1, "the tenant has its usage row");
    (tx, pid)
}

/// The SQLSTATE and the constraint name of a database error.
pub(super) fn refusal(result: &Result<(), sqlx::Error>) -> Option<(String, Option<String>)> {
    match result {
        Err(sqlx::Error::Database(e)) => Some((
            e.code().map(|c| c.into_owned()).unwrap_or_default(),
            e.constraint().map(str::to_owned),
        )),
        _ => None,
    }
}

pub(super) fn exceeded() -> Option<(String, Option<String>)> {
    Some((
        "54000".into(),
        Some("tenant_step_log_quota_exceeded".into()),
    ))
}

pub(super) fn held() -> Option<(String, Option<String>)> {
    Some(("54000".into(), Some("tenant_step_log_quota_held".into())))
}

pub(super) fn slots_taken() -> Option<(String, Option<String>)> {
    Some(("55P03".into(), Some("step_log_wait_slots_taken".into())))
}

/// A lock timeout (`55P03` with no constraint name): a wait for the usage row past its budget.
pub(super) fn lock_timeout() -> Option<(String, Option<String>)> {
    Some(("55P03".into(), None))
}

#[test]
fn the_message_lengths_charge_exactly_what_is_asked() {
    for bytes in [128, 129, 1_000, 8_320, 8_321, 16_640, 2_095_152, 4 * MIB] {
        let lengths = message_lengths(bytes);
        let rows = i64::try_from(lengths.len()).expect("few rows");
        assert_eq!(lengths.iter().sum::<i64>() + ROW * rows, bytes, "{bytes}");
        assert!(
            lengths.iter().all(|n| (0..=MAX_MESSAGE).contains(n)),
            "{bytes}"
        );
    }
    // 4 MiB = 4,194,304 bytes: ⌈4,194,304 / 8,320⌉ = 505 rows (504 × 8,320 = 4,193,280 is short), 64,640 bytes of row
    // charge and 4,129,664 of text.
    assert_eq!(message_lengths(4 * MIB).len(), 505);
}
