//! What a step-log insert charges and what its delete releases (ADR-0027's Amendment of the Amendment, OD-2026-11-20-16),
//! the budget line of a charged insert, and the classes kept apart: the tenant's other diagnostic text is charged to
//! `tenant_log_bytes`, whatever its step-log quota holds.

use std::collections::BTreeMap;

use sqlx::{Connection as _, PgConnection, PgPool};
use uuid::Uuid;

use super::{Fx, LIMIT, ROW, exceeded, refusal};
use crate::support::bounded;

/// One line of the test: its message, exception and fields, each possibly null.
#[derive(Debug, Clone)]
struct Line {
    message: Option<String>,
    exception: Option<String>,
    fields: Option<String>,
}

impl Line {
    /// Its charge: the UTF-8 octets of its three texts plus 128 (`fixed_octets` on `level`).
    fn charge(&self) -> i64 {
        [&self.message, &self.exception, &self.fields]
            .iter()
            .map(|text| {
                text.as_ref()
                    .map_or(0, |t| i64::try_from(t.len()).expect("small"))
            })
            .sum::<i64>()
            + ROW
    }
}

async fn insert_lines(
    conn: &mut PgConnection,
    tenant: Uuid,
    stream: i64,
    first: i64,
    lines: &[Line],
) -> Result<(), sqlx::Error> {
    let sequences: Vec<i64> = (0..lines.len())
        .map(|i| first + i64::try_from(i).expect("few"))
        .collect();
    let messages: Vec<Option<String>> = lines.iter().map(|l| l.message.clone()).collect();
    let exceptions: Vec<Option<String>> = lines.iter().map(|l| l.exception.clone()).collect();
    let fields: Vec<Option<String>> = lines.iter().map(|l| l.fields.clone()).collect();
    sqlx::query(
        "INSERT INTO step_log_lines (tenant_id, stream_id, sequence, run_id, level, kind, message, exception, fields) \
         SELECT $1, $2, t.s, gen_random_uuid(), 'warn', 'line', t.m, t.e, t.f \
           FROM unnest($3::bigint[], $4::text[], $5::text[], $6::text[]) AS t(s, m, e, f)",
    )
    .bind(tenant)
    .bind(stream)
    .bind(sequences)
    .bind(messages)
    .bind(exceptions)
    .bind(fields)
    .execute(conn)
    .await
    .map(|_| ())
}

async fn deltas(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM quota_transaction_deltas")
        .fetch_one(pool)
        .await
        .expect("the delta rows")
}

/// An insert of 10 lines adds one quota-lease charge of their text plus 1,280 bytes (10 × 128) and queues no flush (no
/// delta row, inside the transaction or after it); a `DELETE` of them releases the same through the finalizer.
#[tokio::test]
async fn ten_lines_charge_their_text_plus_1280_bytes_queue_no_flush_and_their_delete_releases_the_same()
 {
    bounded::within("ten lines", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_000)
            .await
            .expect("a quota lease with room");
        let lines: Vec<Line> = (0..10)
            .map(|i| Line {
                message: Some(format!("line {i}: é")),
                exception: (i % 3 == 0).then(|| "boom".to_owned()),
                fields: (i % 2 == 0).then(|| format!("{{\"i\":{i}}}")),
            })
            .collect();
        let text: i64 = lines.iter().map(|l| l.charge() - ROW).sum();
        let before = fx.ledger(fx.tenant).await;
        let first = fx.sequences(10);
        let mut tx = w.conn.begin().await.expect("begin");
        insert_lines(&mut tx, fx.tenant, fx.stream, first, &lines)
            .await
            .expect("ten lines");
        tx.commit().await.expect("commit");
        let after = fx.ledger(fx.tenant).await;
        assert_eq!(after.usage() - before.usage(), text + 10 * ROW);
        assert_eq!(
            (after.s, after.g, after.leases),
            (before.s, before.g, before.leases),
            "one quota-lease charge only"
        );
        assert_eq!(after.used - before.used, text + 1_280);
        assert_eq!(deltas(fx.pool()).await, 0, "no delta row after the commit");
        sqlx::query(
            "DELETE FROM step_log_lines WHERE tenant_id = $1 AND sequence >= $2 AND sequence < $3",
        )
        .bind(fx.tenant)
        .bind(first)
        .bind(first + 10)
        .execute(fx.pool())
        .await
        .expect("the purge's delete");
        assert_eq!(
            fx.ledger(fx.tenant).await.usage(),
            before.usage(),
            "the delete released exactly the charge"
        );
        assert_eq!(
            deltas(fx.pool()).await,
            0,
            "the finalizer consumed its delta"
        );
    })
    .await;
}

/// The charge of any inserted lines equals the release of their delete and the measure of their texts plus 128 a row:
/// Unicode text, canonical JSON fields and text whose NUL bytes were replaced (U+FFFD), any of them null. A seeded
/// generator; the case is in the failure.
#[tokio::test]
async fn the_charge_of_any_inserted_lines_equals_the_release_of_their_delete() {
    bounded::within("charge equals release", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut w = fx.writer().await;
        let alphabet = ['a', 'é', '日', '😀', '\u{FFFD}', '"', '\\', ' ', '\n', 'z'];
        let mut rng = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move |n: u64| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % n
        };
        for case in 0..24 {
            let count = 1 + usize::try_from(next(12)).expect("small");
            let mut text = |max: u64| -> Option<String> {
                if next(4) == 0 {
                    return None;
                }
                let n = next(max);
                Some(
                    (0..n)
                        .map(|_| alphabet[usize::try_from(next(10)).expect("small")])
                        .collect(),
                )
            };
            let lines: Vec<Line> = (0..count)
                .map(|_| Line {
                    message: text(400),
                    exception: text(200),
                    fields: text(60).map(|value| {
                        serde_json::to_string(&serde_json::json!({ "k": value }))
                            .expect("canonical JSON")
                    }),
                })
                .collect();
            let expected: i64 = lines.iter().map(Line::charge).sum();
            let before = fx.ledger(fx.tenant).await.usage();
            let first = fx.sequences(i64::try_from(count).expect("few"));
            insert_lines(&mut w.conn, fx.tenant, fx.stream, first, &lines)
                .await
                .expect("the lines");
            let charged = fx.ledger(fx.tenant).await.usage() - before;
            assert_eq!(
                charged, expected,
                "case {case}: the charge is the measure: {lines:?}"
            );
            sqlx::query("DELETE FROM step_log_lines WHERE tenant_id = $1 AND sequence >= $2")
                .bind(fx.tenant)
                .bind(first)
                .execute(fx.pool())
                .await
                .expect("the delete");
            assert_eq!(
                fx.ledger(fx.tenant).await.usage(),
                before,
                "case {case}: the release equals the charge"
            );
        }
        fx.assert_exact(fx.tenant, 1 << 30).await;
    })
    .await;
}

/// `pg_stat_statements` sums of the nested statements of this database, by text.
async fn nested_by_query(pool: &PgPool) -> BTreeMap<String, i64> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT regexp_replace(query, '\\s+', ' ', 'g'), sum(calls)::bigint FROM pg_stat_statements \
          WHERE NOT toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) GROUP BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("pg_stat_statements")
    .into_iter()
    .collect()
}

async fn evictions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT dealloc FROM pg_stat_statements_info")
        .fetch_one(pool)
        .await
        .expect("pg_stat_statements_info")
}

/// The committed quota rows of the tenant: its quota leases (writer, granted, used) and its usage row (S, G).
async fn quota_rows(pool: &PgPool, tenant: Uuid) -> (Vec<(i32, i64, i64)>, (i64, i64)) {
    let leases = sqlx::query_as("SELECT writer, granted, used FROM step_log_quota_leases WHERE tenant_id = $1 ORDER BY writer")
        .bind(tenant)
        .fetch_all(pool)
        .await
        .expect("the quota leases");
    let usage = sqlx::query_as("SELECT step_log_bytes, step_log_leased_bytes FROM tenant_retained_usage WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_one(pool)
        .await
        .expect("the usage row");
    (leases, usage)
}

/// The budget line of a step-log insert under ADR-0097 (data-model.md §3b): charged into a quota lease with room, an
/// insert of ten lines writes one quota-lease row and no usage or delta row, runs six nested statements for its charge
/// (the transition-table read, the charge call, the three fences in one statement, the limit's read, the unlocked read
/// of S, G and the enforcement flag in one statement, the quota-lease update) besides one foreign-key check a line, and
/// its commit runs no deferred call. The first insert of the writer refills: it writes the usage row once and one
/// quota-lease row. Measured again while `pg_stat_statements` evicts entries.
#[tokio::test]
async fn a_step_log_insert_costs_its_budget_line() {
    bounded::within("budget line", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let pool = fx.pool();
        sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
            .execute(pool)
            .await
            .expect("the test PostgreSQL preloads pg_stat_statements");
        let track: String =
            sqlx::query_scalar("SELECT current_setting('pg_stat_statements.track')")
                .fetch_one(pool)
                .await
                .expect("track");
        assert_eq!(track, "all", "the budget counts nested statements");
        let mut w = fx.writer().await;
        let start = quota_rows(pool, fx.tenant).await;
        let first = fx.sequences(10);
        super::insert_messages(&mut w.conn, fx.tenant, fx.stream, first, &[200; 10])
            .await
            .expect("the first ten lines");
        let refilled = quota_rows(pool, fx.tenant).await;
        assert_eq!(
            refilled.0.len(),
            start.0.len() + 1,
            "a refill inserts the writer's quota lease"
        );
        assert_ne!(refilled.1, start.1, "a refill writes the usage row");
        for _ in 0..8 {
            let evicted = evictions(pool).await;
            let before_rows = quota_rows(pool, fx.tenant).await;
            let items = nested_by_query(pool).await;
            let deltas_before = deltas(pool).await;
            let mut tx = w.conn.begin().await.expect("begin");
            let first = fx.sequences(10);
            super::insert_messages(&mut tx, fx.tenant, fx.stream, first, &[200; 10])
                .await
                .expect("ten more lines");
            let during = nested_by_query(pool).await;
            tx.commit().await.expect("commit");
            let after = nested_by_query(pool).await;
            if evictions(pool).await != evicted {
                continue;
            }
            let ran: Vec<(String, i64)> = during
                .iter()
                .map(|(q, n)| {
                    (
                        q.chars().take(100).collect::<String>(),
                        n - items.get(q).copied().unwrap_or(0),
                    )
                })
                .filter(|(_, n)| *n > 0)
                .collect();
            let foreign_key: i64 = ran
                .iter()
                .filter(|(q, _)| {
                    q.starts_with("SELECT $3 FROM ONLY \"public\".\"step_log_streams\"")
                })
                .map(|(_, n)| n)
                .sum();
            assert_eq!(foreign_key, 10, "one foreign-key check a line: {ran:#?}");
            let nested: i64 = ran.iter().map(|(_, n)| n).sum();
            assert_eq!(
                nested,
                6 + 10,
                "the nested statements of one charged insert: {ran:#?}"
            );
            let committed: i64 = after.values().sum::<i64>() - during.values().sum::<i64>();
            assert_eq!(committed, 0, "the commit runs no deferred quota call");
            let after_rows = quota_rows(pool, fx.tenant).await;
            let changed = after_rows
                .0
                .iter()
                .filter(|row| !before_rows.0.contains(row))
                .count();
            assert_eq!(changed, 1, "one quota-lease row written");
            assert_eq!(after_rows.1, before_rows.1, "the usage row untouched");
            assert_eq!(deltas(pool).await, deltas_before, "no delta row");
            return;
        }
        panic!("pg_stat_statements evicted entries during every one of eight attempts");
    })
    .await;
}

/// A positive step-log delta would be a charge outside a quota lease: the finalizer refuses it.
#[tokio::test]
async fn the_finalizer_refuses_a_step_log_charge_outside_a_quota_lease() {
    bounded::within("unleased charge", LIMIT, async {
        let fx = Fx::new(1 << 30).await;
        let mut tx = fx.pool().begin().await.expect("begin");
        sqlx::query("SELECT sagat_quota_add_delta($1, 0, 0, 0, 128)")
            .bind(fx.tenant)
            .execute(&mut *tx)
            .await
            .expect("a delta");
        sqlx::query("SET CONSTRAINTS ALL IMMEDIATE").execute(&mut *tx).await.expect("immediate");
        // The finalizer runs on a classified write; a namespace's slug is one (content), which flushes every delta row of
        // the transaction.
        let refused = sqlx::query(
            "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES (gen_random_uuid(), $1, 'wsp', now())",
        )
        .bind(fx.tenant)
        .execute(&mut *tx)
        .await
        .map(|_| ());
        assert_eq!(refusal(&refused), Some(("22000".into(), Some("step_log_delta_unleased".into()))));
    })
    .await;
}

/// The tenant's other diagnostic text is charged to `tenant_log_bytes` and stored in full while its step-log quota is
/// full: a script's log, an event's dispatch error and a connector's error (ADR-0027 Amendment item 1).
#[tokio::test]
async fn a_script_log_a_dispatch_error_and_a_connector_error_are_stored_in_full_at_the_step_log_limit()
 {
    bounded::within("classes apart", LIMIT, async {
        let fx = Fx::new(1_024).await;
        let mut w = fx.writer().await;
        fx.charge_committed(&mut w.conn, 1_024).await.expect("the step-log quota full");
        assert_eq!(refusal(&fx.charge_committed(&mut w.conn, ROW).await), exceeded());
        let pool = fx.pool();
        let tenant = fx.tenant;
        let workspace: Uuid = sqlx::query_scalar(
            "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES (gen_random_uuid(), $1, 'wsp', now()) RETURNING id",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await
        .expect("a workspace");
        let namespace: Uuid = sqlx::query_scalar(
            "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
             VALUES (gen_random_uuid(), $1, $2, 'dev', 'dev', now()) RETURNING id",
        )
        .bind(tenant)
        .bind(workspace)
        .fetch_one(pool)
        .await
        .expect("a namespace");
        let logs = || async {
            sqlx::query_scalar::<_, i64>("SELECT log_bytes FROM tenant_retained_usage WHERE tenant_id = $1")
                .bind(tenant)
                .fetch_one(pool)
                .await
                .expect("the log usage")
        };
        let before = logs().await;
        let error = "x".repeat(5_000);
        let connector = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO event_connectors (id, tenant_id, namespace_id, slug, kind, settings, rate_per_second, created_by) \
             VALUES ($1, $2, $3, 'quota-log', 'NATS', '{}'::jsonb, 1, 'test')",
        )
        .bind(connector)
        .bind(tenant)
        .bind(namespace)
        .execute(pool)
        .await
        .expect("a connector");
        sqlx::query("UPDATE event_connectors SET last_error = $2 WHERE id = $1")
            .bind(connector)
            .bind(&error)
            .execute(pool)
            .await
            .expect("a connector error in full");
        sqlx::query(
            "INSERT INTO inbox_events (tenant_id, source, event_id, namespace_id, event_type, payload, payload_digest, \
               received_at, expires_at, status, attempts, next_attempt_at, last_error) \
             VALUES ($1, 'src-a', 'e1', $2, 't', '\\x00', decode(repeat('00', 32), 'hex'), now(), now() + interval '1 day', \
               'PENDING', 1, now(), $3)",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(&error)
        .execute(pool)
        .await
        .expect("a dispatch error in full");
        let digest = format!("sha256:{}", "a".repeat(64));
        sqlx::query(
            "INSERT INTO script_artifacts (tenant_id, digest, language, content, signature, key_id, sbom, toolchain) \
             VALUES ($1, $2, 'python', '\\x01', decode(repeat('00', 64), 'hex'), 'k1', '\\x', 'py')",
        )
        .bind(tenant)
        .bind(&digest)
        .execute(pool)
        .await
        .expect("a script artifact");
        sqlx::query(
            "INSERT INTO script_executions (id, tenant_id, namespace_id, digest, status, log, wall_ms, start_ms, \
               created_at, expires_at) \
             VALUES (gen_random_uuid(), $1, $2, $3, 'SUCCEEDED', $4, 1, 1, now(), now() + interval '1 day')",
        )
        .bind(tenant)
        .bind(namespace)
        .bind(&digest)
        .bind(&error)
        .execute(pool)
        .await
        .expect("a script log in full");
        assert_eq!(logs().await - before, 3 * 5_000, "the three texts, in full, under tenant_log_bytes");
        let stored: i64 = sqlx::query_scalar(
            "SELECT ((SELECT octet_length(last_error) FROM event_connectors WHERE id = $2) \
                  + (SELECT octet_length(last_error) FROM inbox_events WHERE tenant_id = $1) \
                  + (SELECT octet_length(log) FROM script_executions WHERE tenant_id = $1))::bigint",
        )
        .bind(tenant)
        .bind(connector)
        .fetch_one(pool)
        .await
        .expect("the stored texts");
        assert_eq!(stored, 15_000);
        assert_eq!(fx.ledger(tenant).await.usage(), 1_024, "the step-log usage is untouched");
    })
    .await;
}
