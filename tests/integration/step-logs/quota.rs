//! Design 041 section 2, T8, task T041-6: the tenant's step-log quota met by the quota-lease charge's raise rather
//! than by T6's unlocked read. The raise rolls the batch back, nothing is stored or counted, and the answer is
//! `RETRY_LATER` (D041-061); the resend is judged again. Each case has a gap in its batch and a later batch after it,
//! and the counts hold: lines stored + dropped by the worker + dropped by the server = `last_sequence`.
//!
//! The arithmetic follows the refill of ADR-0097 decision 4: a refill near the limit takes back the room of every
//! other quota lease no open transaction holds, then grants g = max(n, min(1 MiB, ⌊F / (2(A + 1))⌋)), F the free room
//! and A the other quota leases holding a grant; it raises `tenant_step_log_quota_held` when n > F but the other quota
//! leases' unused room would cover it, and `tenant_step_log_quota_exceeded` otherwise. The limit is 4,096 bytes.

use std::time::Duration;

use sagat_application::step_logs::{BatchAnswer, StepLogVerdict};

use crate::counted::{recorded, series};
use crate::support::bounded;
use crate::support::engine::world;
use crate::support::step_logs::{
    Hook, hooked_on, ingest, ingest_on_own_pool, leased, lines_of, pool_of, record, records, send,
    set_tenant_limit, spawn_send, step_log_usage, stream_of,
};

const LIMIT: Duration = Duration::from_secs(120);
const TENANT_LIMIT: i64 = 4_096;

const RETRY: BatchAnswer = BatchAnswer {
    verdict: StepLogVerdict::RetryLater { after_ms: 200 },
    stored_through_sequence: 0,
};

/// Two records of 1,000 bytes at sequences 1 and 3 (a gap at 2): a charge of 2 × (1,000 + 128) = 2,256 bytes.
fn gapped() -> Vec<sagat_application::step_logs::StepLogRecordIn> {
    vec![record(1, &"x".repeat(1_000)), record(3, &"y".repeat(1_000))]
}

/// (i) The tenant over its limit at T8 although T6 read room: A's batch (2,256 bytes) reads 4,096 bytes of room at T6
/// and is held after T7; B stores 2,128 bytes meanwhile; A's charge then finds 4,096 − 2,128 = 1,968 bytes free, B's
/// quota lease taken back (no open transaction holds it), and raises `tenant_step_log_quota_exceeded`:
/// `RETRY_LATER`, nothing stored or counted, `cause="limit"`. The resend reads the limit at T6 and is `FINAL_ONLY`, its
/// 2 records counted `tenant_quota` and its gap the worker's (0 + 1 + 2 = 3); a later batch adds its record (4).
#[test]
fn a_charge_over_the_limit_answers_retry_later_and_the_resend_counts_tenant_quota() {
    let page = recorded(|_| async {
        bounded::within("over the limit", LIMIT, async {
            let w = world().await;
            set_tenant_limit(&w, TENANT_LIMIT).await;
            let a = leased(&w, "a").await;
            let b = leased(&w, "b").await;
            let (held, gate) = hooked_on(pool_of(&w, 1).await, Hook::HoldAfterAuthority);
            let first = spawn_send(&held, &a, gapped());
            let holder = gate.reached().await;
            assert_eq!(
                send(&ingest(&w), &b, vec![record(1, &"z".repeat(2_000))]).await,
                BatchAnswer {
                    verdict: StepLogVerdict::Continue,
                    stored_through_sequence: 1
                }
            );
            gate.release();
            assert_eq!(first.await.expect("the batch").expect("answered"), RETRY);
            // The refused batch's transaction ended before its answer: its backend holds no lock any more (the
            // correctness review of task T041-6; a transaction left to its drop is rolled back later).
            let (state, locks): (Option<String>, i64) = sqlx::query_as(
                "SELECT (SELECT state FROM pg_stat_activity WHERE pid = $1), \
                        (SELECT count(*) FROM pg_locks WHERE pid = $1 AND locktype <> 'virtualxid')",
            )
            .bind(holder)
            .fetch_one(&w.db.pool)
            .await
            .expect("pg_stat_activity");
            assert_eq!((state.as_deref(), locks), (Some("idle"), 0));
            assert_eq!(
                stream_of(&w, &a).await,
                None,
                "nothing of A is stored or counted"
            );
            assert_eq!(step_log_usage(&w).await, 2_128);

            let again = ingest(&w);
            assert_eq!(
                send(&again, &a, gapped()).await,
                BatchAnswer {
                    verdict: StepLogVerdict::FinalOnly,
                    stored_through_sequence: 3
                }
            );
            assert_eq!(
                send(&again, &a, records([4])).await,
                BatchAnswer {
                    verdict: StepLogVerdict::FinalOnly,
                    stored_through_sequence: 4
                }
            );
            let stream = stream_of(&w, &a).await.expect("A's stream");
            assert_eq!(
                (
                    stream.records,
                    stream.dropped_by_worker,
                    stream.dropped_by_server,
                    stream.last_sequence,
                    stream.ceiling_reason.as_deref()
                ),
                (0, 1, 3, 4, Some("tenant_quota"))
            );
            assert!(lines_of(&w, &a).await.is_empty());
            assert_eq!(step_log_usage(&w).await, 2_128);
        })
        .await;
    });
    assert_eq!(
        series(
            &page,
            "step_log_quota_refusals_total",
            &[("cause", "limit")]
        ),
        Some(1.0)
    );
    assert_eq!(
        series(&page, "step_log_quota_retries_total", &[]),
        Some(1.0)
    );
    assert_eq!(
        series(
            &page,
            "step_log_lines_dropped_total",
            &[("reason", "tenant_quota")]
        ),
        Some(3.0)
    );
    assert_eq!(
        series(
            &page,
            "step_log_lines_dropped_total",
            &[("reason", "worker_gap")]
        ),
        Some(1.0)
    );
}

/// (ii) The last room held by another open transaction's quota lease: A stores 134 bytes (its refill grants its
/// quota lease ⌊4,096 / 2⌋ = 2,048) and commits; A's next batch charges 134 more into that quota lease and is held
/// after T8, holding it. B's batch (2,256 bytes) reads room at T6 (4,096 − 134), and its refill cannot take A's quota
/// lease back: F = 4,096 − 2,048 = 2,048 < 2,256 ≤ 2,048 + (2,048 − 134) (A's second charge is not visible to the refill's snapshot), so it raises `tenant_step_log_quota_held`:
/// `RETRY_LATER`, `cause="held"`, nothing stored. Once A's transaction has ended, B's resend takes A's room back (S =
/// 268, F = 3,828) and is stored and charged 2,256; a later batch of 134 bytes is stored (2 + 1 + 0 = 3, then 4).
#[test]
fn room_held_by_an_open_transaction_answers_retry_later_and_the_resend_is_stored_once_it_ends() {
    let page = recorded(|_| async {
        bounded::within("held room", LIMIT, async {
            let w = world().await;
            set_tenant_limit(&w, TENANT_LIMIT).await;
            let a = leased(&w, "a").await;
            let b = leased(&w, "b").await;
            let pool = pool_of(&w, 1).await;
            let (writer, _) = hooked_on(pool.clone(), Hook::CommitThenFail);
            assert!(
                writer
                    .append(
                        &a.session.principal,
                        &crate::support::step_logs::batch(&a, records([1]))
                    )
                    .await
                    .is_err(),
                "the first batch commits (its answer is the wrapper's failure)"
            );
            let (held, gate) = hooked_on(pool, Hook::HoldAfterInsert);
            let second = spawn_send(&held, &a, records([2]));
            gate.reached().await;
            let other = ingest_on_own_pool(&w, 1).await;
            assert_eq!(send(&other, &b, gapped()).await, RETRY);
            assert_eq!(
                stream_of(&w, &b).await,
                None,
                "nothing of B is stored or counted"
            );
            gate.release();
            assert_eq!(
                second.await.expect("the batch").expect("answered").verdict,
                StepLogVerdict::Continue
            );
            assert_eq!(
                send(&other, &b, gapped()).await,
                BatchAnswer {
                    verdict: StepLogVerdict::Continue,
                    stored_through_sequence: 3
                }
            );
            assert_eq!(
                send(&other, &b, records([4])).await,
                BatchAnswer {
                    verdict: StepLogVerdict::Continue,
                    stored_through_sequence: 4
                }
            );
            let stream = stream_of(&w, &b).await.expect("B's stream");
            assert_eq!(
                (
                    stream.records,
                    stream.dropped_by_worker,
                    stream.dropped_by_server,
                    stream.last_sequence
                ),
                (3, 1, 0, 4)
            );
            // A: 2 × 134; B: 2,256 + 134.
            assert_eq!(step_log_usage(&w).await, 268 + 2_256 + 134);
        })
        .await;
    });
    assert_eq!(
        series(&page, "step_log_quota_refusals_total", &[("cause", "held")]),
        Some(1.0)
    );
    assert_eq!(
        series(
            &page,
            "step_log_quota_refusals_total",
            &[("cause", "limit")]
        ),
        None
    );
}
