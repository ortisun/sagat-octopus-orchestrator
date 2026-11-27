//! Design 041 section 2, T3, task T041-6: the locks a batch waits for. It takes the six policy fences of the
//! namespace's step-log resources shared, so a change of a resource waits for the batches that read its old value;
//! and, after the fences, the attempt's stream lock (L1b), so that a second batch of the same attempt waits for the
//! first to commit, and answers `RETRY_LATER` when that takes longer than its lock timeout. Held transactions and
//! `pg_stat_activity` / `pg_locks`, never sleeps.

use std::time::Duration;

use sagat_application::step_logs::{BatchAnswer, StepLogVerdict};
use sagat_domain::quota::QuotaResource;
use sagat_domain::step_logs::StopReason;
use sqlx::PgPool;

use crate::support::bounded;
use crate::support::engine::{ctx, world};
use crate::support::step_logs::{
    HELD_LOCK_TIMEOUT, Hook, hooked, ingest, leased, lines_of, records, send, spawn_send,
    until_blocked_by,
};

const LIMIT: Duration = Duration::from_secs(120);

/// The advisory locks a backend holds and waits for: (mode, granted, count).
async fn advisory_locks(pool: &PgPool, pid: i32) -> Vec<(String, bool, i64)> {
    sqlx::query_as(
        "SELECT mode, granted, count(*) FROM pg_locks \
          WHERE pid = $1 AND locktype = 'advisory' GROUP BY mode, granted ORDER BY mode, granted",
    )
    .bind(pid)
    .fetch_all(pool)
    .await
    .expect("pg_locks")
}

/// A change of `step_log_attempt_records` to 0 waits for a batch held after T3 (its fences held shared), and commits
/// after it: the batch is stored under the old value, and the next batch reads capture off.
#[tokio::test]
async fn a_change_of_capture_to_zero_waits_for_a_batch_held_after_t3_and_commits_after_it() {
    bounded::within("change waits", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let (held, gate) = hooked(&w, Hook::HoldAfterPolicies);
        let batch = spawn_send(&held, &a, records([1]));
        let holder = gate.reached().await;
        let context = ctx();
        let change = w.quotas.set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::StepLogAttemptRecords,
            0,
            &context,
        );
        let (changed, ()) = tokio::join!(change, async {
            until_blocked_by(&w.db.pool, holder).await;
            gate.release();
        });
        changed.expect("the change commits after the batch");
        assert_eq!(
            batch.await.expect("the batch").expect("answered"),
            BatchAnswer {
                verdict: StepLogVerdict::Continue,
                stored_through_sequence: 1
            }
        );
        assert_eq!(lines_of(&w, &a).await.len(), 1);
        assert_eq!(
            send(&ingest(&w), &a, records([2])).await,
            BatchAnswer {
                verdict: StepLogVerdict::StopCapture(StopReason::CaptureOff),
                stored_through_sequence: 0
            }
        );
    })
    .await;
}

/// A batch of the same attempt as one held after T3 waits on the attempt's stream lock, holding the six policy
/// fences already (L1 before L1b): `pg_locks` shows its six shared advisory locks granted and one exclusive advisory
/// lock waited for. With the lock timeout of 200 ms it answers `RETRY_LATER` and stores nothing; with a longer one it
/// commits after the first.
#[tokio::test]
async fn a_second_batch_of_the_attempt_waits_on_the_stream_lock_after_the_fences() {
    bounded::within("stream lock", LIMIT, async {
        let w = world().await;
        let a = leased(&w, "a").await;
        let (held, gate) = hooked(&w, Hook::HoldAfterPolicies);
        let first = spawn_send(&held, &a, records([1]));
        let holder = gate.reached().await;

        assert_eq!(
            send(&ingest(&w), &a, records([2])).await,
            BatchAnswer {
                verdict: StepLogVerdict::RetryLater { after_ms: 200 },
                stored_through_sequence: 0
            },
            "the 200 ms lock timeout"
        );

        let patient = sagat_application::step_logs::StepLogIngest::new(
            sagat_infra_postgres::PgStepLogStore::new(w.execution_pool.clone())
                .with_lock_timeout(HELD_LOCK_TIMEOUT),
            sagat_application::clock::SystemClock,
            true,
        );
        let second = {
            let principal = a.session.principal.clone();
            let batch = crate::support::step_logs::batch(&a, records([2]));
            tokio::spawn(async move { patient.append(&principal, &batch).await })
        };
        let waiter = until_blocked_by(&w.db.pool, holder).await;
        assert_eq!(
            advisory_locks(&w.db.pool, waiter).await,
            [
                ("ExclusiveLock".to_owned(), false, 1),
                ("ShareLock".to_owned(), true, 6)
            ]
        );
        gate.release();
        assert_eq!(
            first.await.expect("the first").expect("answered").verdict,
            StepLogVerdict::Continue
        );
        assert_eq!(
            second.await.expect("the second").expect("answered"),
            BatchAnswer {
                verdict: StepLogVerdict::Continue,
                stored_through_sequence: 2
            }
        );
        assert_eq!(lines_of(&w, &a).await.len(), 2);
    })
    .await;
}
