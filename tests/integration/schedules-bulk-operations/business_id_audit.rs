//! Business-id conflicts are audited (`FR-018-044`, `G2086`, OD-2026-11-13-3): a `business_id.conflict` record is
//! written where the refusal is decided, the `409` of a person's start and the decision of a schedule that finds its id
//! held, with the closed reason of the refusal and never the identifier itself (it is customer text and may be
//! sensitive). A held id that answers with its holder (`USE_EXISTING`) is no conflict and writes none. Real PostgreSQL,
//! the real run service and firing.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::schedule_firing::Fired;
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use serde_json::json;
use time::Duration;

use crate::business_ids::{business, cancel, key, start};
use crate::firing::{Fixture, fixture, hourly};

/// `(actor, resource type, resource id, reason)` of every `business_id.conflict` record, oldest first.
async fn conflicts(f: &Fixture) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT actor, resource_type, resource_id, reason FROM audit_records \
         WHERE action = 'business_id.conflict' ORDER BY created_at, id",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("the audit trail")
}

/// How many audit records, of any action, carry `text` in a column a reader is shown.
async fn records_naming(f: &Fixture, text: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records \
         WHERE strpos(coalesce(reason, ''), $1) > 0 OR strpos(resource_id, $1) > 0 \
            OR strpos(actor, $1) > 0",
    )
    .bind(text)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("the audit trail")
}

#[tokio::test]
async fn a_start_refused_because_an_open_execution_holds_the_id_is_audited_without_the_id() {
    let f = fixture().await;
    let id = "invoice-for-patient-4711";
    start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the first start binds the id");
    assert!(
        conflicts(&f).await.is_empty(),
        "a start that binds is no conflict"
    );

    let refused = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await;
    assert!(
        matches!(
            refused,
            Err(AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_IN_USE",
                ..
            })
        ),
        "{refused:?}"
    );

    assert_eq!(
        conflicts(&f).await,
        [(
            f.w.admin.subject().as_str().to_owned(),
            "workflow".to_owned(),
            f.workflow.to_string(),
            Some("BUSINESS_ID_IN_USE".to_owned()),
        )],
        "one record, by the person, on the workflow they asked to start, with the closed reason"
    );
    assert_eq!(
        records_naming(&f, id).await,
        0,
        "no audit record carries the identifier"
    );
}

#[tokio::test]
async fn a_start_the_reuse_policy_refuses_is_audited_with_its_own_reason() {
    let f = fixture().await;
    let id = "order-reuse-1";
    let first = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::RejectDuplicate),
        &key(),
    )
    .await
    .expect("the first start")
    .run
    .info
    .run_id;
    cancel(&f, first).await;

    let refused = start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::RejectDuplicate),
        &key(),
    )
    .await;
    assert!(
        matches!(
            refused,
            Err(AppError::BusinessIdConflict {
                reason: "BUSINESS_ID_REUSE_REJECTED",
                ..
            })
        ),
        "{refused:?}"
    );
    let reasons: Vec<Option<String>> = conflicts(&f).await.into_iter().map(|c| c.3).collect();
    assert_eq!(reasons, [Some("BUSINESS_ID_REUSE_REJECTED".to_owned())]);
    assert_eq!(records_naming(&f, id).await, 0);
}

#[tokio::test]
async fn a_held_id_that_answers_with_its_holder_is_no_conflict_and_is_not_audited_as_one() {
    let f = fixture().await;
    let id = "order-existing-1";
    start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the first start");
    let existing = start(
        &f,
        business(id, ConflictPolicy::UseExisting, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("the holder answers");
    assert!(existing.existing);
    assert!(conflicts(&f).await.is_empty());
}

#[tokio::test]
async fn a_schedule_that_finds_its_id_held_audits_the_conflict_with_its_decision() {
    let f = fixture().await;
    let schedule = f
        .try_create(f.write_of(
            "audited",
            &hourly(),
            &json!({}),
            Some(json!({
                "workflowId": f.workflow.to_string(),
                "businessIdPrefix": "secret-order",
                "businessIdConflictPolicy": "FAIL",
                "businessIdReusePolicy": "ALLOW_DUPLICATE"
            })),
        ))
        .await
        .expect("create");
    // a person holds the id the first action takes
    let id = "secret-order-2026-10-28T13:00:00Z";
    start(
        &f,
        business(id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
        &key(),
    )
    .await
    .expect("a person holds the id");

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(f.fire(&schedule).await, Fired::Idle);

    let decided: Vec<String> =
        sqlx::query_scalar("SELECT outcome FROM schedule_actions WHERE schedule_id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_all(&f.w.db.pool)
            .await
            .expect("the decisions");
    assert_eq!(decided, ["BUSINESS_ID_CONFLICT"]);
    assert_eq!(
        conflicts(&f).await,
        [(
            format!("system:schedule:{}", schedule.id),
            "schedule".to_owned(),
            schedule.id.to_string(),
            Some("BUSINESS_ID_IN_USE".to_owned()),
        )],
        "one record, by the schedule, on the schedule, with the closed reason: the decision is the schedule's, \
         and the person's start that held the id first was no conflict"
    );
    assert_eq!(records_naming(&f, "secret-order").await, 0);
}
