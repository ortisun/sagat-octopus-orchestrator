//! Design 041 section 2, task T041-6: the ingest of `AppendStepLogs`, called directly (`StepLogIngest`; the RPC is task
//! T041-7's) against a real PostgreSQL, the rows asserted by SQL (the read routes come with task T041-11). For a stream,
//! lines stored + dropped by the worker + dropped by the server = `last_sequence` (design section 2).

use std::time::Duration;

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::step_logs::{StepLogIngest, StepLogVerdict};
use sagat_application::worker::WorkerStore as _;
use sagat_domain::identity::Permission;
use sagat_domain::quota::QuotaResource;
use sagat_domain::step_logs::StopReason;
use sagat_infra_postgres::PgStepLogStore;

use crate::support::bounded;
use crate::support::engine::world;
use crate::support::step_logs::{
    Hook, batch, hooked, ingest, leased, lines_of, record, records, send, set_namespace_policy,
    set_tenant_limit, step_log_usage, stream_of,
};

const LIMIT: Duration = Duration::from_secs(120);
/// The per-row charge of the tenant quota (OD-2026-11-15-6).
const ROW: i64 = 128;
/// The first sequence of the server's markers, 2^62.
const MARKER: i64 = 1 << 62;

fn continued(through: u64) -> sagat_application::step_logs::BatchAnswer {
    sagat_application::step_logs::BatchAnswer {
        verdict: StepLogVerdict::Continue,
        stored_through_sequence: through,
    }
}

fn final_only(through: u64) -> sagat_application::step_logs::BatchAnswer {
    sagat_application::step_logs::BatchAnswer {
        verdict: StepLogVerdict::FinalOnly,
        stored_through_sequence: through,
    }
}

fn stopped_attempt(reason: StopReason) -> sagat_application::step_logs::BatchAnswer {
    sagat_application::step_logs::BatchAnswer {
        verdict: StepLogVerdict::StopAttempt(reason),
        stored_through_sequence: 0,
    }
}

/// A batch is stored once: its lines, its counters, and its charge of their text plus 128 bytes a row. Sent again,
/// it stores nothing more and is answered `CONTINUE` with the same mark (FR-041-004).
#[tokio::test]
async fn a_batch_is_stored_once_and_its_resend_stores_nothing_more() {
    bounded::within("stored once", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records(1..=3)).await, continued(3));
        assert_eq!(send(&ingest, &a, records(1..=3)).await, continued(3));
        let lines = lines_of(&w, &a).await;
        assert_eq!(
            lines,
            (1..=3)
                .map(|n| (n, "line".to_owned(), Some(format!("line {n}"))))
                .collect::<Vec<_>>()
        );
        let stream = stream_of(&w, &a).await.expect("a stream");
        // "line 1" .. "line 3": 6 bytes each.
        assert_eq!(
            (stream.records, stream.bytes, stream.last_sequence),
            (3, 18, 3)
        );
        assert_eq!(step_log_usage(&w).await, 18 + 3 * ROW);
    })
    .await;
}

/// A gap is counted as dropped by the worker; a late batch (sequences at or below the mark) stores nothing and is not
/// counted again: 4 lines stored + 2 dropped by the worker = the mark, 6.
#[tokio::test]
async fn a_gap_is_dropped_by_the_worker_and_a_late_batch_after_a_newer_one_stores_nothing() {
    bounded::within("gaps", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records([1, 2])).await, continued(2));
        assert_eq!(send(&ingest, &a, records([5, 6])).await, continued(6));
        assert_eq!(send(&ingest, &a, records([3, 4])).await, continued(6));
        let stream = stream_of(&w, &a).await.expect("a stream");
        assert_eq!(
            (
                stream.records,
                stream.dropped_by_worker,
                stream.dropped_by_server,
                stream.last_sequence
            ),
            (4, 2, 0, 6)
        );
        assert_eq!(
            lines_of(&w, &a)
                .await
                .iter()
                .map(|(n, ..)| *n)
                .collect::<Vec<_>>(),
            [1, 2, 5, 6]
        );
    })
    .await;
}

/// The attempt's record ceiling: the records that fit are kept in sequence order, the rest dropped by the server, one
/// ceiling marker at 2^62, and `FINAL_ONLY` from then on; a later batch adds only drops. 3 stored + 4 dropped by the
/// server = 7.
#[tokio::test]
async fn the_attempt_ceiling_answers_final_only_with_one_marker() {
    bounded::within("ceiling", LIMIT, async {
        let w = world().await;
        set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 3).await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records(1..=5)).await, final_only(5));
        assert_eq!(send(&ingest, &a, records([6, 7])).await, final_only(7));
        let stream = stream_of(&w, &a).await.expect("a stream");
        assert_eq!(
            (
                stream.records,
                stream.dropped_by_worker,
                stream.dropped_by_server,
                stream.last_sequence,
                stream.ceiling_sequence,
                stream.ceiling_reason.as_deref()
            ),
            (3, 0, 4, 7, Some(5), Some("attempt_records"))
        );
        let lines = lines_of(&w, &a).await;
        assert_eq!(
            lines
                .iter()
                .map(|(n, kind, _)| (*n, kind.as_str()))
                .collect::<Vec<_>>(),
            [
                (1, "line"),
                (2, "line"),
                (3, "line"),
                (MARKER, "ceiling_marker")
            ]
        );
        // The marker is charged 128 bytes like any row.
        assert_eq!(step_log_usage(&w).await, 18 + 4 * ROW);
    })
    .await;
}

/// The tenant's limit read without a lock at T6: when what is kept, the planned marker's 128 bytes included, does not
/// fit, nothing is stored, every record is counted `tenant_quota` and the answer is `FINAL_ONLY`. Here one line of 6
/// bytes fits the room left (6 + 128 + 127 bytes) and its ceiling marker does not.
#[tokio::test]
async fn the_tenant_limit_read_at_t6_counts_tenant_quota_with_the_planned_marker_included() {
    bounded::within("tenant limit", LIMIT, async {
        let w = world().await;
        set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 1).await;
        set_tenant_limit(&w, 6 + ROW + 127).await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        assert_eq!(send(&ingest, &a, records([1, 2])).await, final_only(2));
        let stream = stream_of(&w, &a).await.expect("a stream");
        assert_eq!(
            (
                stream.records,
                stream.dropped_by_server,
                stream.last_sequence,
                stream.ceiling_reason.as_deref()
            ),
            (0, 2, 2, Some("tenant_quota"))
        );
        assert!(lines_of(&w, &a).await.is_empty());
        assert_eq!(step_log_usage(&w).await, 0);
    })
    .await;
}

/// Capture turned off in the namespace (`step_log_attempt_records` = 0): `STOP_CAPTURE` (`CAPTURE_OFF`), no row written.
/// The deployment switch off answers the same before any statement.
#[tokio::test]
async fn capture_off_stores_nothing_and_stops_capture() {
    bounded::within("capture off", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let off = StepLogIngest::new(
            PgStepLogStore::new(w.execution_pool.clone()),
            SystemClock,
            false,
        );
        let stopped = sagat_application::step_logs::BatchAnswer {
            verdict: StepLogVerdict::StopCapture(StopReason::CaptureOff),
            stored_through_sequence: 0,
        };
        assert_eq!(send(&off, &a, records([1])).await, stopped);
        set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 0).await;
        assert_eq!(send(&ingest(&w), &a, records([1])).await, stopped);
        assert_eq!(stream_of(&w, &a).await, None, "no stream row is written");
    })
    .await;
}

/// P4: a batch outside its bounds is refused, naming the bound, and stores nothing.
#[tokio::test]
async fn a_malformed_batch_is_refused_and_stores_nothing() {
    bounded::within("malformed", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        let mut unknown = record(1, "x");
        unknown.level = None;
        let mut too_large = batch(&a, records([1]));
        too_large.encoded_len = 192 * 1024 + 1;
        for (bad, named) in [
            (batch(&a, Vec::new()), "a step-log batch holds no record"),
            (
                batch(&a, records([2, 1])),
                "step-log sequences are not strictly increasing",
            ),
            (
                batch(&a, records([1, 1])),
                "step-log sequences are not strictly increasing",
            ),
            (
                batch(&a, records([0])),
                "a step-log sequence is outside 1 to 2^62-1",
            ),
            (
                batch(&a, records([1 << 62])),
                "a step-log sequence is outside 1 to 2^62-1",
            ),
            (
                batch(&a, vec![unknown]),
                "a step-log record has an unknown level",
            ),
            (
                batch(&a, records(1..=501)),
                "a step-log batch holds more than 500 records",
            ),
            (too_large, "a step-log batch is larger than 192 KiB"),
        ] {
            let refused = ingest.append(&a.session.principal, &bad).await;
            assert!(
                matches!(&refused, Err(AppError::NotAllowed(message)) if *message == named),
                "{bad:?}: {refused:?}"
            );
        }
        assert_eq!(stream_of(&w, &a).await, None);
    })
    .await;
}

/// P1: a runner token and a database-runner token are refused with the application's `Forbidden`, whatever their
/// role holds.
#[tokio::test]
async fn a_runner_or_database_runner_token_is_refused_by_p1() {
    bounded::within("p1", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        let database_runner = sagat_domain::identity::Principal::new(
            w.admin.tenant_id(),
            crate::support::subject("db-runner"),
            sagat_domain::identity::Role::DatabaseRunner,
        );
        for principal in [&w.runner_identity, &database_runner] {
            let refused = ingest.append(principal, &batch(&a, records([1]))).await;
            assert!(
                matches!(
                    refused,
                    Err(AppError::Forbidden {
                        permission: Permission::WorkerExecute
                    })
                ),
                "{refused:?}"
            );
        }
        assert_eq!(stream_of(&w, &a).await, None);
    })
    .await;
}

async fn last_stored_xid(
    w: &crate::support::engine::World,
    a: &crate::support::step_logs::Attempt,
) -> String {
    sqlx::query_scalar(
        "SELECT last_stored_xid::text FROM step_log_streams WHERE tenant_id = $1 AND run_id = $2 AND step_id = $3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(a.run.as_uuid())
    .bind(a.task.step_id)
    .fetch_one(&w.db.pool)
    .await
    .expect("the order guard")
}

/// T9 raises `last_stored_xid` only when T8 inserted a row (the order guard of section 2a), and the guard trigger
/// refuses a lower value.
#[tokio::test]
async fn t9_raises_the_order_guard_only_when_it_stored_a_row_and_the_trigger_refuses_a_lower_one() {
    bounded::within("order guard", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        send(&ingest, &a, records([1])).await;
        let first: u64 = last_stored_xid(&w, &a).await.parse().expect("an id");
        send(&ingest, &a, records([1])).await;
        let resent: u64 = last_stored_xid(&w, &a).await.parse().expect("an id");
        assert_eq!(resent, first, "a batch that stored nothing leaves the guard");
        send(&ingest, &a, records([2])).await;
        let second: u64 = last_stored_xid(&w, &a).await.parse().expect("an id");
        assert!(second > first, "{second} > {first}");
        let lowered = sqlx::query(
            "UPDATE step_log_streams SET last_stored_xid = '1'::xid8 WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(a.run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect_err("the guard refuses a lower value");
        assert_eq!(
            crate::support::db_error(lowered),
            ("23001".to_owned(), Some("step_log_streams_guard".to_owned()))
        );
    })
    .await;
}

/// D041-145 (7): the database does not refuse an insert into a stream whose removal has begun; the ingest's T4 locks
/// the row and reads `removed_at`, so a batch after the mark is answered `REMOVED` and stores nothing.
#[tokio::test]
async fn a_batch_after_the_removal_mark_is_refused_removed() {
    bounded::within("removed", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        send(&ingest, &a, records([1])).await;
        sqlx::query(
            "UPDATE step_log_streams SET removed_at = now() WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(a.run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("the mark");
        assert_eq!(
            send(&ingest, &a, records([2])).await,
            stopped_attempt(StopReason::Removed)
        );
        assert_eq!(lines_of(&w, &a).await.len(), 1);
        assert_eq!(stream_of(&w, &a).await.expect("a stream").last_sequence, 1);
    })
    .await;
}

/// T041-10's test that needs the ingest (D041-144 (6)): an idle stream whose task lease lives is marked by the purge's
/// first step, and its next batch is answered `REMOVED`.
#[tokio::test]
async fn an_idle_stream_marked_by_the_purge_answers_its_next_batch_removed() {
    bounded::within("purge mark", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        send(&ingest, &a, records([1])).await;
        sqlx::query(
            "UPDATE step_log_streams SET expires_at = now() - interval '1 minute' \
              WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(a.run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("an idle stream");
        use sagat_application::maintenance::MaintenanceStore as _;
        assert_eq!(
            w.store
                .mark_expired_step_log_streams(500)
                .await
                .expect("mark"),
            1
        );
        assert_eq!(
            send(&ingest, &a, records([2])).await,
            stopped_attempt(StopReason::Removed)
        );
        assert_eq!(lines_of(&w, &a).await.len(), 1);
    })
    .await;
}

/// T041-10's test that needs the ingest (D041-144 (6)): a lowered retention leaves an existing stream's retention as it
/// was (its next batch moves its expiry by its own 14 days) and applies to a new stream (ADR-0027).
#[tokio::test]
async fn a_lowered_retention_applies_to_new_streams_only() {
    bounded::within("retention", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let ingest = ingest(&w);
        send(&ingest, &a, records([1])).await;
        set_namespace_policy(&w, QuotaResource::StepLogRetentionDays, 1).await;
        send(&ingest, &a, records([2])).await;
        let b = leased(&w, "b").await;
        send(&ingest, &b, records([1])).await;
        let kept: Vec<(i16, bool)> = sqlx::query_as(
            "SELECT retention_days, expires_at = last_at + make_interval(days => retention_days::integer) \
               FROM step_log_streams WHERE tenant_id = $1 AND run_id = ANY($2) ORDER BY run_id = $3 DESC",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(vec![a.run.as_uuid(), b.run.as_uuid()])
        .bind(a.run.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("the streams");
        assert_eq!(kept, [(14, true), (1, true)]);
    })
    .await;
}

/// A crash between the commit and the answer (the wrapper commits and then fails, as a lost answer would): the lines
/// are stored, and the resend stores nothing more and is answered `CONTINUE`. A failure before the commit stores
/// nothing, and the resend stores the batch.
#[tokio::test]
async fn a_lost_answer_is_resent_and_a_failed_commit_is_resent() {
    bounded::within("crash points", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let (lost, _) = hooked(&w, Hook::CommitThenFail);
        assert!(
            lost.append(&a.session.principal, &batch(&a, records([1, 2])))
                .await
                .is_err()
        );
        assert_eq!(lines_of(&w, &a).await.len(), 2, "the commit happened");
        assert_eq!(send(&ingest(&w), &a, records([1, 2])).await, continued(2));
        assert_eq!(lines_of(&w, &a).await.len(), 2);

        let b = leased(&w, "b").await;
        let (failed, _) = hooked(&w, Hook::FailBeforeCommit);
        assert!(
            failed
                .append(&b.session.principal, &batch(&b, records([1])))
                .await
                .is_err()
        );
        assert!(lines_of(&w, &b).await.is_empty());
        assert_eq!(stream_of(&w, &b).await, None);
        assert_eq!(send(&ingest(&w), &b, records([1])).await, continued(1));
        assert_eq!(lines_of(&w, &b).await.len(), 1);
    })
    .await;
}

/// The store proves it can serve protocol 1.7 (`WorkerStore::supports_step_log_protocol`), which `protocol_ceiling`
/// requires before it offers 1.7 (task T041-7).
#[tokio::test]
async fn the_store_proves_step_log_support() {
    let w = world().await;
    assert!(w.store.supports_step_log_protocol());
}
