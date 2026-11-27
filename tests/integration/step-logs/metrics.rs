//! Design 041 data-model.md §7, task T041-6: what the ingest counts, read from a recorder of the test's own
//! (`counted.rs`). Each series is asserted at its exact value after one batch of each kind: a stored batch with a
//! scrubbed, a hidden and a cut line, its resend, a lock timeout, and capture turned off.

use std::time::Duration;

use sagat_application::step_logs::StepLogVerdict;
use sagat_domain::quota::QuotaResource;
use sagat_domain::step_logs::{FieldValue, StepLogLevel};
use sqlx::Connection as _;

use crate::counted::{recorded, series};
use crate::support::bounded;
use crate::support::engine::world;
use crate::support::step_logs::{ingest, leased, record, records, send, set_namespace_policy};

const LIMIT: Duration = Duration::from_secs(120);

#[test]
fn the_ingest_counts_each_outcome_line_byte_cut_redaction_and_lock_timeout() {
    let page = recorded(|_| async {
        bounded::within("metrics", LIMIT, async {
            let w = world().await;
            let a = leased(&w, "a").await;
            let ingest = ingest(&w);
            let mut scrubbed = record(1, "key AKIAIOSFODNN7EXAMPLE");
            scrubbed.level = Some(StepLogLevel::Warn);
            let mut hidden = record(2, "m");
            hidden.fields = vec![("password".into(), FieldValue::Int(7))];
            let cut = record(3, &"x".repeat(9_000));
            let stored = vec![scrubbed, hidden, cut];
            assert_eq!(
                send(&ingest, &a, stored.clone()).await.verdict,
                StepLogVerdict::Continue
            );
            assert_eq!(
                send(&ingest, &a, stored).await.verdict,
                StepLogVerdict::Continue
            );
            // A lock timeout: another transaction holds the attempt's stream lock past 200 ms.
            let key = sagat_infra_postgres::stream_lock_key(
                w.admin.tenant_id(),
                a.task.run_id,
                a.task.step_id,
                a.task.fence.attempt,
                a.task.fence.generation,
            );
            let mut holder = sqlx::PgConnection::connect(&w.db.url)
                .await
                .expect("a holder");
            let mut tx = holder.begin().await.expect("a transaction");
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(&key)
                .execute(&mut *tx)
                .await
                .expect("the stream lock");
            assert!(matches!(
                send(&ingest, &a, records([4])).await.verdict,
                StepLogVerdict::RetryLater { .. }
            ));
            tx.rollback().await.expect("release");
            set_namespace_policy(&w, QuotaResource::StepLogAttemptRecords, 0).await;
            assert!(matches!(
                send(&ingest, &a, records([4])).await.verdict,
                StepLogVerdict::StopCapture(_)
            ));
        })
        .await;
    });
    let one = |name: &str, labels: &[(&str, &str)]| series(&page, name, labels);
    assert_eq!(
        one("step_log_batches_total", &[("outcome", "stored")]),
        Some(1.0)
    );
    assert_eq!(
        one("step_log_batches_total", &[("outcome", "duplicate")]),
        Some(1.0)
    );
    assert_eq!(
        one("step_log_batches_total", &[("outcome", "retry_later")]),
        Some(1.0)
    );
    assert_eq!(
        one("step_log_batches_total", &[("outcome", "stop_capture")]),
        Some(1.0)
    );
    assert_eq!(
        one("step_log_lines_stored_total", &[("level", "warn")]),
        Some(1.0)
    );
    assert_eq!(
        one("step_log_lines_stored_total", &[("level", "info")]),
        Some(2.0)
    );
    // "key [REDACTED]" (14) + "m" and {"password":"[REDACTED]"} (1 + 25) + 8,192 cut bytes.
    assert_eq!(
        one("step_log_bytes_stored_total", &[]),
        Some((14 + 1 + 25 + 8_192) as f64)
    );
    assert_eq!(one("step_log_lines_truncated_total", &[]), Some(1.0));
    assert_eq!(
        one("step_log_redactions_total", &[("rule", "shape")]),
        Some(1.0)
    );
    assert_eq!(
        one("step_log_redactions_total", &[("rule", "field_name")]),
        Some(1.0)
    );
    assert_eq!(one("step_log_lock_timeouts_total", &[]), Some(1.0));
}
