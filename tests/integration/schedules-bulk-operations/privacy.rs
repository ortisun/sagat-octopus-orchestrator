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
use sagat_domain::identity::NamespaceId;
use sagat_telemetry::Metrics;
use uuid::Uuid;

use crate::firing::fixture;
use crate::support::engine::ctx;
use crate::support::logs::capture_with_spans;

fn options(id: &str, conflict: ConflictPolicy, reuse: ReusePolicy) -> StartOptions {
    StartOptions {
        business: Some(BusinessStart {
            id: BusinessId::new(id).expect("an identifier"),
            conflict,
            reuse,
            window: ReuseWindow::default(),
        }),
        ..StartOptions::default()
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
    let (guard, logs) = capture_with_spans(tracing::Level::TRACE);
    // the capture is alive: a line and a span of ours are in it, so an empty log means the platform
    // logged nothing, and no span line means no span was seen
    tracing::warn!(probe = "privacy-probe", "the capture is on");
    drop(tracing::info_span!("privacy_span_probe", probe = "kept"));
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
            // the binding as the lookup reads it, and a lookup that finds nothing
            let id = BusinessId::new(&canary).expect("an identifier");
            let binding = f
                .w
                .runs
                .business_binding(&f.w.admin, f.w.namespace.id(), &id)
                .await
                .expect("the binding");
            said.push(("the binding as read", format!("{binding:?}")));
            let nothing = f
                .w
                .runs
                .business_binding(&f.w.admin, NamespaceId::generate(), &id)
                .await
                .expect_err("another namespace holds nothing");
            said.push(("a lookup that finds nothing", format!("{nothing} {nothing:?}")));
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
        logs.text().contains("privacy-probe") && logs.text().contains("privacy_span_probe"),
        "the log capture was listening, spans included"
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

/// The places that keep the prefix of a schedule's action, and the ids made from it, on purpose: the revision that holds the
/// action, the binding and the run.
const PREFIX_ALLOWED: [&str; 3] = ["business_ids", "runs", "schedule_revisions"];

#[test]
fn the_prefix_of_a_schedules_action_and_the_ids_it_makes_reach_only_the_revision_the_binding_and_the_run()
 {
    use sagat_application::identity::RequestContext;
    use sagat_application::schedule_firing::TriggerRequest;
    use sagat_domain::schedules::Overlap;
    use serde_json::json;
    use time::Duration;

    use crate::firing::{hourly, t0};

    let canary = format!("canary-prefix-{}", Uuid::now_v7().simple());
    let hex: String = canary.bytes().map(|b| format!("{b:02x}")).collect();
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (guard, logs) = capture_with_spans(tracing::Level::TRACE);
    tracing::warn!(probe = "privacy-probe", "the capture is on");
    drop(tracing::info_span!("privacy_span_probe", probe = "kept"));
    let said = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let f = fixture().await;
            let mut said: Vec<(&str, String)> = Vec::new();
            let schedule = f
                .create(
                    "privacy-prefixed",
                    &hourly(),
                    &json!({"overlap": "ALLOW_ALL"}),
                    Some(json!({
                        "workflowId": f.workflow.to_string(),
                        "businessIdPrefix": canary,
                        "businessIdConflictPolicy": "FAIL"
                    })),
                )
                .await;
            said.push(("the schedule as created", format!("{schedule:?}")));
            // a person holds the first id: the action is a recorded conflict, then the next one starts a run
            let held = format!("{canary}-2026-10-28T13:00:00Z");
            let holder = crate::business_ids::start(
                &f,
                options(&held, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
                &crate::business_ids::key(),
            )
            .await
            .expect("the holder");
            f.clock.advance(Duration::hours(1) + Duration::seconds(5));
            f.drain(&schedule).await;
            f.clock.advance(Duration::hours(1));
            f.drain(&schedule).await;
            // trigger-now: a run under the id of its second, and a second request in the same second, refused by the id
            let ctx = RequestContext::default();
            f.clock.set(t0() + Duration::milliseconds(300_250));
            let ask = |key| TriggerRequest {
                overlap: Some(Overlap::AllowAll),
                key,
            };
            let made = f
                .firing
                .trigger(&f.w.admin, schedule.id, &ask("privacy-trigger-0001"), &ctx)
                .await
                .expect("trigger");
            f.clock.set(t0() + Duration::milliseconds(300_750));
            let refused = f
                .firing
                .trigger(&f.w.admin, schedule.id, &ask("privacy-trigger-0002"), &ctx)
                .await
                .expect("trigger");
            said.push(("a trigger that started", format!("{made:?}")));
            said.push(("a trigger that found the id held", format!("{refused:?}")));
            assert_eq!(refused.outcome, "BUSINESS_ID_CONFLICT");
            let read = f.read(&schedule).await;
            said.push(("the schedule as read", format!("{read:?}")));
            let described = f
                .service
                .describe(&f.w.admin, schedule.id, 3, 10)
                .await
                .expect("describe");
            said.push(("the description", format!("{described:?}")));
            f.w.operate_run(holder.run.info.run_id, RunOperation::Cancel)
                .await;
            assert_eq!(f.runs(&schedule).await.len(), 2, "the runs the schedule made");

            let tables: Vec<String> = sqlx::query_scalar(
                "SELECT tablename::text FROM pg_tables WHERE schemaname = current_schema() ORDER BY 1",
            )
            .fetch_all(&f.w.db.pool)
            .await
            .expect("tables");
            assert!(tables.len() > 50, "the whole schema is searched: {}", tables.len());
            let mut leaked_tables: Vec<String> = Vec::new();
            for table in tables {
                if PREFIX_ALLOWED.contains(&table.as_str()) {
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
            for table in PREFIX_ALLOWED {
                let kept: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%'"
                )))
                .bind(&canary)
                .fetch_one(&f.w.db.pool)
                .await
                .expect("count");
                assert!(kept > 0, "{table} keeps the prefix");
            }
            (said, leaked_tables)
        })
    });
    drop(guard);
    let (said, leaked_tables) = said;
    assert!(
        leaked_tables.is_empty(),
        "the prefix or an id made from it reached a table of the database: {leaked_tables:?}"
    );
    for (surface, text) in said {
        assert!(
            !text.contains(&canary) && !text.contains(&hex),
            "the prefix reached {surface}"
        );
    }
    assert!(
        logs.text().contains("privacy-probe") && logs.text().contains("privacy_span_probe"),
        "the log capture was listening, spans included"
    );
    assert!(
        !logs.text().contains(&canary),
        "the prefix reached a log line"
    );
    assert!(
        !metrics.render().contains(&canary),
        "the prefix reached a metric label"
    );
}

/// The places that keep the prefix of a batch selector, and the ids of the runs it reaches, on purpose: the job (`batch_jobs.selector`,
/// the one column that holds the selector, which the migration says is customer text and which no response, log or event repeats), and
/// the binding and the run of the ids themselves. Nothing else: not the audit trail, the outbox, the journal, the idempotency records
/// (which keep a hash of the request, not its text), the items or the payloads.
const BATCH_ALLOWED: [&str; 3] = ["batch_jobs", "business_ids", "runs"];

#[test]
fn the_prefix_of_a_batch_selector_reaches_only_the_job_that_holds_it() {
    use sagat_domain::batch::{Action, Filter, Selector};

    use crate::batch_executor::{create, executor, live, select, service, settle, write};

    let canary = format!("canary-selector-{}", Uuid::now_v7().simple());
    let hex: String = canary.bytes().map(|b| format!("{b:02x}")).collect();
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (guard, logs) = capture_with_spans(tracing::Level::TRACE);
    tracing::warn!(probe = "privacy-probe", "the capture is on");
    drop(tracing::info_span!("privacy_span_probe", probe = "kept"));
    let said = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let f = fixture().await;
            let mut said: Vec<(&str, String)> = Vec::new();
            // two runs under ids that start with the canary, and one that does not
            for n in 0..2 {
                crate::business_ids::start(
                    &f,
                    options(
                        &format!("{canary}-{n}"),
                        ConflictPolicy::Fail,
                        ReusePolicy::AllowDuplicate,
                    ),
                    &crate::business_ids::key(),
                )
                .await
                .expect("a run under an id of the prefix");
            }
            let by_prefix = Selector::filter(Filter {
                business_id_prefix: Some(canary.clone()),
                ..Filter::default()
            })
            .expect("selector");
            said.push(("the selector", format!("{by_prefix:?}")));
            // the preview, and what it answered
            let preview = service(&f)
                .preview(&f.w.admin, f.w.namespace.id(), &by_prefix, None)
                .await
                .expect("a preview by the prefix");
            assert_eq!(preview.matched, 2, "the preview reached the two runs");
            said.push(("the preview", format!("{preview:?}")));
            // the job: created, selected, applied, and read in every way the service reads it
            live(&f);
            let exec = executor(&f);
            let job = create(
                &f,
                write(Action::Cancel, by_prefix.clone()),
                "privacy-batch-selector-key",
            )
            .await;
            said.push(("the job as created", format!("{job:?}")));
            select(&exec, &job).await;
            for _ in 0..50 {
                f.clock.advance(time::Duration::seconds(1));
                if crate::batch_executor::job(&f, job.id).await.state
                    == sagat_domain::batch::JobState::Completed
                {
                    break;
                }
                exec.step(&crate::batch_executor::due(&job), None)
                    .await
                    .expect("step");
            }
            let _ = settle(&exec, &job).await;
            let read = crate::batch_executor::job(&f, job.id).await;
            assert_eq!(
                read.state,
                sagat_domain::batch::JobState::Completed,
                "the job ran to its end"
            );
            said.push(("the job as read", format!("{read:?}")));
            let listed = service(&f)
                .list(
                    &f.w.admin,
                    f.w.namespace.id(),
                    None,
                    &sagat_application::identity::PageRequest::default(),
                )
                .await
                .expect("the jobs");
            said.push(("the list of jobs", format!("{listed:?}")));
            let items = crate::batch_executor::items(&f, job.id).await;
            assert_eq!(items.len(), 2, "the two runs of the prefix");
            said.push(("the items", format!("{items:?}")));
            // a replay of the creation (the same key), which is answered from the record
            let again = create(
                &f,
                write(Action::Cancel, by_prefix.clone()),
                "privacy-batch-selector-key",
            )
            .await;
            said.push(("the replay of the creation", format!("{again:?}")));

            let tables: Vec<String> = sqlx::query_scalar(
                "SELECT tablename::text FROM pg_tables WHERE schemaname = current_schema() ORDER BY 1",
            )
            .fetch_all(&f.w.db.pool)
            .await
            .expect("tables");
            assert!(tables.len() > 50, "the whole schema is searched: {}", tables.len());
            let mut leaked_tables: Vec<String> = Vec::new();
            for table in tables {
                if BATCH_ALLOWED.contains(&table.as_str()) {
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
            for table in BATCH_ALLOWED {
                let kept: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%'"
                )))
                .bind(&canary)
                .fetch_one(&f.w.db.pool)
                .await
                .expect("count");
                assert!(kept > 0, "{table} keeps the prefix");
            }
            (said, leaked_tables)
        })
    });
    drop(guard);
    let (said, leaked_tables) = said;
    assert!(
        leaked_tables.is_empty(),
        "the prefix of a selector reached a table of the database: {leaked_tables:?}"
    );
    for (surface, text) in said {
        assert!(
            !text.contains(&canary) && !text.contains(&hex),
            "the prefix of a selector reached {surface}"
        );
    }
    assert!(
        logs.text().contains("privacy-probe") && logs.text().contains("privacy_span_probe"),
        "the log capture was listening, spans included"
    );
    assert!(
        !logs.text().contains(&canary),
        "the prefix of a selector reached a log line or a span"
    );
    assert!(
        !metrics.render().contains(&canary),
        "the prefix of a selector reached a metric label"
    );
}
/// How the rounds of the job fail in the canary test of a failed selection.
#[derive(Clone, Copy, Debug)]
enum Cause {
    /// The store of the test answers every round with an error whose text quotes the prefix: what the executor and the pass do with an
    /// error they are handed, whatever its text.
    HandedOn,
    /// The real store over a real database error whose message quotes the prefix (a trigger raises it when the round inserts an item):
    /// what the storage layer does with the error before anyone is handed it, `store_error` and its log line included (G2094, D2203).
    Database,
}

/// A job over a business-id prefix whose selection fails until it is failed for good leaves the prefix where it was put and in no
/// other table, log line, span or metric label, and the text of the errors of its rounds, which here quotes the prefix as a database
/// error can quote a value, is kept nowhere (G2093, D2197, D2203). The real store records the failures, so the count, the backoff, the
/// end of the job and its audit record are the real ones.
fn a_failed_selection_keeps_the_prefix_where_it_was_put(cause: Cause) {
    use std::sync::Arc;

    use sagat_application::batch::BatchStore;
    use sagat_application::batch_executor::{BatchExecutor, ExecutorSettings, Stepped};
    use sagat_domain::batch::{Action, Filter, JobState, SELECTION_RETRIES, Selector};
    use sagat_infra_postgres::{PgBatchStore, PgIdentityStore};

    use crate::batch_executor::{Runs, create, due, live, service, write};
    use crate::batch_selection_failures::{break_selection_quoting, role_pool, the_backoff_ends};
    use crate::firing::Fixture;
    use crate::gap_store::SlotComesBetween;
    use crate::support::schedules::TestClock;

    /// Steps the job to its end, moving the wait of each recorded failure; bounded by a time and by the count.
    async fn to_the_end<S: BatchStore>(
        f: &Fixture,
        exec: &BatchExecutor<S, Runs, PgIdentityStore, TestClock>,
        job: &sagat_application::batch::JobRecord,
    ) {
        let give_up = std::time::Instant::now() + StdDuration::from_secs(180);
        let mut recorded = 0;
        loop {
            assert!(
                std::time::Instant::now() < give_up && recorded <= SELECTION_RETRIES,
                "the job {} did not fail in time: {recorded} failed rounds recorded",
                job.id
            );
            match exec.step(&due(job), None).await.expect("step") {
                Stepped::Acted => break,
                Stepped::Backoff => {
                    recorded += 1;
                    the_backoff_ends(f, job.id).await;
                }
                // its selection waits for a transaction of the whole server
                _ => tokio::time::sleep(StdDuration::from_millis(20)).await,
            }
        }
        assert_eq!(
            recorded, SELECTION_RETRIES,
            "ten failed rounds, then the end"
        );
    }

    let canary = format!("canary-failed-selector-{}", Uuid::now_v7().simple());
    let hex: String = canary.bytes().map(|b| format!("{b:02x}")).collect();
    let quoting = format!("the statement over the ids that start with {canary} was cancelled");
    let (metrics, recorder) = Metrics::detached(StdDuration::from_secs(60));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (guard, logs, said, leaked_tables) = metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let f = fixture().await;
            let mut said: Vec<(&str, String)> = Vec::new();
            for n in 0..2 {
                crate::business_ids::start(
                    &f,
                    options(
                        &format!("{canary}-{n}"),
                        ConflictPolicy::Fail,
                        ReusePolicy::AllowDuplicate,
                    ),
                    &crate::business_ids::key(),
                )
                .await
                .expect("a run under an id of the prefix");
            }
            let by_prefix = Selector::filter(Filter {
                business_id_prefix: Some(canary.clone()),
                ..Filter::default()
            })
            .expect("selector");
            live(&f);
            let job = create(
                &f,
                write(Action::Cancel, by_prefix.clone()),
                "privacy-failed-selection-key",
            )
            .await;
            let pool = role_pool(&f).await;
            if matches!(cause, Cause::Database) {
                break_selection_quoting(&f.w.db.pool, job.id, &quoting).await;
            }
            // The capture is installed as late as the test allows, right before the rounds whose log is searched: `tracing`
            // caches per call site whether anybody listens, and a call site that another test of this binary registers at the
            // moment a subscriber is installed can be cached as "nobody" (`support/logs.rs`).
            let (guard, logs) = capture_with_spans(tracing::Level::TRACE);
            tracing::warn!(probe = "privacy-probe", "the capture is on");
            drop(tracing::info_span!("privacy_span_probe", probe = "kept"));
            match cause {
                Cause::HandedOn => {
                    let exec = BatchExecutor::new(
                        SlotComesBetween::failing_selection(pool.clone(), quoting.clone()),
                        Runs(Arc::clone(&f.w)),
                        PgIdentityStore::new(pool.clone()),
                        f.clock.clone(),
                        ExecutorSettings::default(),
                    );
                    to_the_end(&f, &exec, &job).await;
                }
                Cause::Database => {
                    let exec = BatchExecutor::new(
                        PgBatchStore::new(pool.clone()),
                        Runs(Arc::clone(&f.w)),
                        PgIdentityStore::new(pool.clone()),
                        f.clock.clone(),
                        ExecutorSettings::default(),
                    );
                    to_the_end(&f, &exec, &job).await;
                }
            }
            let read = crate::batch_executor::job(&f, job.id).await;
            assert_eq!(read.state, JobState::Failed, "the job failed its selection");
            said.push(("the job as read", format!("{read:?}")));
            let listed = service(&f)
                .list(
                    &f.w.admin,
                    f.w.namespace.id(),
                    None,
                    &sagat_application::identity::PageRequest::default(),
                )
                .await
                .expect("the jobs");
            said.push(("the list of jobs", format!("{listed:?}")));
            let items = crate::batch_executor::items(&f, job.id).await;
            said.push(("the items", format!("{items:?}")));
            let ending: Vec<String> = sqlx::query_scalar(
                "SELECT t::text FROM audit_records t WHERE action = 'batch.finished' AND resource_id = $1",
            )
            .bind(job.id.to_string())
            .fetch_all(&f.w.db.pool)
            .await
            .expect("the audit record of the end");
            assert_eq!(ending.len(), 1, "one record of the end");
            said.push(("the audit record of the end", ending.join("\n")));
            let events: Vec<String> = sqlx::query_scalar(
                "SELECT t::text FROM event_outbox t WHERE t::text LIKE '%' || $1 || '%'",
            )
            .bind(job.id.to_string())
            .fetch_all(&f.w.db.pool)
            .await
            .expect("the events that name the job");
            said.push(("the events that name the job", events.join("\n")));

            let tables: Vec<String> = sqlx::query_scalar(
                "SELECT tablename::text FROM pg_tables WHERE schemaname = current_schema() ORDER BY 1",
            )
            .fetch_all(&f.w.db.pool)
            .await
            .expect("tables");
            assert!(
                tables.len() > 50,
                "the whole schema is searched: {}",
                tables.len()
            );
            let mut leaked_tables: Vec<String> = Vec::new();
            for table in tables {
                if BATCH_ALLOWED.contains(&table.as_str()) {
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
            // the text of the errors is kept nowhere at all, the places that hold the prefix on purpose included
            for table in BATCH_ALLOWED {
                let kept: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%'"
                )))
                .bind("was cancelled")
                .fetch_one(&f.w.db.pool)
                .await
                .expect("count");
                assert_eq!(kept, 0, "{table} keeps the text of an error of a round");
                let holds_the_prefix: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} t WHERE t::text LIKE '%' || $1 || '%'"
                )))
                .bind(&canary)
                .fetch_one(&f.w.db.pool)
                .await
                .expect("count");
                assert!(holds_the_prefix > 0, "{table} keeps the prefix");
            }
            (guard, logs, said, leaked_tables)
        })
    });
    drop(guard);
    assert!(
        leaked_tables.is_empty(),
        "{cause:?}: the prefix, or the text of an error that quotes it, reached a table of the database: {leaked_tables:?}"
    );
    for (surface, text) in said {
        assert!(
            !text.contains(&canary) && !text.contains(&hex) && !text.contains("was cancelled"),
            "{cause:?}: the prefix, or the text of an error that quotes it, reached {surface}"
        );
    }
    let text = logs.text();
    assert!(
        text.contains("privacy-probe") && text.contains("privacy_span_probe"),
        "the log capture was listening, spans included"
    );
    assert!(
        text.contains("a batch selection round failed") && text.contains("a batch job failed"),
        "{cause:?}: the failures were logged, which is what this searches: {text}"
    );
    assert!(
        !text.contains(&canary) && !text.contains(&hex) && !text.contains("was cancelled"),
        "{cause:?}: the prefix, or the text of an error that quotes it, reached a log line or a span: {text}"
    );
    let page = metrics.render();
    assert!(
        page.contains("batch_selection_failures_total"),
        "{cause:?}: the failures were counted, which is what this searches: {page}"
    );
    assert!(
        !page.contains(&canary) && !page.contains("was cancelled"),
        "{cause:?}: the prefix, or the text of an error that quotes it, reached a metric label"
    );
    if matches!(cause, Cause::Database) {
        // what an operator is given in place of the text: the SQLSTATE of the error, a closed code
        assert!(
            text.contains("sqlstate=P0001"),
            "the executor's line names the SQLSTATE of the error of the round: {text}"
        );
    }
}

/// The error of a round, handed to the executor with a text that quotes the prefix of the selector, is read by nobody (D2197): an
/// executor and a pass that log `%error` would put the prefix in the log. Catches: the error logged by the executor.
#[test]
fn the_prefix_of_a_batch_selector_whose_selection_fails_reaches_only_the_job_that_holds_it() {
    a_failed_selection_keeps_the_prefix_where_it_was_put(Cause::HandedOn);
}

/// The same over the real path: the round fails for real inside the store, with a database error whose message quotes the prefix
/// (a trigger raises it when the round inserts an item), and nothing of it may reach a log line, a span, a label, an audit record or
/// a table (G2094, D2203). Catches: the storage layer logging the text of the error of a round (`store_error`).
#[test]
fn the_prefix_of_a_batch_selector_quoted_by_a_database_error_of_its_selection_reaches_no_log() {
    a_failed_selection_keeps_the_prefix_where_it_was_put(Cause::Database);
}
