//! Backfill requests (`T018-025`, `FR-018-011`): what is accepted, what is refused and how many may run
//! at once. The steps that act on the times are `backfill_pass.rs`. Real PostgreSQL, a clock the
//! test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::Fired;
use sagat_application::schedules::{BackfillRecord, BackfillRequest, BackfillState, DueBackfill};
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_domain::schedules::Overlap;
use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::firing::{Fixture, fixture, hourly, t0};

fn five_hours() -> BackfillRequest {
    BackfillRequest {
        start: t0() - Duration::hours(5),
        end: t0(),
        overlap: None,
    }
}

async fn request(
    f: &Fixture,
    schedule: sagat_domain::schedules::ScheduleId,
    request: &BackfillRequest,
) -> Result<BackfillRecord, AppError> {
    f.service
        .create_backfill(&f.w.admin, schedule, request, &RequestContext::default())
        .await
}

/// One step of a backfill.
async fn step(f: &Fixture, backfill: &BackfillRecord) -> Fired {
    f.firing
        .backfill_step(
            &DueBackfill {
                tenant_id: backfill.tenant_id,
                schedule: backfill.schedule,
                id: backfill.id,
            },
            None,
        )
        .await
        .expect("a step")
}

/// Steps until the backfill has nothing more to do now; returns how many steps acted.
async fn drain(f: &Fixture, backfill: &BackfillRecord) -> u32 {
    let mut acted = 0;
    for _ in 0..50 {
        match step(f, backfill).await {
            Fired::Acted => acted += 1,
            Fired::Idle | Fired::Refused(_) => return acted,
        }
    }
    panic!("the backfill never became idle");
}

async fn read(f: &Fixture, backfill: &BackfillRecord) -> BackfillRecord {
    f.service
        .backfills(&f.w.admin, backfill.schedule)
        .await
        .expect("list")
        .into_iter()
        .find(|b| b.id == backfill.id)
        .expect("the backfill")
}

fn counters(pairs: &[(&str, u64)]) -> std::collections::BTreeMap<String, u64> {
    pairs.iter().map(|(k, n)| ((*k).to_owned(), *n)).collect()
}

fn reason(error: &AppError) -> &'static str {
    match error {
        AppError::InvalidSchedule { reason, .. } => reason,
        other => panic!("not a validation error: {other:?}"),
    }
}

#[tokio::test]
async fn a_backfill_is_recorded_with_its_range_and_the_buffering_default() {
    let f = fixture().await;
    let schedule = f.create("past", &hourly(), &json!({}), None).await;
    let backfill = request(&f, schedule.id, &five_hours())
        .await
        .expect("a backfill");
    assert_eq!(backfill.state, BackfillState::Pending);
    assert_eq!(
        backfill.overlap,
        Overlap::BufferAll,
        "the default of a backfill"
    );
    assert_eq!(
        (backfill.start, backfill.end),
        (t0() - Duration::hours(5), t0())
    );
    assert_eq!((backfill.cursor, backfill.finished_at), (None, None));
    assert!(backfill.counters.is_empty());
    assert_eq!(backfill.requested_by, "alice");

    let listed = f
        .service
        .backfills(&f.w.admin, schedule.id)
        .await
        .expect("list");
    assert_eq!(listed, vec![backfill.clone()]);
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'schedule.backfill_requested' AND resource_id = $1",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1);

    let own = request(
        &f,
        schedule.id,
        &BackfillRequest {
            overlap: Some(Overlap::Skip),
            ..five_hours()
        },
    )
    .await
    .expect_err("one at a time");
    assert!(matches!(own, AppError::QuotaExceeded(_)), "{own:?}");
}

#[tokio::test]
async fn a_range_that_makes_no_sense_or_is_too_big_is_refused_with_a_reason() {
    let f = fixture().await;
    let schedule = f.create("bounds", &hourly(), &json!({}), None).await;
    for (range, expected) in [
        (
            BackfillRequest {
                start: t0(),
                end: t0() - Duration::hours(1),
                overlap: None,
            },
            "BACKFILL_RANGE_INVALID",
        ),
        (
            BackfillRequest {
                start: t0() - Duration::hours(1),
                end: t0() - Duration::hours(1),
                overlap: None,
            },
            "BACKFILL_RANGE_INVALID",
        ),
        (
            BackfillRequest {
                start: t0() - Duration::hours(1),
                end: t0() + Duration::seconds(1),
                overlap: None,
            },
            "BACKFILL_RANGE_INVALID",
        ),
    ] {
        let refused = request(&f, schedule.id, &range).await.expect_err("refused");
        assert_eq!(reason(&refused), expected, "{range:?}");
    }
    // a thousand hourly times fit; a thousand and one do not
    let thousand = BackfillRequest {
        start: t0() - Duration::hours(1_000),
        end: t0(),
        overlap: None,
    };
    let fits = request(&f, schedule.id, &thousand)
        .await
        .expect("1,000 times");
    f.service
        .cancel_backfill(&f.w.admin, schedule.id, fits.id, &RequestContext::default())
        .await
        .expect("cancel");
    let too_many = request(
        &f,
        schedule.id,
        &BackfillRequest {
            start: t0() - Duration::hours(1_001),
            ..thousand
        },
    )
    .await
    .expect_err("1,001 times");
    assert_eq!(reason(&too_many), "BACKFILL_LIMIT");
}

#[tokio::test]
async fn a_schedule_runs_one_backfill_at_a_time_and_a_namespace_four() {
    let f = fixture().await;
    let first = f.create("one", &hourly(), &json!({}), None).await;
    let held = request(&f, first.id, &five_hours()).await.expect("first");
    let second = request(&f, first.id, &five_hours())
        .await
        .expect_err("a second on the same schedule");
    assert!(matches!(second, AppError::QuotaExceeded(_)), "{second:?}");
    f.service
        .cancel_backfill(&f.w.admin, first.id, held.id, &RequestContext::default())
        .await
        .expect("cancel");
    request(&f, first.id, &five_hours())
        .await
        .expect("room again once it is stopped");

    // the namespace allows four at once: the first schedule holds one
    for n in 0..3 {
        let other = f
            .create(&format!("other-{n}"), &hourly(), &json!({}), None)
            .await;
        request(&f, other.id, &five_hours()).await.expect("room");
    }
    let last = f.create("last", &hourly(), &json!({}), None).await;
    let full = request(&f, last.id, &five_hours())
        .await
        .expect_err("the namespace is full");
    assert!(matches!(full, AppError::QuotaExceeded(_)), "{full:?}");
}

#[tokio::test]
async fn stopping_a_backfill_is_final_and_idempotent_and_belongs_to_its_schedule() {
    let f = fixture().await;
    let schedule = f.create("stop", &hourly(), &json!({}), None).await;
    let other = f.create("elsewhere", &hourly(), &json!({}), None).await;
    let backfill = request(&f, schedule.id, &five_hours())
        .await
        .expect("backfill");

    let ctx = RequestContext::default();
    let stopped = f
        .service
        .cancel_backfill(&f.w.admin, schedule.id, backfill.id, &ctx)
        .await
        .expect("cancel");
    assert_eq!(stopped.state, BackfillState::Cancelled);
    assert!(stopped.finished_at.is_some());
    let again = f
        .service
        .cancel_backfill(&f.w.admin, schedule.id, backfill.id, &ctx)
        .await
        .expect("a second cancel");
    assert_eq!(again, stopped, "stopping it twice changes nothing");

    for (schedule, backfill) in [(other.id, backfill.id), (schedule.id, Uuid::now_v7())] {
        let missing = f
            .service
            .cancel_backfill(&f.w.admin, schedule, backfill, &ctx)
            .await
            .expect_err("not found");
        assert!(matches!(missing, AppError::NotFound), "{missing:?}");
    }
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'schedule.backfill_cancelled' AND resource_id = $1",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1, "audited once, not once per attempt");
}

#[tokio::test]
async fn who_may_backfill_and_what_deleting_the_schedule_does_to_it() {
    let f = fixture().await;
    let schedule = f.create("guarded", &hourly(), &json!({}), None).await;
    let viewer = Principal::new(
        f.w.admin.tenant_id(),
        Subject::parse("vera").expect("subject"),
        Role::Viewer,
    );
    let ctx = RequestContext::default();
    let denied = f
        .service
        .create_backfill(&viewer, schedule.id, &five_hours(), &ctx)
        .await
        .expect_err("a viewer reads");
    assert!(matches!(denied, AppError::Forbidden { .. }), "{denied:?}");
    let backfill = request(&f, schedule.id, &five_hours())
        .await
        .expect("backfill");
    assert_eq!(
        f.service
            .backfills(&viewer, schedule.id)
            .await
            .expect("read"),
        vec![backfill.clone()],
        "a viewer reads them"
    );

    f.service
        .delete(&f.w.admin, schedule.id, None, &ctx)
        .await
        .expect("delete");
    let state: String = sqlx::query_scalar(
        "SELECT state FROM schedule_backfills WHERE schedule_id = $1 AND id = $2",
    )
    .bind(schedule.id.as_uuid())
    .bind(backfill.id)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("state");
    assert_eq!(state, "CANCELLED", "a deleted schedule backfills nothing");
    let gone = f
        .service
        .backfills(&f.w.admin, schedule.id)
        .await
        .expect_err("deleted");
    assert!(matches!(gone, AppError::NotFound), "{gone:?}");
}

fn allow_all() -> BackfillRequest {
    BackfillRequest {
        overlap: Some(Overlap::AllowAll),
        ..five_hours()
    }
}

#[tokio::test]
async fn a_backfill_acts_on_each_time_of_its_range_in_order_and_then_completes() {
    let f = fixture().await;
    let schedule = f.create("steps", &hourly(), &json!({}), None).await;
    let backfill = request(&f, schedule.id, &allow_all())
        .await
        .expect("backfill");

    assert_eq!(
        drain(&f, &backfill).await,
        6,
        "five times, then the step that finds none left"
    );
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 5);
    let provenance: Vec<(String, OffsetDateTime)> = sqlx::query_as(
        "SELECT schedule_action_kind, schedule_nominal_at FROM runs WHERE schedule_id = $1 ORDER BY created_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("provenance");
    assert_eq!(
        provenance,
        (7..=11)
            .map(|h| ("BACKFILL".to_owned(), t0() - Duration::hours(12 - h)))
            .collect::<Vec<_>>(),
        "in the order of the times, each a BACKFILL action"
    );
    let done = read(&f, &backfill).await;
    assert_eq!(done.state, BackfillState::Completed);
    assert_eq!(done.counters, counters(&[("STARTED", 5)]));
    assert_eq!(done.cursor, Some(t0() - Duration::hours(1)));
    assert!(done.finished_at.is_some());
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        schedule.next_action_at,
        "the schedule's own times are where they were"
    );
    assert_eq!(
        step(&f, &done).await,
        Fired::Idle,
        "a finished backfill is done"
    );
}

#[tokio::test]
async fn a_time_that_already_has_an_action_is_counted_not_acted_on_twice() {
    let f = fixture().await;
    let schedule = f
        .create("twice", &hourly(), &json!({"overlap": "ALLOW_ALL"}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "13:00 starts");

    let backfill = request(
        &f,
        schedule.id,
        &BackfillRequest {
            start: t0(),
            end: t0() + Duration::hours(1) + Duration::seconds(1),
            overlap: Some(Overlap::AllowAll),
        },
    )
    .await
    .expect("backfill over 12:00 and 13:00");
    drain(&f, &backfill).await;
    assert_eq!(
        f.runs(&schedule).await.len(),
        2,
        "12:00 started, 13:00 did not start again"
    );
    let done = read(&f, &backfill).await;
    assert_eq!(done.state, BackfillState::Completed);
    assert_eq!(
        done.counters,
        counters(&[("ALREADY_ACTED", 1), ("STARTED", 1)])
    );
}

#[tokio::test]
async fn a_stopped_backfill_acts_on_nothing_more_and_a_paused_schedule_does_not_stop_one() {
    let f = fixture().await;
    let schedule = f.create("stopped", &hourly(), &json!({}), None).await;
    f.service
        .pause(
            &f.w.admin,
            schedule.id,
            sagat_domain::schedules::Note::new("freeze").expect("note"),
            &RequestContext::default(),
        )
        .await
        .expect("pause");
    let backfill = request(&f, schedule.id, &allow_all())
        .await
        .expect("backfill");
    assert_eq!(step(&f, &backfill).await, Fired::Acted);
    assert_eq!(step(&f, &backfill).await, Fired::Acted);
    assert_eq!(
        f.runs(&schedule).await.len(),
        2,
        "it runs on a paused schedule"
    );

    f.service
        .cancel_backfill(
            &f.w.admin,
            schedule.id,
            backfill.id,
            &RequestContext::default(),
        )
        .await
        .expect("stop");
    assert_eq!(step(&f, &backfill).await, Fired::Idle);
    assert_eq!(
        f.runs(&schedule).await.len(),
        2,
        "what was acted on stays, nothing more"
    );
    let stopped = read(&f, &backfill).await;
    assert_eq!(stopped.state, BackfillState::Cancelled);
    assert_eq!(
        stopped.cursor,
        Some(t0() - Duration::hours(4)),
        "it got to 08:00"
    );
    assert_eq!(
        f.read(&schedule).await.state,
        sagat_application::schedules::ScheduleState::Paused
    );
}

#[tokio::test]
async fn a_backfill_whose_requester_lost_run_start_is_stopped_and_starts_nothing() {
    let f = fixture().await;
    let schedule = f.create("revoked", &hourly(), &json!({}), None).await;
    let backfill = request(&f, schedule.id, &allow_all())
        .await
        .expect("backfill");
    assert_eq!(step(&f, &backfill).await, Fired::Acted);
    sqlx::query(
        "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = 'alice'",
    )
    .bind(schedule.tenant_id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("demote");
    assert_eq!(
        step(&f, &backfill).await,
        Fired::Acted,
        "the stop is a step"
    );
    assert_eq!(
        f.runs(&schedule).await.len(),
        1,
        "no run after the revocation"
    );
    assert_eq!(read(&f, &backfill).await.state, BackfillState::Cancelled);
}

#[tokio::test]
async fn a_backfill_that_buffers_waits_for_room_instead_of_dropping_times() {
    let f = fixture().await;
    let schedule = f.create("patient", &hourly(), &json!({}), None).await;
    let backfill = request(
        &f,
        schedule.id,
        &BackfillRequest {
            overlap: Some(Overlap::BufferOne),
            ..five_hours()
        },
    )
    .await
    .expect("backfill");
    assert_eq!(step(&f, &backfill).await, Fired::Acted, "07:00 starts");
    assert_eq!(step(&f, &backfill).await, Fired::Acted, "08:00 waits");
    assert_eq!(
        step(&f, &backfill).await,
        Fired::Idle,
        "09:00 would be dropped: the backfill waits instead"
    );
    let waiting = read(&f, &backfill).await;
    assert_eq!(waiting.cursor, Some(t0() - Duration::hours(4)));
    assert_eq!(
        waiting.counters,
        counters(&[("BUFFERED", 1), ("STARTED", 1)])
    );
    let dropped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'DROPPED_BUFFER_FULL'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("dropped");
    assert_eq!(dropped, 0, "no time of a backfill is dropped");
}
