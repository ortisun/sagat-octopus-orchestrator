//! Trigger-now (`T018-024`, `FR-018-010`): one `TRIGGERED` action at the time of the request, also
//! while the schedule is paused, deduplicated by its `Idempotency-Key`, never counted against the
//! remaining actions and never the same identity as a scheduled time. Real PostgreSQL, a real run
//! service, a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{ActionView, ScheduleRecord, ScheduleState};
use sagat_domain::identity::{Principal, Role, Subject, TenantId};
use sagat_domain::schedules::{ActionKind, Note, Origin, Overlap};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::firing::{Fixture, fixture, hourly};

async fn trigger(
    f: &Fixture,
    schedule: &ScheduleRecord,
    overlap: Option<Overlap>,
    key: &str,
) -> Result<ActionView, AppError> {
    f.firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest { overlap, key },
            &RequestContext::default(),
        )
        .await
}

async fn pause(f: &Fixture, schedule: &ScheduleRecord) {
    f.service
        .pause(
            &f.w.admin,
            schedule.id,
            Note::new("freeze").expect("note"),
            &RequestContext::default(),
        )
        .await
        .expect("pause");
}

async fn set(f: &Fixture, schedule: &ScheduleRecord, assignment: &str) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE schedules SET {assignment} WHERE id = $1"
    )))
    .bind(schedule.id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("set");
}

#[tokio::test]
async fn trigger_now_starts_a_manual_run_even_while_the_schedule_is_paused() {
    let f = fixture().await;
    let schedule = f.create("manual", &hourly(), &json!({}), None).await;
    pause(&f, &schedule).await;
    let before = f.read(&schedule).await;
    assert_eq!(before.state, ScheduleState::Paused);

    let action = trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("trigger");
    assert_eq!(
        (action.kind, action.origin, action.outcome.as_str()),
        (ActionKind::Triggered, Origin::Manual, "STARTED")
    );
    assert_eq!(action.nominal_at, f.clock.now(), "the time of the request");
    let run = action.run_id.expect("a run");
    assert_eq!(f.runs(&schedule).await, vec![run]);

    let (kind, nominal): (String, OffsetDateTime) =
        sqlx::query_as("SELECT schedule_action_kind, schedule_nominal_at FROM runs WHERE id = $1")
            .bind(run)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("provenance");
    assert_eq!((kind.as_str(), nominal), ("TRIGGERED", action.nominal_at));
    let actor: String = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE action = 'run.started' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, format!("system:schedule:{}", schedule.id));

    let after = f.read(&schedule).await;
    assert_eq!(after.state, ScheduleState::Paused, "it stays paused");
    assert_eq!(after.tracked_runs, vec![run], "its run is tracked");
    assert_eq!(after.next_action_at, None, "paused: no scheduled time");

    // and the decision of a request that starts nothing is recorded on a paused schedule too
    f.clock.advance(Duration::seconds(1));
    let skipped = trigger(&f, &schedule, None, "trigger-key-0002")
        .await
        .expect("trigger");
    assert_eq!(skipped.outcome, "SKIPPED_OVERLAP");
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Paused);
}

#[tokio::test]
async fn trigger_now_does_not_use_up_a_remaining_action() {
    let f = fixture().await;
    let schedule = f.create("limited", &hourly(), &json!({}), None).await;
    set(&f, &schedule, "remaining_actions = 3").await;
    trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("trigger");
    assert_eq!(f.read(&schedule).await.remaining_actions, Some(3));

    // and it works when none is left
    set(&f, &schedule, "remaining_actions = 0").await;
    f.clock.advance(Duration::seconds(1));
    let action = trigger(&f, &schedule, Some(Overlap::AllowAll), "trigger-key-0002")
        .await
        .expect("trigger");
    assert_eq!(action.outcome, "STARTED");
    assert_eq!(f.read(&schedule).await.remaining_actions, Some(0));
}

#[tokio::test]
async fn the_overlap_rule_of_the_schedule_applies_unless_the_request_overrides_it() {
    let f = fixture().await;
    let schedule = f.create("busy", &hourly(), &json!({}), None).await;
    let first = trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("idle: it starts");
    assert_eq!(first.outcome, "STARTED");

    // SKIP: the run is open
    f.clock.advance(Duration::seconds(1));
    let skipped = trigger(&f, &schedule, None, "trigger-key-0002")
        .await
        .expect("trigger");
    assert_eq!(
        (skipped.outcome.as_str(), skipped.run_id),
        ("SKIPPED_OVERLAP", None)
    );

    // overridden to ALLOW_ALL: it starts beside the open run
    f.clock.advance(Duration::seconds(1));
    let allowed = trigger(&f, &schedule, Some(Overlap::AllowAll), "trigger-key-0003")
        .await
        .expect("trigger");
    assert_eq!(allowed.outcome, "STARTED");
    assert_eq!(f.runs(&schedule).await.len(), 2);

    // overridden to BUFFER_ONE: it waits
    f.clock.advance(Duration::seconds(1));
    let waiting = trigger(&f, &schedule, Some(Overlap::BufferOne), "trigger-key-0004")
        .await
        .expect("trigger");
    assert_eq!(waiting.outcome, "BUFFERED");
    assert_eq!(f.read(&schedule).await.buffered, 1);
    assert_eq!(
        f.read(&schedule).await.policies.overlap(),
        Overlap::Skip,
        "the override is for this action only"
    );
}

#[tokio::test]
async fn a_trigger_and_a_scheduled_time_at_the_same_instant_are_two_actions() {
    let f = fixture().await;
    let schedule = f
        .create("both", &hourly(), &json!({"overlap": "ALLOW_ALL"}), None)
        .await;
    f.clock.advance(Duration::hours(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    let action = trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("trigger");
    assert_eq!(action.outcome, "STARTED");
    assert_eq!(f.runs(&schedule).await.len(), 2);
    let origins: Vec<(String, OffsetDateTime)> = sqlx::query_as(
        "SELECT origin, nominal_at FROM schedule_actions WHERE schedule_id = $1 ORDER BY origin",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("actions");
    assert_eq!(
        origins.iter().map(|(o, _)| o.as_str()).collect::<Vec<_>>(),
        ["MANUAL", "SPEC"]
    );
    assert_eq!(
        origins[0].1, origins[1].1,
        "the same instant, two identities"
    );
}

#[tokio::test]
async fn the_same_key_answers_with_the_same_action_and_a_changed_request_is_refused() {
    let f = fixture().await;
    let schedule = f.create("replayed", &hourly(), &json!({}), None).await;
    let first = trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("trigger");
    f.clock.advance(Duration::seconds(30));
    let again = trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("a replay");
    assert_eq!(again, first, "the answer of the first request");
    assert_eq!(f.runs(&schedule).await.len(), 1, "no second run");

    let changed = trigger(&f, &schedule, Some(Overlap::AllowAll), "trigger-key-0001")
        .await
        .expect_err("the same key for another request");
    assert!(
        matches!(changed, AppError::IdempotencyKeyReused),
        "{changed:?}"
    );

    // a request that started no run is replayed too
    let skipped = trigger(&f, &schedule, None, "trigger-key-0002")
        .await
        .expect("skipped");
    assert_eq!(skipped.outcome, "SKIPPED_OVERLAP");
    f.clock.advance(Duration::seconds(30));
    let skipped_again = trigger(&f, &schedule, None, "trigger-key-0002")
        .await
        .expect("a replay");
    assert_eq!(skipped_again, skipped);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("rows");
    assert_eq!(rows, 2, "one row per request, not per attempt");

    // the key belongs to the schedule: another schedule is another request
    let other = f.create("another", &hourly(), &json!({}), None).await;
    let elsewhere = trigger(&f, &other, None, "trigger-key-0001")
        .await
        .expect("trigger");
    assert_eq!(elsewhere.outcome, "STARTED");
    assert_eq!(f.runs(&other).await.len(), 1);
}

#[tokio::test]
async fn two_requests_with_one_key_at_once_make_one_action() {
    let f = fixture().await;
    let schedule = f.create("racing", &hourly(), &json!({}), None).await;
    let f = Arc::new(f);
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let f = Arc::clone(&f);
        let id = schedule.id;
        tasks.push(tokio::spawn(async move {
            f.firing
                .trigger(
                    &f.w.admin,
                    id,
                    &TriggerRequest {
                        overlap: None,
                        key: "trigger-key-0001",
                    },
                    &RequestContext::default(),
                )
                .await
        }));
    }
    let mut answers = Vec::new();
    for task in tasks {
        answers.push(task.await.expect("task").expect("trigger"));
    }
    assert_eq!(answers[0], answers[1], "both are told the same thing");
    assert_eq!(f.runs(&schedule).await.len(), 1, "one run");
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("rows");
    assert_eq!(rows, 1, "one action");
}

#[tokio::test]
async fn who_may_trigger_and_what_can_be_triggered() {
    let f = fixture().await;
    let schedule = f.create("guarded", &hourly(), &json!({}), None).await;

    let viewer = Principal::new(
        f.w.admin.tenant_id(),
        Subject::parse("vera").expect("subject"),
        Role::Viewer,
    );
    let denied = f
        .firing
        .trigger(
            &viewer,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "trigger-key-0001",
            },
            &RequestContext::default(),
        )
        .await
        .expect_err("a viewer reads");
    assert!(matches!(denied, AppError::Forbidden { .. }), "{denied:?}");

    let stranger = Principal::new(
        TenantId::generate(),
        Subject::parse("mallory").expect("subject"),
        Role::Admin,
    );
    let hidden = f
        .firing
        .trigger(
            &stranger,
            schedule.id,
            &TriggerRequest {
                overlap: None,
                key: "trigger-key-0001",
            },
            &RequestContext::default(),
        )
        .await
        .expect_err("another tenant's schedule");
    assert!(matches!(hidden, AppError::NotFound), "{hidden:?}");

    assert!(f.runs(&schedule).await.is_empty(), "nothing started");

    // a schedule a cron trigger owns answers to the legacy pass until the handover
    sqlx::query("ALTER TABLE schedules DISABLE TRIGGER schedules_identity")
        .execute(&f.w.db.pool)
        .await
        .expect("the test turns the identity guard off to make a trigger-owned schedule");
    set(
        &f,
        &schedule,
        &format!("origin = 'TRIGGER', trigger_id = '{}'", Uuid::now_v7()),
    )
    .await;
    let owned = trigger(&f, &schedule, None, "trigger-key-0002")
        .await
        .expect_err("trigger-owned");
    assert!(
        matches!(
            owned,
            AppError::ScheduleConflict {
                reason: "SCHEDULE_TRIGGER_HANDOVER_PENDING"
            }
        ),
        "{owned:?}"
    );

    // and a deleted schedule is gone
    let gone = f.create("gone", &hourly(), &json!({}), None).await;
    f.service
        .delete(&f.w.admin, gone.id, None, &RequestContext::default())
        .await
        .expect("delete");
    let missing = trigger(&f, &gone, None, "trigger-key-0003")
        .await
        .expect_err("deleted");
    assert!(matches!(missing, AppError::NotFound), "{missing:?}");
}

#[tokio::test]
async fn a_trigger_leaves_a_time_that_is_owed_alone() {
    let f = fixture().await;
    let schedule = f
        .create("owed", &hourly(), &json!({"overlap": "ALLOW_ALL"}), None)
        .await;
    set(&f, &schedule, "retry_at = '2026-10-28T12:30:00Z'").await;
    let before = f.read(&schedule).await;
    assert!(before.retry_at.is_some());

    trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("a start");
    assert_eq!(f.read(&schedule).await.retry_at, before.retry_at);

    // a skipped request leaves it alone too
    f.clock.advance(Duration::seconds(1));
    let skipped = trigger(&f, &schedule, Some(Overlap::Skip), "trigger-key-0002")
        .await
        .expect("a skip");
    assert_eq!(skipped.outcome, "SKIPPED_OVERLAP");
    assert_eq!(f.read(&schedule).await.retry_at, before.retry_at);
}

#[tokio::test]
async fn a_waiting_trigger_starts_when_the_run_ends_and_uses_up_nothing() {
    let f = fixture().await;
    let schedule = f
        .create(
            "patient",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            None,
        )
        .await;
    set(&f, &schedule, "remaining_actions = 3").await;
    let first = trigger(&f, &schedule, None, "trigger-key-0001")
        .await
        .expect("trigger");
    f.clock.advance(Duration::seconds(1));
    let waiting = trigger(&f, &schedule, None, "trigger-key-0002")
        .await
        .expect("trigger");
    assert_eq!(waiting.outcome, "BUFFERED");

    f.end(first.run_id.expect("run"), "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(10));
    let steps = f.drain(&schedule).await;
    assert!(
        steps >= 2,
        "the run is forgotten, then the waiting one starts"
    );
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 2);
    let after = f.read(&schedule).await;
    assert_eq!(
        after.remaining_actions,
        Some(3),
        "a manual action never uses one up, waiting or not"
    );
    assert_eq!(after.buffered, 0);
}
