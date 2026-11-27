//! Design 041 section 2a, task T041-36, first slice: the carry's SQL function `sagat_step_log_carry` (migration
//! `20270219000000_step_log_carry`, data-model.md §8a) against a real PostgreSQL, called directly, before the report
//! path calls it.
//!
//! * The keys: the policy fences and the stream lock the function builds in SQL are exactly the keys the ingest builds
//!   in Rust (`step_log_policy_fence_keys`, `stream_lock_key`) for the same inputs (decision D041-146 (3)).
//! * The plan: the function and the domain's `plan_batch` run on the same generated carries (new and existing streams,
//!   each ceiling met, the tenant allowance exceeded, `in_order` false, duplicates, gaps, each planned mark-only reason
//!   and each answer of the block) and must keep the same sequences and marker, leave the same counters and count the
//!   same drops by reason (decision D041-100). The policies the function resolves are compared with the domain's
//!   `resolve` on the same stored rows.
//! * The function refuses a mark-only reason outside the four the application plans, and its migration never names
//!   the usage row (FR-041-008).
//!
//! Each generated case runs in a transaction of its own that is rolled back, so that the tenant's usage stays 0 and
//! the cases do not depend on one another; the function runs on a pool of the runtime role under
//! `SAGAT_TEST_AS_RUNTIME_ROLE`, the fixtures are the owner's.

use std::collections::BTreeMap;

use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::{QuotaResource, ceiling, resolve};
use sagat_domain::step_logs::{
    AttemptCeilings, CarryMark, CeilingReason, DropReason, FIRST_MARKER_SEQUENCE, MarkOnlyReason,
    NotStoredReason, Plan, SizedRecord, StepLogLevel, StreamCounters, TenantAllowance, plan_batch,
};
use sagat_infra_postgres::{step_log_policy_fence_keys, stream_lock_key};
use serde_json::Value;
use sqlx::{Connection as _, PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::support::engine::{World, world};
use crate::support::step_logs::pool_of;

/// The migration that creates the carry's functions.
const MIGRATION: &str =
    include_str!("../../../crates/infra-postgres/migrations/20270219000000_step_log_carry.up.sql");

/// The generated cases of the plan's property test.
const CASES: u64 = 300;
/// 2^53, where the drop counts saturate.
const SATURATION: u64 = 1 << 53;
/// The top-up mark M (decision D041-124).
const TOP_UP_MARK: i64 = 65_536;

/// SplitMix64: a small generator whose seed, printed with a failure, reproduces the case.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[usize::try_from(self.below(items.len() as u64)).expect("an index")]
    }
}

fn signed(value: u64) -> i64 {
    i64::try_from(value).expect("a value that fits a bigint")
}

/// A record as the function receives it.
#[derive(Debug, Clone)]
struct Rec {
    sequence: u64,
    level: StepLogLevel,
    message: String,
    exception: Option<String>,
    fields: Option<String>,
    truncated: bool,
}

impl Rec {
    /// The text bytes C3 computes and the statement trigger measures: the UTF-8 octets of the three parts.
    fn size(&self) -> u64 {
        (self.message.len()
            + self.exception.as_ref().map_or(0, String::len)
            + self.fields.as_ref().map_or(0, String::len)) as u64
    }
}

/// A stored setting of one resource: the operator's ceiling, the tenant's and the namespace's settings.
#[derive(Debug, Clone, Copy, Default)]
struct Settings {
    ceiling: Option<u64>,
    tenant: Option<u64>,
    namespace: Option<u64>,
}

impl Settings {
    /// The value in force, by the domain's rule (the rule `effective_policy` applies).
    fn resolved(self, resource: QuotaResource) -> u64 {
        resolve(
            resource,
            self.namespace,
            self.tenant,
            ceiling(resource, self.ceiling),
        )
        .0
    }
}

/// An existing stream: its counters and whether a batch stored after this transaction began.
#[derive(Debug, Clone, Copy)]
struct Existing {
    counters: StreamCounters,
    in_order: bool,
}

#[derive(Debug, Clone)]
struct Case {
    existing: Option<Existing>,
    records_policy: Settings,
    bytes_policy: Settings,
    retention_policy: Settings,
    /// The tenant's `tenant_step_log_bytes`, when the case sets one (else the 1 GiB default).
    tenant_limit: Option<i64>,
    records: Vec<Rec>,
    highest: u64,
    mark: Option<MarkOnlyReason>,
    /// Run on the connection whose quota lease was topped up (the fast path when its room covers the carry).
    topped_up: bool,
}

fn text(rng: &mut Rng, max: u64) -> String {
    const ALPHABET: [&str; 6] = ["a", "b", "é", "€", " ", "z"];
    (0..rng.below(max + 1))
        .map(|_| rng.pick(&ALPHABET))
        .collect()
}

fn settings(rng: &mut Rng, low: u64, high: u64, off: bool) -> Settings {
    let value = |rng: &mut Rng| {
        rng.chance(35).then(|| {
            if off && rng.chance(8) {
                0
            } else {
                low + rng.below(high - low + 1)
            }
        })
    };
    Settings {
        ceiling: value(rng),
        tenant: value(rng),
        namespace: value(rng),
    }
}

fn reason_of_ceiling(rng: &mut Rng) -> CeilingReason {
    rng.pick(&CeilingReason::ALL)
}

fn generate(rng: &mut Rng) -> Case {
    let existing = rng.chance(60).then(|| {
        let last = rng.below(31);
        let records = rng.below(last + 1);
        let ceiling = rng.chance(25).then(|| reason_of_ceiling(rng));
        let near = |rng: &mut Rng| {
            if rng.chance(5) {
                SATURATION - rng.below(3)
            } else {
                rng.below(5)
            }
        };
        Existing {
            counters: StreamCounters {
                records,
                bytes: records * rng.below(60),
                last_sequence: last,
                ceiling_sequence: ceiling.map(|_| rng.below(last + 1)),
                ceiling,
                not_stored_reason: rng.chance(15).then_some(NotStoredReason::ServiceBusy),
                dropped_by_worker: near(rng),
                dropped_by_server: near(rng),
                next_marker_sequence: FIRST_MARKER_SEQUENCE + rng.below(3),
                truncated_records: rng.below(records + 1),
            },
            in_order: rng.chance(80),
        }
    });
    let last = existing.map_or(0, |e| e.counters.last_sequence);
    let mark = rng.chance(30).then(|| rng.pick(&MarkOnlyReason::PLANNED));
    let n = rng.below(13);
    let record = |rng: &mut Rng, sequence: u64| Rec {
        sequence,
        level: rng.pick(&StepLogLevel::ALL),
        message: text(rng, 120),
        exception: rng.chance(20).then(|| text(rng, 40)),
        fields: rng
            .chance(20)
            .then(|| format!("{{\"k\":\"{}\"}}", text(rng, 20))),
        truncated: rng.chance(15),
    };
    let (records, highest) = if mark.is_some() {
        // A mark-only carry's sequences in any order, with duplicates.
        let records: Vec<Rec> = (0..n)
            .map(|_| {
                let sequence = 1 + rng.below(last + 15);
                record(rng, sequence)
            })
            .collect();
        (records, rng.below(last + 13))
    } else {
        let mut sequence = if existing.is_some() {
            last.saturating_sub(rng.below(4)).max(1)
        } else {
            1 + rng.below(3)
        };
        let mut records = Vec::new();
        for _ in 0..n {
            records.push(record(rng, sequence));
            sequence += 1 + if rng.chance(20) { rng.below(3) } else { 0 };
        }
        let top = records.last().map_or_else(
            || last.saturating_sub(rng.below(3)),
            |record| record.sequence,
        );
        (records, top + if rng.chance(50) { rng.below(4) } else { 0 })
    };
    Case {
        existing,
        records_policy: settings(rng, 1, 40, true),
        // A byte setting is at least 1,024 (`quota_limits_value_range`).
        bytes_policy: settings(rng, 1_024, 3_000, false),
        retention_policy: settings(rng, 1, 365, false),
        tenant_limit: rng.chance(15).then(|| signed(200 + rng.below(2_800))),
        records,
        highest,
        mark,
        topped_up: rng.chance(50),
    }
}

/// The policy rows of the case, written by the owner as an operator and an administrator would.
async fn write_policies(w: &World, case: &Case) {
    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let mut tx = w.db.pool.begin().await.expect("a transaction");
    for statement in [
        "DELETE FROM quota_limits WHERE tenant_id = $1 AND resource LIKE 'step_log_%'",
        "DELETE FROM quota_ceilings WHERE tenant_id = $1 AND resource LIKE 'step_log_%'",
    ] {
        sqlx::query(statement)
            .bind(tenant)
            .execute(&mut *tx)
            .await
            .expect("clear the policies");
    }
    for (resource, s) in [
        (QuotaResource::StepLogAttemptRecords, case.records_policy),
        (QuotaResource::StepLogAttemptBytes, case.bytes_policy),
        (QuotaResource::StepLogRetentionDays, case.retention_policy),
    ] {
        if let Some(value) = s.ceiling {
            sqlx::query(
                "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) VALUES ($1, $2, $3, 'test')",
            )
            .bind(tenant)
            .bind(resource.as_str())
            .bind(signed(value))
            .execute(&mut *tx)
            .await
            .expect("a ceiling");
        }
        for (scope, value) in [(None, s.tenant), (Some(namespace), s.namespace)] {
            if let Some(value) = value {
                sqlx::query(
                    "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
                     VALUES ($1, $2, $3, $4, 'test')",
                )
                .bind(tenant)
                .bind(scope)
                .bind(resource.as_str())
                .bind(signed(value))
                .execute(&mut *tx)
                .await
                .expect("a setting");
            }
        }
    }
    tx.commit().await.expect("the policies");
}

/// Sets or clears the tenant's `tenant_step_log_bytes` when it differs from the last case's: each write reclaims every
/// quota lease of the tenant (ADR-0097 decision 6).
async fn write_tenant_limit(w: &World, limit: Option<i64>, current: &mut Option<i64>) {
    if *current == limit {
        return;
    }
    let tenant = w.admin.tenant_id().as_uuid();
    sqlx::query(
        "DELETE FROM quota_limits WHERE tenant_id = $1 AND namespace_id IS NULL AND resource = 'tenant_step_log_bytes'",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await
    .expect("clear the tenant's limit");
    if let Some(limit) = limit {
        sqlx::query(
            "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
             VALUES ($1, NULL, 'tenant_step_log_bytes', $2, 'test')",
        )
        .bind(tenant)
        .bind(limit)
        .execute(&w.db.pool)
        .await
        .expect("the tenant's limit");
    }
    *current = limit;
}

/// The identity of the attempt a case carries for: a run and a step of their own, attempt 1, generation 0.
#[derive(Debug, Clone, Copy)]
struct Identity {
    tenant: Uuid,
    namespace: Uuid,
    run: Uuid,
    step: Uuid,
}

/// Inserts the existing stream of a case in the case's transaction, as the runtime role may.
async fn insert_existing(conn: &mut PgConnection, id: Identity, existing: Existing) {
    let c = existing.counters;
    sqlx::query(
        "INSERT INTO step_log_streams (tenant_id, public_id, namespace_id, run_id, step_id, node, \
                instance_path_nodes, instance_path_indices, attempt, generation, retention_days, records, bytes, \
                last_sequence, ceiling_sequence, next_marker_sequence, dropped_by_worker, dropped_by_server, \
                ceiling_reason, not_stored_reason, truncated_records, last_stored_xid, last_at, expires_at) \
         VALUES ($1, gen_random_uuid(), $2, $3, $4, 'act', '{}', '{}', 1, 0, 14, $5, $6, $7, $8, $9, $10, $11, $12, \
                 $13, $14, CASE WHEN $15 THEN '1'::xid8 \
                                ELSE (pg_current_xact_id()::text::bigint + 1000000)::text::xid8 END, \
                 clock_timestamp(), clock_timestamp() + interval '14 days')",
    )
    .bind(id.tenant)
    .bind(id.namespace)
    .bind(id.run)
    .bind(id.step)
    .bind(i32::try_from(c.records).expect("records"))
    .bind(signed(c.bytes))
    .bind(signed(c.last_sequence))
    .bind(c.ceiling_sequence.map(signed))
    .bind(signed(c.next_marker_sequence))
    .bind(signed(c.dropped_by_worker))
    .bind(signed(c.dropped_by_server))
    .bind(c.ceiling.map(CeilingReason::as_str))
    .bind(c.not_stored_reason.map(NotStoredReason::as_str))
    .bind(i32::try_from(c.truncated_records).expect("truncated"))
    .bind(existing.in_order)
    .execute(&mut *conn)
    .await
    .expect("the existing stream");
}

/// What the function returned.
#[derive(Debug)]
struct Answer {
    outcome: String,
    kept: i32,
    dropped_by_worker: i64,
    dropped_by_server: Value,
    ceiling_reason: Option<String>,
    not_stored_reason: Option<String>,
    cause: Option<String>,
    path: String,
    room: Option<i64>,
    top_up: bool,
}

/// One call of the function, as the store will make it.
async fn carry(
    conn: &mut PgConnection,
    id: Identity,
    records: &[Rec],
    highest: u64,
    mark: Option<&str>,
) -> Result<Answer, sqlx::Error> {
    carry_with_slots(conn, id, records, highest, mark, 5).await
}

/// [`carry`] with *Q*, the replica's wait slots the store passes.
async fn carry_with_slots(
    conn: &mut PgConnection,
    id: Identity,
    records: &[Rec],
    highest: u64,
    mark: Option<&str>,
    wait_slots: i32,
) -> Result<Answer, sqlx::Error> {
    let row = sqlx::query(
        "SELECT outcome, kept, dropped_by_worker, dropped_by_server, ceiling_reason, not_stored_reason, cause, path, \
                room, top_up \
           FROM sagat_step_log_carry($1, $2, $3, $4, 'act', '{}', '{}', 1, 0, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                                     $14, $15)",
    )
    .bind(id.tenant)
    .bind(id.namespace)
    .bind(id.run)
    .bind(id.step)
    .bind(records.iter().map(|r| signed(r.sequence)).collect::<Vec<_>>())
    .bind(records.iter().map(|r| r.level.as_str()).collect::<Vec<_>>())
    .bind(
        records
            .iter()
            .map(|_| None::<time::OffsetDateTime>)
            .collect::<Vec<_>>(),
    )
    .bind(records.iter().map(|r| r.message.clone()).collect::<Vec<_>>())
    .bind(records.iter().map(|r| r.exception.clone()).collect::<Vec<_>>())
    .bind(records.iter().map(|r| r.fields.clone()).collect::<Vec<_>>())
    .bind(records.iter().map(|r| r.truncated).collect::<Vec<_>>())
    .bind(
        records
            .iter()
            .map(|r| i32::try_from(r.size()).expect("a size"))
            .collect::<Vec<_>>(),
    )
    .bind(signed(highest))
    .bind(wait_slots)
    .bind(mark)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Answer {
        outcome: row.try_get("outcome")?,
        kept: row.try_get("kept")?,
        dropped_by_worker: row.try_get("dropped_by_worker")?,
        dropped_by_server: row.try_get("dropped_by_server")?,
        ceiling_reason: row.try_get("ceiling_reason")?,
        not_stored_reason: row.try_get("not_stored_reason")?,
        cause: row.try_get("cause")?,
        path: row.try_get("path")?,
        room: row.try_get("room")?,
        top_up: row.try_get("top_up")?,
    })
}

/// The stream row after the call, read in the case's transaction.
#[derive(Debug, PartialEq, Eq)]
struct Stored {
    counters: StreamCounters,
    retention_days: i16,
    final_mark: bool,
    stored_by_this_transaction: bool,
    stream_id: i64,
}

async fn stored(conn: &mut PgConnection, id: Identity) -> Option<Stored> {
    let row = sqlx::query(
        "SELECT stream_id, records, bytes, last_sequence, ceiling_sequence, ceiling_reason, not_stored_reason, \
                dropped_by_worker, dropped_by_server, next_marker_sequence, truncated_records, retention_days, \
                final_at IS NOT NULL AS final_mark, \
                last_stored_xid IS NOT DISTINCT FROM pg_current_xact_id() AS mine \
           FROM step_log_streams WHERE tenant_id = $1 AND run_id = $2 AND step_id = $3 AND attempt = 1 \
            AND generation = 0",
    )
    .bind(id.tenant)
    .bind(id.run)
    .bind(id.step)
    .fetch_optional(&mut *conn)
    .await
    .expect("the stream")?;
    let int = |name: &str| u64::try_from(row.get::<i64, _>(name)).expect(name);
    let small = |name: &str| u64::try_from(row.get::<i32, _>(name)).expect(name);
    let ceiling: Option<String> = row.get("ceiling_reason");
    let not_stored: Option<String> = row.get("not_stored_reason");
    Some(Stored {
        counters: StreamCounters {
            records: small("records"),
            bytes: int("bytes"),
            last_sequence: int("last_sequence"),
            ceiling_sequence: row
                .get::<Option<i64>, _>("ceiling_sequence")
                .map(|v| u64::try_from(v).expect("a sequence")),
            ceiling: ceiling.map(|t| CeilingReason::parse(&t).expect("a ceiling reason")),
            not_stored_reason: not_stored
                .map(|t| NotStoredReason::parse(&t).expect("a not-stored reason")),
            dropped_by_worker: int("dropped_by_worker"),
            dropped_by_server: int("dropped_by_server"),
            next_marker_sequence: int("next_marker_sequence"),
            truncated_records: small("truncated_records"),
        },
        retention_days: row.get("retention_days"),
        final_mark: row.get("final_mark"),
        stored_by_this_transaction: row.get("mine"),
        stream_id: row.get("stream_id"),
    })
}

/// A line as stored: (sequence, kind, level, message, exception, fields, truncated, marker reason).
type Line = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
);

async fn lines_stored_now(conn: &mut PgConnection, tenant: Uuid, stream: i64) -> Vec<Line> {
    sqlx::query_as(
        "SELECT sequence, kind, level, message, exception, fields, truncated, marker_reason FROM step_log_lines \
          WHERE tenant_id = $1 AND stream_id = $2 AND stored_xid = pg_current_xact_id() ORDER BY sequence",
    )
    .bind(tenant)
    .bind(stream)
    .fetch_all(&mut *conn)
    .await
    .expect("the lines")
}

/// The remaining allowance the function reads (T6): none when the quota is not enforced.
async fn allowance(conn: &mut PgConnection, tenant: Uuid) -> TenantAllowance {
    let (used, limit, enforced): (i64, i64, bool) =
        sqlx::query_as("SELECT used, max_value, enforced FROM sagat_quota_step_log_allowance($1)")
            .bind(tenant)
            .fetch_one(&mut *conn)
            .await
            .expect("the allowance");
    TenantAllowance {
        remaining: enforced.then(|| {
            u64::try_from(limit)
                .unwrap_or(0)
                .saturating_sub(u64::try_from(used).unwrap_or(0))
        }),
    }
}

/// The drops by the server of a plan, as the function returns them: every server reason with a count.
fn server_drops(plan: &Plan) -> Value {
    let map: serde_json::Map<String, Value> = plan
        .drops
        .iter()
        .filter(|(reason, _)| *reason != DropReason::WorkerGap)
        .map(|(reason, count)| (reason.as_str().to_owned(), Value::from(count)))
        .collect();
    Value::Object(map)
}

/// The lines a plan inserts, in sequence order.
fn planned_lines(plan: &Plan, records: &[Rec]) -> Vec<Line> {
    let mut lines: Vec<Line> = plan
        .keep
        .iter()
        .map(|&i| {
            let r = &records[i];
            (
                signed(r.sequence),
                "line".to_owned(),
                r.level.as_str().to_owned(),
                Some(r.message.clone()),
                r.exception.clone(),
                r.fields.clone(),
                r.truncated,
                None,
            )
        })
        .collect();
    if let Some((sequence, reason)) = plan.marker {
        lines.push((
            signed(sequence),
            "ceiling_marker".to_owned(),
            "info".to_owned(),
            None,
            None,
            None,
            false,
            Some(reason.as_str().to_owned()),
        ));
    }
    lines
}

/// The plan of `plan_batch` for the carry of a case on `stream`.
fn expected(
    case: &Case,
    stream: &StreamCounters,
    in_order: bool,
    tenant: TenantAllowance,
    mark: Option<MarkOnlyReason>,
) -> Plan {
    let sized: Vec<SizedRecord> = case
        .records
        .iter()
        .map(|r| SizedRecord {
            sequence: r.sequence,
            text_bytes: r.size(),
            truncated: r.truncated,
        })
        .collect();
    plan_batch(
        stream,
        AttemptCeilings {
            records: case
                .records_policy
                .resolved(QuotaResource::StepLogAttemptRecords),
            bytes: case
                .bytes_policy
                .resolved(QuotaResource::StepLogAttemptBytes),
        },
        tenant,
        &sized,
        Some(CarryMark {
            highest_sequence: case.highest,
            in_order,
            mark_only: mark,
        }),
    )
}

/// Asserts what the function did against the plan of `plan_batch` on the same carry.
fn assert_as_planned(
    label: &str,
    answer: &Answer,
    after: &Stored,
    lines: &[Line],
    plan: &Plan,
    records: &[Rec],
    outcome: &str,
) {
    assert_eq!(answer.outcome, outcome, "{label}: the outcome");
    assert_eq!(
        usize::try_from(answer.kept).expect("kept"),
        plan.keep.len(),
        "{label}: the lines kept"
    );
    assert_eq!(
        u64::try_from(answer.dropped_by_worker).expect("drops"),
        plan.drops.by_worker(),
        "{label}: the drops by the worker"
    );
    assert_eq!(
        answer.dropped_by_server,
        server_drops(plan),
        "{label}: the drops by the server, by reason"
    );
    assert_eq!(
        answer.ceiling_reason.as_deref(),
        plan.after.ceiling.map(CeilingReason::as_str),
        "{label}: the ceiling reason returned"
    );
    assert_eq!(
        answer.not_stored_reason.as_deref(),
        plan.after.not_stored_reason.map(NotStoredReason::as_str),
        "{label}: the not-stored reason returned"
    );
    assert_eq!(after.counters, plan.after, "{label}: the stream's counters");
    assert!(after.final_mark, "{label}: the stream keeps its final mark");
    assert_eq!(
        lines,
        planned_lines(plan, records),
        "{label}: the lines and the marker stored"
    );
    assert_eq!(
        after.stored_by_this_transaction,
        !lines.is_empty(),
        "{label}: the order guard is raised exactly when a row is stored"
    );
    assert_eq!(
        answer.room.is_none(),
        answer.path == "none",
        "{label}: the room is null exactly when the path is none"
    );
    assert_eq!(
        answer.top_up,
        answer.room.is_some_and(|room| room < TOP_UP_MARK),
        "{label}: a top-up exactly when the room left is under 65,536 bytes"
    );
}

/// A connection of the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE` (else of the owner), kept for the test: its
/// server backend is the writer of its quota lease.
async fn writer(pool: &PgPool) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
    pool.acquire().await.expect("a writer's connection")
}

/// The domain's `plan_batch` is the rule, and the function implements its T5 and T6 for a carry (decision D041-100):
/// on 300 generated carries (new and existing streams, each ceiling met, the tenant allowance exceeded, `in_order`
/// false, duplicates, gaps, each planned mark-only reason, capture off) the function keeps the same sequences and
/// marker, leaves the same counters and counts the same drops by reason. Half the cases run on a writer whose quota
/// lease was topped up (the fast path when its room covers the carry), half on one with none (the block and its
/// refill); both paths, and the carries that charge nothing, are each taken at least twenty times.
#[tokio::test]
async fn the_carry_function_plans_every_carry_as_plan_batch_plans_it() {
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let pool = pool_of(&w, 2).await;
    let mut topped = writer(&pool).await;
    let mut bare = writer(&pool).await;
    let seed = std::env::var("SAGAT_CARRY_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0041_0036);
    let mut rng = Rng(seed);
    let mut current_limit = None;
    let mut paths: BTreeMap<String, u64> = BTreeMap::new();
    let mut outcomes: BTreeMap<String, u64> = BTreeMap::new();
    for number in 0..CASES {
        let case = generate(&mut rng);
        let label = format!("seed {seed:#x}, case {number}: {case:?}");
        write_policies(&w, &case).await;
        write_tenant_limit(&w, case.tenant_limit, &mut current_limit).await;
        let conn: &mut PgConnection = if case.topped_up {
            sqlx::query("SELECT outcome FROM sagat_quota_step_log_top_up($1)")
                .bind(tenant)
                .execute(&mut *topped)
                .await
                .expect("the writer's top-up");
            &mut topped
        } else {
            &mut bare
        };
        let id = Identity {
            tenant,
            namespace,
            run: Uuid::now_v7(),
            step: Uuid::now_v7(),
        };
        let mut tx = conn.begin().await.expect("the case's transaction");
        if let Some(existing) = case.existing {
            insert_existing(&mut tx, id, existing).await;
        }
        let tenant_allowance = allowance(&mut tx, tenant).await;
        let answer = carry(
            &mut tx,
            id,
            &case.records,
            case.highest,
            case.mark.map(|m| m.drop_reason().as_str()),
        )
        .await
        .unwrap_or_else(|e| panic!("{label}: the call failed: {e}"));
        let after = stored(&mut tx, id).await;
        *outcomes.entry(answer.outcome.clone()).or_default() += 1;
        *paths.entry(answer.path.clone()).or_default() += 1;
        let off = case
            .records_policy
            .resolved(QuotaResource::StepLogAttemptRecords)
            == 0;
        if off {
            assert_eq!(answer.outcome, "off", "{label}");
            assert_eq!(answer.path, "none", "{label}");
            let unchanged = case.existing.map(|e| e.counters);
            assert_eq!(
                after.map(|s| s.counters),
                unchanged,
                "{label}: capture off writes nothing"
            );
        } else {
            let after = after.unwrap_or_else(|| panic!("{label}: no stream"));
            let lines = lines_stored_now(&mut tx, tenant, after.stream_id).await;
            let (stream, in_order) = case
                .existing
                .map_or((StreamCounters::new(), true), |e| (e.counters, e.in_order));
            let plan = expected(&case, &stream, in_order, tenant_allowance, case.mark);
            let outcome = if case.mark.is_some() {
                "mark_only"
            } else {
                "written"
            };
            assert_as_planned(
                &label,
                &answer,
                &after,
                &lines,
                &plan,
                &case.records,
                outcome,
            );
            if case.existing.is_none() {
                let retention = case
                    .retention_policy
                    .resolved(QuotaResource::StepLogRetentionDays);
                assert_eq!(
                    u64::try_from(after.retention_days).expect("days"),
                    retention,
                    "{label}: a new stream takes the namespace's retention"
                );
            }
            let charge_planned = !plan.keep.is_empty() || plan.marker.is_some();
            match answer.path.as_str() {
                "fast" => assert!(case.topped_up, "{label}: only a topped-up writer has room"),
                "refill" => assert!(charge_planned, "{label}: a refill charges"),
                "none" => assert!(
                    case.mark.is_some() || case.records.is_empty() || !charge_planned,
                    "{label}: a carry that charges takes the fast path or the refill"
                ),
                other => panic!("{label}: path {other}"),
            }
            assert_eq!(answer.cause, None, "{label}: no answer of the block");
        }
        tx.rollback().await.expect("the case is rolled back");
    }
    for (path, at_least) in [("fast", 20), ("refill", 20), ("none", 20)] {
        assert!(
            paths.get(path).copied().unwrap_or(0) >= at_least,
            "the path {path} was taken at least {at_least} times: {paths:?}"
        );
    }
    for (outcome, at_least) in [("written", 100), ("mark_only", 40), ("off", 3)] {
        assert!(
            outcomes.get(outcome).copied().unwrap_or(0) >= at_least,
            "the outcome {outcome} came at least {at_least} times: {outcomes:?}"
        );
    }
}

/// Three records of 700 bytes and their charge, 3 × (700 + 128) = 2,484 bytes.
fn three_records() -> Vec<Rec> {
    (1..=3)
        .map(|sequence| Rec {
            sequence,
            level: StepLogLevel::Info,
            message: "x".repeat(700),
            exception: None,
            fields: None,
            truncated: false,
        })
        .collect()
}

/// The answers of the block (data-model.md §8a): the charge refused with the room held by an open transaction's quota
/// lease (`held`), and the fallback refill finding the usage row held and no wait slot free (`busy`), each answered in
/// the same call with the mark-only final mark `plan_batch` plans for `tenant_quota` and `service_busy`: on a new stream
/// and on an existing one, nothing charged, `path` none, no top-up. Removed and final streams are answered so and left
/// as they were.
#[tokio::test]
async fn the_blocks_answers_and_the_closed_streams_are_planned_as_plan_batch_plans_them() {
    let w = world().await;
    let tenant = w.admin.tenant_id().as_uuid();
    let namespace = w.namespace.id().as_uuid();
    let pool = pool_of(&w, 1).await;
    let mut conn = writer(&pool).await;
    let application: String = sqlx::query_scalar("SELECT current_setting('application_name')")
        .fetch_one(&mut *conn)
        .await
        .expect("the writer's application name");
    let records = three_records();
    let case = |existing: Option<Existing>| Case {
        existing,
        records_policy: Settings::default(),
        bytes_policy: Settings::default(),
        retention_policy: Settings::default(),
        tenant_limit: None,
        records: records.clone(),
        highest: 5,
        mark: None,
        topped_up: false,
    };
    let existing = Existing {
        counters: StreamCounters {
            // A stream a batch created and stored nothing in: every record of the carry is fresh.
            ..StreamCounters::new()
        },
        in_order: true,
    };
    sqlx::query(
        "INSERT INTO tenant_retained_usage (tenant_id) VALUES ($1) ON CONFLICT (tenant_id) DO NOTHING",
    )
    .bind(tenant)
    .execute(&w.db.pool)
    .await
    .expect("the tenant's usage row");

    // busy: the usage row held and the tenant's only wait slot (Q = 1, P = 1) of this writer's replica taken.
    let mut holder = w.db.pool.begin().await.expect("the holder");
    sqlx::query("SELECT 1 FROM tenant_retained_usage WHERE tenant_id = $1 FOR UPDATE")
        .bind(tenant)
        .execute(&mut *holder)
        .await
        .expect("hold the usage row");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('sagat:step_log_wait:t:' || $1::text || ':' || $2 || ':1', 0))")
        .bind(tenant)
        .bind(&application)
        .execute(&mut *holder)
        .await
        .expect("take the tenant's wait slot");
    for stream in [None, Some(existing)] {
        let id = Identity {
            tenant,
            namespace,
            run: Uuid::now_v7(),
            step: Uuid::now_v7(),
        };
        let mut tx = conn.begin().await.expect("a transaction");
        if let Some(existing) = stream {
            insert_existing(&mut tx, id, existing).await;
        }
        let allowance = allowance(&mut tx, tenant).await;
        // Q = 1 for this call: one tenant slot, taken.
        let answer = carry_with_slots(&mut tx, id, &records, 5, None, 1)
            .await
            .expect("the call");
        let after = stored(&mut tx, id).await.expect("the stream");
        let lines = lines_stored_now(&mut tx, tenant, after.stream_id).await;
        let c = case(stream);
        let (counters, in_order) =
            stream.map_or((StreamCounters::new(), true), |e| (e.counters, e.in_order));
        let plan = expected(
            &c,
            &counters,
            in_order,
            allowance,
            Some(MarkOnlyReason::ServiceBusy),
        );
        let label = format!(
            "busy, {}",
            if stream.is_some() { "existing" } else { "new" }
        );
        assert_as_planned(
            &label,
            &answer,
            &after,
            &lines,
            &plan,
            &records,
            "mark_only",
        );
        assert_eq!(answer.cause.as_deref(), Some("busy"), "{label}");
        assert_eq!(
            (answer.path.as_str(), answer.room, answer.top_up),
            ("none", None, false),
            "{label}"
        );
        assert_eq!(
            after.counters.not_stored_reason,
            Some(NotStoredReason::ServiceBusy),
            "{label}"
        );
        tx.rollback().await.expect("rolled back");
    }
    holder.rollback().await.expect("released");

    // held: a limit of 4,000 bytes; another writer's quota lease topped up to ⌊4,000 / 2⌋ = 2,000 and held by its open
    // transaction; this writer's refill folds nothing it can take back, finds 4,000 − 2,000 = 2,000 bytes free for a
    // charge of 2,484, and the other lease's unused 2,000 would cover it: `tenant_step_log_quota_held`. T6 read 4,000
    // bytes of room, so the plan kept the three records before the block answered.
    let mut limit = None;
    write_tenant_limit(&w, Some(4_000), &mut limit).await;
    let mut other = w.db.pool.acquire().await.expect("another writer");
    let (made, granted): (String, i64) =
        sqlx::query_as("SELECT outcome, room FROM sagat_quota_step_log_top_up($1)")
            .bind(tenant)
            .fetch_one(&mut *other)
            .await
            .expect("the other writer's top-up");
    assert_eq!((made.as_str(), granted), ("made", 2_000));
    let mut other_tx = other.begin().await.expect("the other writer's transaction");
    let (reserved, _): (bool, Option<i64>) =
        sqlx::query_as("SELECT reserved, room FROM sagat_quota_step_log_reserve($1, 1)")
            .bind(tenant)
            .fetch_one(&mut *other_tx)
            .await
            .expect("hold the other quota lease");
    assert!(reserved, "the other writer holds its quota lease");
    for stream in [None, Some(existing)] {
        let id = Identity {
            tenant,
            namespace,
            run: Uuid::now_v7(),
            step: Uuid::now_v7(),
        };
        let mut tx = conn.begin().await.expect("a transaction");
        if let Some(existing) = stream {
            insert_existing(&mut tx, id, existing).await;
        }
        let allowance = allowance(&mut tx, tenant).await;
        assert_eq!(
            allowance.remaining,
            Some(4_000),
            "T6 reads room for the charge"
        );
        let answer = carry(&mut tx, id, &records, 5, None)
            .await
            .expect("the call");
        let after = stored(&mut tx, id).await.expect("the stream");
        let lines = lines_stored_now(&mut tx, tenant, after.stream_id).await;
        let c = case(stream);
        let (counters, in_order) =
            stream.map_or((StreamCounters::new(), true), |e| (e.counters, e.in_order));
        let plan = expected(
            &c,
            &counters,
            in_order,
            allowance,
            Some(MarkOnlyReason::TenantQuota),
        );
        let label = format!(
            "held, {}",
            if stream.is_some() { "existing" } else { "new" }
        );
        assert_as_planned(
            &label,
            &answer,
            &after,
            &lines,
            &plan,
            &records,
            "mark_only",
        );
        assert_eq!(answer.cause.as_deref(), Some("held"), "{label}");
        assert_eq!(
            (answer.path.as_str(), answer.room, answer.top_up),
            ("none", None, false),
            "{label}"
        );
        assert_eq!(
            after.counters.ceiling,
            Some(CeilingReason::TenantQuota),
            "{label}"
        );
        tx.rollback().await.expect("rolled back");
    }
    other_tx.rollback().await.expect("released");
    write_tenant_limit(&w, None, &mut limit).await;

    // Removed and final streams: answered so, nothing written.
    for (state, outcome) in [
        ("removed_at = clock_timestamp()", "removed"),
        (
            "expires_at = clock_timestamp() - interval '1 second'",
            "removed",
        ),
        ("final_at = clock_timestamp()", "final_recorded"),
    ] {
        let id = Identity {
            tenant,
            namespace,
            run: Uuid::now_v7(),
            step: Uuid::now_v7(),
        };
        let mut tx = conn.begin().await.expect("a transaction");
        insert_existing(&mut tx, id, existing).await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE step_log_streams SET {state} WHERE tenant_id = $1 AND run_id = $2"
        )))
        .bind(tenant)
        .bind(id.run)
        .execute(&mut *tx)
        .await
        .expect("close the stream");
        let before = stored(&mut tx, id).await.expect("the stream");
        let answer = carry(&mut tx, id, &records, 5, None)
            .await
            .expect("the call");
        assert_eq!(answer.outcome, outcome, "{state}");
        assert_eq!(
            (answer.kept, answer.path.as_str(), answer.room),
            (0, "none", None),
            "{state}"
        );
        assert_eq!(
            stored(&mut tx, id).await,
            Some(before),
            "{state}: nothing written"
        );
        tx.rollback().await.expect("rolled back");
    }
}

/// Decision D041-146 (3): the carry builds its keys in SQL (security review Nit S-4), the ingest in Rust; the two
/// builders return exactly the same keys for the same inputs, the edges of each part included, so that a carry and an
/// ingest of one attempt, and a change of a step-log resource, always take the same fences and the same stream lock.
#[tokio::test]
async fn the_carrys_sql_builds_exactly_the_fence_and_stream_keys_the_ingest_builds() {
    let w = world().await;
    let pool = pool_of(&w, 1).await;
    let mut rng = Rng(0x0041_0146);
    let mut inputs = vec![
        (
            Uuid::nil(),
            Uuid::nil(),
            Uuid::nil(),
            Uuid::nil(),
            1_u32,
            0_u64,
        ),
        (
            Uuid::max(),
            Uuid::max(),
            Uuid::max(),
            Uuid::max(),
            i32::MAX as u32,
            i64::MAX as u64,
        ),
    ];
    for _ in 0..62 {
        inputs.push((
            Uuid::from_u128(u128::from(rng.next()) << 64 | u128::from(rng.next())),
            Uuid::from_u128(u128::from(rng.next()) << 64 | u128::from(rng.next())),
            Uuid::now_v7(),
            Uuid::from_u128(u128::from(rng.next()) << 64 | u128::from(rng.next())),
            1 + u32::try_from(rng.below(2_147_483_646)).expect("an attempt"),
            rng.below(u64::try_from(i64::MAX).expect("a bound")),
        ));
    }
    for (tenant, namespace, run, step, attempt, generation) in inputs {
        let (fences, stream): (Vec<String>, String) = sqlx::query_as(
            "SELECT sagat_step_log_policy_fence_keys($1, $2), sagat_step_log_stream_lock_key($1, $3, $4, $5, $6)",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(run)
        .bind(step)
        .bind(i32::try_from(attempt).expect("an attempt"))
        .bind(signed(generation))
        .fetch_one(&pool)
        .await
        .expect("the SQL keys");
        assert_eq!(
            fences,
            step_log_policy_fence_keys(TenantId::new(tenant), NamespaceId::new(namespace)),
            "the policy fences of {tenant} and {namespace}"
        );
        assert_eq!(
            stream,
            stream_lock_key(
                TenantId::new(tenant),
                RunId::new(run),
                step,
                attempt,
                generation
            ),
            "the stream lock of {tenant}, {run}, {step}, {attempt}, {generation}"
        );
    }
}

/// Item 0 of data-model.md §8a: a mark-only reason outside the four the application plans (`invalid`, `secret`,
/// `content_fallback`, `failed`) is refused at the top, with a fixed text and before any lock; so are the arrays of a
/// full carry that disagree (lengths, order, a size that is not its text's). Nothing is written.
#[tokio::test]
async fn the_carry_function_refuses_a_reason_or_records_it_was_not_given() {
    let w = world().await;
    let pool = pool_of(&w, 1).await;
    let mut conn = writer(&pool).await;
    let id = Identity {
        tenant: w.admin.tenant_id().as_uuid(),
        namespace: w.namespace.id().as_uuid(),
        run: Uuid::now_v7(),
        step: Uuid::now_v7(),
    };
    let records = three_records();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("the writer's backend");
    for reason in ["tenant_quota", "service_busy", "worker_gap", "", "secret "] {
        let mut tx = conn.begin().await.expect("a transaction");
        let error = carry(&mut tx, id, &records, 5, Some(reason))
            .await
            .expect_err("refused");
        let database = error.as_database_error().expect("a database error");
        assert_eq!(database.code().as_deref(), Some("22023"), "{reason:?}");
        assert_eq!(
            database.message(),
            "a step-log carry takes only the mark-only reasons invalid, secret, content_fallback and failed"
        );
        // An advisory lock taken before the raise would be held until the transaction ends.
        let locks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND pid = $1",
        )
        .bind(pid)
        .fetch_one(&w.db.pool)
        .await
        .expect("the writer's locks");
        assert_eq!(locks, 0, "{reason:?}: refused before any lock");
        tx.rollback().await.expect("rolled back");
    }
    for reason in MarkOnlyReason::PLANNED {
        let mut tx = conn.begin().await.expect("a transaction");
        let answer = carry(
            &mut tx,
            id,
            &records,
            5,
            Some(reason.drop_reason().as_str()),
        )
        .await
        .expect("a planned reason");
        assert_eq!(answer.outcome, "mark_only");
        tx.rollback().await.expect("rolled back");
    }
    let mut shuffled = records.clone();
    shuffled.swap(0, 1);
    for (label, records, highest) in [
        ("out of order", shuffled, 5),
        ("highest below the last record", records.clone(), 2),
    ] {
        let mut tx = conn.begin().await.expect("a transaction");
        let error = carry(&mut tx, id, &records, highest, None)
            .await
            .expect_err(label);
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("22023"),
            "{label}"
        );
        tx.rollback().await.expect("rolled back");
    }
    // A size that is not its text's: the caller's measure and the trigger's would disagree.
    let mut tx = conn.begin().await.expect("a transaction");
    let error = sqlx::query(
        "SELECT * FROM sagat_step_log_carry($1, $2, $3, $4, 'act', '{}', '{}', 1, 0, '{1}', '{info}', '{NULL}', \
                                            '{abc}', '{NULL}', '{NULL}', '{false}', '{4}', 1, 5, NULL)",
    )
    .bind(id.tenant)
    .bind(id.namespace)
    .bind(id.run)
    .bind(id.step)
    .fetch_one(&mut *tx)
    .await
    .expect_err("a size that is not the text's");
    assert_eq!(
        error
            .as_database_error()
            .map(|e| e.message().to_owned())
            .as_deref(),
        Some("a size of a step-log carry is not the size of its text")
    );
    tx.rollback().await.expect("rolled back");
    let mut tx = conn.begin().await.expect("a transaction");
    assert_eq!(stored(&mut tx, id).await, None, "nothing written");
    tx.rollback().await.expect("rolled back");
}

/// FR-041-008: a carry never takes the usage row itself (its charge takes a quota lease, and the usage row only to
/// refill); it reads the allowance only through `sagat_quota_step_log_allowance`. Its migration never names the table.
#[test]
fn the_carrys_migration_never_names_the_usage_row() {
    assert!(
        MIGRATION.contains("CREATE FUNCTION sagat_step_log_carry("),
        "the migration that creates the carry"
    );
    assert!(
        !MIGRATION.contains("tenant_retained_usage"),
        "the carry's functions never name the usage row"
    );
    assert!(
        MIGRATION.contains("sagat_quota_step_log_allowance(p_tenant)"),
        "the allowance is read through its accessor"
    );
}
