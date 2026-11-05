//! A business identifier is customer text (`T018-039`, `FR-018-042`, `SC-018-014`): it is kept in the
//! binding and on the run, and in no other place. Not in a log (at every level), not in a metric, not
//! in the text of an error, and not in any other table of the database: the audit trail, the outbox,
//! the journal, the dedupe records and the payloads included. A canary identifier goes through a start,
//! a refusal, a returned holder, a cancellation and the refusal of a reuse, and everything the
//! platform wrote or said is searched for it. Real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use sagat_application::error::AppError;
use sagat_application::runs::{BusinessStart, RunOperation, StartOptions, StartRun};
use sagat_domain::business_ids::{BusinessId, ConflictPolicy, ReusePolicy, ReuseWindow};
use sagat_telemetry::Metrics;
use uuid::Uuid;

use crate::firing::fixture;
use crate::support::engine::ctx;
use crate::support::logs::capture_at;

fn options(id: &str, conflict: ConflictPolicy, reuse: ReusePolicy) -> StartOptions {
    StartOptions {
        business: Some(BusinessStart {
            id: BusinessId::new(id).expect("an identifier"),
            conflict,
            reuse,
            window: ReuseWindow::default(),
        }),
    }
}

/// The places that keep an identifier on purpose.
const ALLOWED: [&str; 2] = ["business_ids", "runs"];

#[test]
fn an_identifier_reaches_only_the_binding_and_the_run() {
    let canary = format!("canary-business-{}", Uuid::now_v7().simple());
    let hex: String = canary.bytes().map(|b| format!("{b:02x}")).collect();
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (guard, logs) = capture_at(tracing::Level::TRACE);
    // the capture is alive: a line of ours is in it, so an empty log means the platform logged nothing
    tracing::warn!(probe = "privacy-probe", "the capture is on");
    let said = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let f = fixture().await;
            let mut said: Vec<(&str, String)> = Vec::new();
            let mut leaked_tables: Vec<String> = Vec::new();
            let start = |conflict, reuse| {
                let f = &f;
                let canary = &canary;
                async move {
                    f.w.runs
                        .start_with_options(
                            &f.w.admin,
                            &StartRun {
                                namespace: "payments",
                                workflow: f.workflow,
                                version: None,
                                input: b"{}",
                                correlation_id: None,
                                deadline: None,
                                key: &format!("privacy-{}", Uuid::now_v7()),
                            },
                            options(canary, conflict, reuse),
                            &ctx(),
                        )
                        .await
                }
            };
            let first = start(ConflictPolicy::Fail, ReusePolicy::AllowDuplicate)
                .await
                .expect("the first start");
            // a refusal, and the text of the error as a client or a log would see it
            let refused = start(ConflictPolicy::Fail, ReusePolicy::AllowDuplicate)
                .await
                .expect_err("held");
            assert!(matches!(refused, AppError::BusinessIdConflict { .. }));
            said.push(("the text of a refusal", format!("{refused} {refused:?}")));
            // the holder returned in place of a new run
            let again = start(ConflictPolicy::UseExisting, ReusePolicy::AllowDuplicate)
                .await
                .expect("the holder");
            said.push(("the options of a start", format!("{:?}", options(&canary, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate))));
            assert!(again.existing);
            // the holder ends, and a start that may not reuse it is refused
            f.w.operate_run(first.run.info.run_id, RunOperation::Cancel)
                .await;
            let rejected = start(ConflictPolicy::Fail, ReusePolicy::RejectDuplicate)
                .await
                .expect_err("remembered");
            said.push(("the text of a reuse refusal", format!("{rejected} {rejected:?}")));

            // every table of the database, as text and as the hex of its bytes
            let tables: Vec<String> = sqlx::query_scalar(
                "SELECT tablename::text FROM pg_tables WHERE schemaname = current_schema() ORDER BY 1",
            )
            .fetch_all(&f.w.db.pool)
            .await
            .expect("tables");
            assert!(tables.len() > 50, "the whole schema is searched: {}", tables.len());
            for table in tables {
                if ALLOWED.contains(&table.as_str()) {
                    continue;
                }
                let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%' OR t::text LIKE '%' || $2 || '%'"
                )))
                .bind(&canary)
                .bind(&hex)
                .fetch_one(&f.w.db.pool)
                .await
                .unwrap_or_else(|error| panic!("search {table}: {error}"));
                if hits > 0 {
                    leaked_tables.push(table);
                }
            }
            // the two places that keep it do
            for table in ALLOWED {
                let kept: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%'"
                )))
                .bind(&canary)
                .fetch_one(&f.w.db.pool)
                .await
                .expect("count");
                assert!(kept > 0, "{table} keeps the identifier");
            }
            (said, leaked_tables)
        })
    });
    drop(guard);
    let (said, leaked_tables) = said;

    assert!(
        leaked_tables.is_empty(),
        "the identifier reached a table of the database: {leaked_tables:?}"
    );
    for (surface, text) in said {
        assert!(
            !text.contains(&canary) && !text.contains(&hex),
            "the identifier reached {surface}"
        );
    }
    assert!(
        logs.text().contains("privacy-probe"),
        "the log capture was listening"
    );
    assert!(
        !logs.text().contains(&canary),
        "the identifier reached a log line"
    );
    assert!(
        !metrics.render().contains(&canary),
        "the identifier reached a metric label"
    );
}
