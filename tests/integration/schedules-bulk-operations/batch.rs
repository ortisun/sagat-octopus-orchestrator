//! Batch jobs, the part that does not touch runs (`T018-045`, `FR-018-027`, `FR-018-028`,
//! `FR-018-032`, `FR-018-033`, `FR-018-034`): creating a job, its limits and its payload, who may,
//! the ceilings of active jobs, the same key twice, reading, listing, cancelling and previewing.
//! Selecting the runs and applying the action are the executor's (`batch_executor.rs`). Real
//! PostgreSQL, a real run service, a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::batch::{BatchService, BatchWrite, JobRecord, NamedPayload};
use sagat_application::error::AppError;
use sagat_application::identity::{PageRequest, RequestContext};
use sagat_domain::batch::{Action, Filter, ItemState, JobState, Justification, Rate, Selector};
use sagat_domain::execution::RunStatus;
use sagat_domain::identity::Role;
use sagat_infra_postgres::PgBatchStore;
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::firing::{Fixture, fixture};
use crate::support::schedules::TestClock;

fn service(f: &Fixture) -> BatchService<PgBatchStore, TestClock> {
    BatchService::new(PgBatchStore::new(f.w.db.pool.clone()), f.clock.clone())
}

fn ctx() -> RequestContext {
    RequestContext::default()
}

/// A cancel job over one run id nobody has, which is enough for a job that is only created.
fn cancel() -> BatchWrite {
    BatchWrite {
        selector: Selector::run_ids(vec![Uuid::new_v4()]).expect("selector"),
        action: Action::Cancel,
        step_id: None,
        instance_path: Vec::new(),
        signal: None,
        update: None,
        drop_business_id: false,
        repin: false,
        reason: Justification::new("clean up the runs that are stuck").expect("reason"),
        max_items: None,
        rate: None,
    }
}

async fn create(f: &Fixture, write: BatchWrite, key: &str) -> Result<JobRecord, AppError> {
    service(f)
        .create(&f.w.admin, f.w.namespace.id(), write, key, &ctx())
        .await
}

async fn jobs_stored(f: &Fixture) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM batch_jobs")
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count")
}

fn reason_of(error: &AppError) -> &'static str {
    match error {
        AppError::InvalidBatch { reason, .. } => reason,
        other => panic!("not a batch validation error: {other:?}"),
    }
}

// --- creating -------------------------------------------------------------------------------------

#[tokio::test]
async fn a_job_is_created_selecting_with_its_limits_and_an_audit_record() {
    let f = fixture().await;
    let write = BatchWrite {
        max_items: Some(50),
        rate: Some(Rate::per_second(5).expect("rate")),
        ..cancel()
    };

    let job = create(&f, write, "create-0001").await.expect("create");

    assert_eq!(job.state, JobState::Selecting);
    assert_eq!(job.action, Action::Cancel);
    assert_eq!((job.max_items, job.rate.milli()), (50, 5_000));
    assert_eq!(
        job.counts.selected + job.counts.succeeded + job.counts.failed,
        0
    );
    assert_eq!(job.created_by, "alice");
    assert_eq!(job.expires_at - job.created_at, Duration::days(30));
    assert!(job.payload_digest.is_none());
    let horizon: i64 =
        sqlx::query_scalar("SELECT selection_horizon::text::bigint FROM batch_jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("horizon");
    assert!(
        horizon > 0,
        "the transaction snapshot the selection waits for"
    );
    let audit: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT action, actor, reason FROM audit_records WHERE resource_id = $1")
            .bind(job.id.to_string())
            .fetch_all(&f.w.db.pool)
            .await
            .expect("audit");
    assert_eq!(
        audit,
        vec![(
            "batch.created".to_owned(),
            "alice".to_owned(),
            Some("CANCEL".to_owned())
        )],
        "the audit record names the action, never the justification"
    );
}

#[tokio::test]
async fn the_payload_of_a_signal_is_kept_as_content_and_never_returned() {
    let f = fixture().await;
    let write = BatchWrite {
        action: Action::Signal,
        signal: Some(NamedPayload {
            name: "approve".to_owned(),
            body: br#"{"ticket":"canary-018"}"#.to_vec(),
        }),
        ..cancel()
    };

    let job = create(&f, write, "create-signal").await.expect("create");

    assert_eq!(
        job.params,
        json!({"name": "approve"}),
        "the name, not the body"
    );
    let digest = job
        .payload_digest
        .expect("the payload is named by its digest");
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
            .bind(job.tenant_id.as_uuid())
            .bind(digest.as_slice())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("payload");
    assert_eq!(stored, br#"{"ticket":"canary-018"}"#.to_vec());
    let leaked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM batch_jobs WHERE to_jsonb(batch_jobs)::text LIKE '%canary-018%') \
         OR EXISTS (SELECT 1 FROM audit_records WHERE to_jsonb(audit_records)::text LIKE '%canary-018%')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("canary");
    assert!(!leaked, "the body is in the payload row and nowhere else");
}

#[tokio::test]
async fn creating_a_job_needs_run_batch_and_every_permission_of_the_action() {
    let f = fixture().await;
    // room for every job the allowed cases create: the deployment ceiling first, then the limit
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'batch_jobs', 16, 'operator')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a higher ceiling");
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', 16, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("room for the jobs");
    let cases = [
        (Role::Viewer, Action::Cancel, false),
        (Role::Developer, Action::Cancel, false),
        (Role::Operator, Action::Cancel, true),
        (Role::Operator, Action::Pause, true),
        (Role::Operator, Action::Rerun, true),
        (Role::Operator, Action::MarkManualReview, true),
        (Role::Developer, Action::MarkManualReview, false),
    ];
    for (n, (role, action, allowed)) in cases.into_iter().enumerate() {
        let who = f.w.member(&format!("batch-{n}"), role).await;
        let write = BatchWrite {
            action,
            step_id: matches!(action, Action::MarkManualReview).then(|| "a".to_owned()),
            ..cancel()
        };
        let answer = service(&f)
            .create(
                &who,
                f.w.namespace.id(),
                write,
                &format!("permission-key-{n}"),
                &ctx(),
            )
            .await;
        assert_eq!(
            answer.is_ok(),
            allowed,
            "{role:?} may create {action:?}: {answer:?}"
        );
        if !allowed {
            assert!(
                matches!(answer, Err(AppError::Forbidden { .. })),
                "{answer:?}"
            );
        }
    }
}

#[tokio::test]
async fn parameters_the_action_does_not_take_and_limits_out_of_range_create_nothing() {
    let f = fixture().await;
    let cases: Vec<(BatchWrite, &str)> = vec![
        (
            BatchWrite {
                step_id: Some("a".to_owned()),
                ..cancel()
            },
            "BATCH_PARAMS_INVALID",
        ),
        (
            BatchWrite {
                action: Action::RetryStep,
                ..cancel()
            },
            "BATCH_PARAMS_INVALID",
        ),
        (
            BatchWrite {
                drop_business_id: true,
                ..cancel()
            },
            "BATCH_PARAMS_INVALID",
        ),
        (
            BatchWrite {
                action: Action::Repin,
                repin: true,
                ..cancel()
            },
            "BATCH_ACTION_UNAVAILABLE",
        ),
        (
            // a declared Update is not applied by a batch yet (G1959): refuse it, do not fail every item
            BatchWrite {
                action: Action::Update,
                ..cancel()
            },
            "BATCH_ACTION_UNAVAILABLE",
        ),
        (
            BatchWrite {
                selector: Selector::filter(Filter {
                    business_id_prefix: Some("order-".to_owned()),
                    ..Filter::default()
                })
                .expect("selector"),
                ..cancel()
            },
            "BATCH_SELECTOR_UNSUPPORTED",
        ),
        (
            BatchWrite {
                action: Action::Signal,
                signal: Some(NamedPayload {
                    name: "approve".to_owned(),
                    body: b"not json".to_vec(),
                }),
                ..cancel()
            },
            "BATCH_PAYLOAD_INVALID",
        ),
        (
            BatchWrite {
                max_items: Some(0),
                ..cancel()
            },
            "BATCH_LIMITS_INVALID",
        ),
        (
            BatchWrite {
                max_items: Some(100_001),
                ..cancel()
            },
            "BATCH_LIMITS_INVALID",
        ),
    ];
    for (n, (write, reason)) in cases.into_iter().enumerate() {
        let error = create(&f, write, &format!("refused-{n}"))
            .await
            .expect_err(reason);
        assert_eq!(reason_of(&error), reason);
    }
    assert_eq!(jobs_stored(&f).await, 0, "nothing was stored");
}

// --- the same key ---------------------------------------------------------------------------------

#[tokio::test]
async fn the_same_key_creates_one_job_and_a_changed_request_is_refused() {
    let f = fixture().await;
    let write = cancel();
    let first = create(&f, write.clone(), "same-key").await.expect("first");
    let again = create(&f, write.clone(), "same-key").await.expect("replay");
    assert_eq!(
        again.id, first.id,
        "the replay is the job the first request made"
    );
    assert_eq!(jobs_stored(&f).await, 1);

    let changed = BatchWrite {
        reason: Justification::new("a different justification").expect("reason"),
        ..write
    };
    let refused = create(&f, changed, "same-key").await;
    assert!(
        matches!(refused, Err(AppError::IdempotencyKeyReused)),
        "{refused:?}"
    );
    assert_eq!(jobs_stored(&f).await, 1);
}

#[tokio::test]
async fn a_key_that_is_too_short_or_too_long_is_refused_before_anything_is_stored() {
    let f = fixture().await;
    for key in ["short", &"k".repeat(161)] {
        let refused = create(&f, cancel(), key).await;
        assert!(
            matches!(refused, Err(AppError::NotAllowed(_))),
            "{refused:?}"
        );
    }
    assert_eq!(jobs_stored(&f).await, 0);
}

// --- the ceilings of active jobs ---------------------------------------------------------------------

#[tokio::test]
async fn active_jobs_are_limited_per_namespace_and_a_limit_of_zero_refuses_before_any_work() {
    let f = fixture().await;
    create(&f, cancel(), "batch-one").await.expect("one");
    create(&f, cancel(), "batch-two").await.expect("two");
    let third = create(&f, cancel(), "batch-three").await;
    assert!(
        matches!(third, Err(AppError::QuotaExceeded(_))),
        "two active jobs per namespace by default: {third:?}"
    );
    assert_eq!(jobs_stored(&f).await, 2);

    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a limit of zero");
    let zero = create(&f, cancel(), "batch-four").await;
    assert!(matches!(zero, Err(AppError::QuotaExceeded(_))), "{zero:?}");
    assert_eq!(jobs_stored(&f).await, 2, "nothing more was stored");
}

#[tokio::test]
async fn the_active_jobs_of_a_tenant_are_limited_too() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, NULL, 'tenant_batch_jobs', 1, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("a tenant limit of one");
    create(&f, cancel(), "first-key").await.expect("first");

    let second = create(&f, cancel(), "second-key").await;

    assert!(
        matches!(second, Err(AppError::QuotaExceeded(_))),
        "the namespace has room, the tenant has none: {second:?}"
    );
}

#[tokio::test]
async fn a_creation_waits_for_the_fence_of_the_active_jobs_before_it_counts_them() {
    let f = fixture().await;
    let tenant = f.w.admin.tenant_id();
    // another transaction holds the fence of the tenant's active jobs
    let mut held = f.w.db.pool.begin().await.expect("hold the fence");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("quota:tenant_batch_jobs:{tenant}"))
        .execute(&mut *held)
        .await
        .expect("take the fence");

    let creation = create(&f, cancel(), "fenced-job-1");
    let release = async {
        f.wait_for_a_backend_on_a_lock("the creation waits for the fence of the active jobs")
            .await;
        assert_eq!(
            jobs_stored(&f).await,
            0,
            "nothing is counted or stored while it waits"
        );
        held.commit().await.expect("release the fence");
    };
    let (job, ()) = tokio::join!(creation, release);

    job.expect("the creation goes on once the fence is free");
    assert_eq!(jobs_stored(&f).await, 1);
}

#[tokio::test]
async fn a_race_for_the_last_place_makes_exactly_one_job() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'batch_jobs', 1, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("room for one");
    let keys: Vec<String> = (0..6).map(|n| format!("race-key-{n}")).collect();
    let attempts = keys.iter().map(|key| create(&f, cancel(), key));

    let answers = futures_util::future::join_all(attempts).await;

    let created = answers.iter().filter(|a| a.is_ok()).count();
    let refused = answers
        .iter()
        .filter(|a| matches!(a, Err(AppError::QuotaExceeded(_))))
        .count();
    assert_eq!((created, refused), (1, 5), "{answers:?}");
    assert_eq!(jobs_stored(&f).await, 1);
}

// --- reading, listing, cancelling ----------------------------------------------------------------------

#[tokio::test]
async fn jobs_are_read_listed_newest_first_and_never_across_tenants() {
    let f = fixture().await;
    let a = create(&f, cancel(), "read-job-a").await.expect("a");
    let b = create(&f, cancel(), "read-job-b").await.expect("b");
    let viewer = f.w.member("reader", Role::Viewer).await;
    let service = service(&f);

    assert_eq!(service.get(&viewer, a.id).await.expect("get").id, a.id);
    let page = service
        .list(
            &viewer,
            f.w.namespace.id(),
            None,
            &PageRequest {
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .expect("list");
    assert_eq!(
        page.items.iter().map(|j| j.id).collect::<Vec<_>>(),
        vec![b.id]
    );
    let next = service
        .list(
            &viewer,
            f.w.namespace.id(),
            None,
            &PageRequest {
                limit: Some(1),
                cursor: page.next_cursor,
            },
        )
        .await
        .expect("next page");
    assert_eq!(
        next.items.iter().map(|j| j.id).collect::<Vec<_>>(),
        vec![a.id]
    );
    assert!(next.next_cursor.is_none());
    let cancelling = service
        .list(
            &viewer,
            f.w.namespace.id(),
            Some(JobState::Cancelling),
            &PageRequest::default(),
        )
        .await
        .expect("filtered");
    assert!(cancelling.items.is_empty());

    let stranger = &f.w.other_admin;
    assert!(matches!(
        service.get(stranger, a.id).await,
        Err(AppError::NotFound)
    ));
    assert!(matches!(
        service
            .list(stranger, f.w.namespace.id(), None, &PageRequest::default())
            .await,
        Err(AppError::NotFound)
    ));
    let items = service
        .items(
            &viewer,
            a.id,
            Some(ItemState::Pending),
            &PageRequest::default(),
        )
        .await
        .expect("items");
    assert!(items.items.is_empty(), "no run was selected yet");
}

#[tokio::test]
async fn cancelling_stops_the_job_once_audits_once_and_leaves_a_finished_job_alone() {
    let f = fixture().await;
    let job = create(&f, cancel(), "to-cancel").await.expect("create");
    let service = service(&f);
    let viewer = f.w.member("watcher", Role::Viewer).await;
    assert!(matches!(
        service.cancel(&viewer, job.id, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        service.cancel(&f.w.other_admin, job.id, &ctx()).await,
        Err(AppError::NotFound)
    ));

    let cancelling = service
        .cancel(&f.w.admin, job.id, &ctx())
        .await
        .expect("cancel");
    let twice = service
        .cancel(&f.w.admin, job.id, &ctx())
        .await
        .expect("again");

    assert_eq!(cancelling.state, JobState::Cancelling);
    assert_eq!(
        twice.state,
        JobState::Cancelling,
        "asking twice is asking once"
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'batch.cancelled' AND resource_id = $1",
    )
    .bind(job.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audits, 1);

    // a job that is over stays as it ended
    let done = create(&f, cancel(), "done-job-1").await.expect("done");
    sqlx::query("UPDATE batch_jobs SET state = 'COMPLETED', finished_at = now() WHERE id = $1")
        .bind(done.id)
        .execute(&f.w.db.pool)
        .await
        .expect("finish");
    let unchanged = service
        .cancel(&f.w.admin, done.id, &ctx())
        .await
        .expect("cancel");
    assert_eq!(unchanged.state, JobState::Completed);
}

// --- previewing ----------------------------------------------------------------------------------------

#[tokio::test]
async fn a_preview_counts_up_to_the_cap_samples_a_few_and_changes_nothing() {
    let f = fixture().await;
    for _ in 0..5 {
        f.w.start(f.workflow, &json!({})).await;
    }
    let service = service(&f);
    let everything = Selector::filter(Filter::default()).expect("selector");

    let capped = service
        .preview(&f.w.admin, f.w.namespace.id(), &everything, Some(2))
        .await
        .expect("preview");
    let all = service
        .preview(&f.w.admin, f.w.namespace.id(), &everything, None)
        .await
        .expect("preview");

    assert_eq!(
        (capped.matched, capped.capped),
        (3, true),
        "counted up to maxItems + 1"
    );
    assert_eq!((all.matched, all.capped), (5, false));
    assert_eq!(all.sample.len(), 5);
    assert!(all.sample.windows(2).all(|w| w[0] < w[1]), "in order of id");
    let by_status = Selector::filter(Filter {
        statuses: vec![RunStatus::Succeeded],
        ..Filter::default()
    })
    .expect("selector");
    let none = service
        .preview(&f.w.admin, f.w.namespace.id(), &by_status, None)
        .await
        .expect("preview");
    assert_eq!(none.matched, 0);
    let by_ids = Selector::run_ids(vec![all.sample[0], Uuid::new_v4()]).expect("selector");
    let found = service
        .preview(&f.w.admin, f.w.namespace.id(), &by_ids, None)
        .await
        .expect("preview");
    assert_eq!(
        found.sample,
        vec![all.sample[0]],
        "an id nobody has matches nothing"
    );
    assert_eq!(jobs_stored(&f).await, 0, "a preview changes nothing");

    let viewer = f.w.member("previewer", Role::Viewer).await;
    assert!(matches!(
        service
            .preview(&viewer, f.w.namespace.id(), &everything, None)
            .await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        service
            .preview(&f.w.other_admin, f.w.namespace.id(), &everything, None)
            .await,
        Err(AppError::NotFound)
    ));
}

// --- the guards of the tables ------------------------------------------------------------------------

#[tokio::test]
async fn what_a_job_is_never_changes_and_a_finished_job_stays_finished() {
    let f = fixture().await;
    let job = create(&f, cancel(), "guarded-1").await.expect("create");
    for (statement, why) in [
        (
            "UPDATE batch_jobs SET reason = 'a different justification' WHERE id = $1",
            "the justification",
        ),
        (
            "UPDATE batch_jobs SET max_items = 5 WHERE id = $1",
            "the limit",
        ),
        (
            "UPDATE batch_jobs SET action = 'PAUSE' WHERE id = $1",
            "the action",
        ),
    ] {
        let refused = sqlx::query(statement)
            .bind(job.id)
            .execute(&f.w.db.pool)
            .await;
        assert!(refused.is_err(), "{why} never changes");
    }
    sqlx::query("UPDATE batch_jobs SET state = 'RUNNING', started_at = now() WHERE id = $1")
        .bind(job.id)
        .execute(&f.w.db.pool)
        .await
        .expect("progress changes");
    sqlx::query("UPDATE batch_jobs SET state = 'COMPLETED', finished_at = now() WHERE id = $1")
        .bind(job.id)
        .execute(&f.w.db.pool)
        .await
        .expect("it finishes");
    let reopened =
        sqlx::query("UPDATE batch_jobs SET state = 'RUNNING', finished_at = NULL WHERE id = $1")
            .bind(job.id)
            .execute(&f.w.db.pool)
            .await;
    assert!(reopened.is_err(), "a finished job stays finished");
    let purged = sqlx::query("DELETE FROM batch_jobs WHERE id = $1 AND state = 'RUNNING'")
        .bind(job.id)
        .execute(&f.w.db.pool)
        .await
        .expect("nothing to delete while it is not running");
    assert_eq!(purged.rows_affected(), 0);
}

#[tokio::test]
async fn the_runtime_role_may_not_delete_a_job_or_an_item() {
    let f = fixture().await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app')")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("role");
    if !exists {
        return;
    }
    let (jobs, items): (bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege('sagat_app', 'batch_jobs', 'DELETE'), \
                has_table_privilege('sagat_app', 'batch_items', 'DELETE')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("privileges");
    assert!(!jobs && !items, "retention runs through a bounded function");
}

const BEFORE_BATCH: i64 = 20_261_231_000_000;

#[tokio::test]
async fn a_rollback_is_refused_while_a_job_is_kept_and_applying_again_brings_the_tables_back() {
    let f = fixture().await;
    let job = create(&f, cancel(), "kept-job-1").await.expect("create");

    let refused = crate::migration::undo_to(&f.w.db, BEFORE_BATCH)
        .await
        .expect_err("a job is evidence of what was done");
    assert!(
        format!("{refused:?}").contains("retained") || format!("{refused:?}").contains("55000"),
        "{refused:?}"
    );

    // an operator who accepts the loss clears the jobs first
    let mut tx = f.w.db.pool.begin().await.expect("begin");
    for statement in [
        "UPDATE batch_jobs SET state = 'COMPLETED', finished_at = now()",
        "DELETE FROM batch_jobs",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    tx.commit().await.expect("the drain commits");
    crate::migration::undo_to(&f.w.db, BEFORE_BATCH)
        .await
        .expect("rollback with no job left");
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_name IN ('batch_jobs', 'batch_items')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("tables");
    assert_eq!(tables, 0);

    sagat_infra_postgres::migrate(&f.w.db.pool)
        .await
        .expect("forward again");
    assert_eq!(jobs_stored(&f).await, 0, "{}", job.id);
}
