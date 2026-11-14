//! The decisions of a schedule, listed (`T018-028`, `FR-018-013`): newest first, in pages that end
//! where the next one begins even when several decisions were made in the same instant, optionally
//! of one outcome, and never across tenants. Real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::schedules::{ActionCursor, ActionView, ScheduleRecord};
use sagat_domain::identity::Role;
use sagat_domain::schedules::Outcome;
use serde_json::json;
use time::{Duration, OffsetDateTime};

use crate::firing::{Fixture, fixture, hourly};

/// A decision of `schedule`: `decided` and `nominal` hours before `base`, to the microsecond, so
/// that two decisions made `decided` hours ago are made in the very same instant.
async fn decide(
    f: &Fixture,
    base: OffsetDateTime,
    schedule: &ScheduleRecord,
    origin: &str,
    nominal: i32,
    decided: i32,
    outcome: &str,
) {
    let kind = if origin == "MANUAL" {
        "TRIGGERED"
    } else {
        "SCHEDULED"
    };
    sqlx::query(
        "INSERT INTO schedule_actions \
         (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
         VALUES ($1, $2, $3, $4, $5, 1, $6, $7)",
    )
    .bind(schedule.tenant_id.as_uuid())
    .bind(schedule.id.as_uuid())
    .bind(origin)
    .bind(base - Duration::hours(i64::from(nominal)))
    .bind(kind)
    .bind(outcome)
    .bind(base - Duration::hours(i64::from(decided)))
    .execute(&f.w.db.pool)
    .await
    .expect("a decision");
}

/// Every page of the listing, `size` at a time.
async fn pages(
    f: &Fixture,
    schedule: &ScheduleRecord,
    outcome: Option<Outcome>,
    size: u32,
) -> Vec<ActionView> {
    let mut all = Vec::new();
    let mut after = None;
    for _ in 0..50 {
        let page = f
            .service
            .actions(&f.w.admin, schedule.id, outcome, after, size + 1)
            .await
            .expect("a page");
        let more = page.len() > size as usize;
        let page: Vec<ActionView> = page.into_iter().take(size as usize).collect();
        after = page.last().map(ActionCursor::after);
        all.extend(page);
        if !more {
            return all;
        }
    }
    panic!("the listing never ended");
}

#[tokio::test]
async fn the_decisions_come_newest_first_in_pages_that_neither_skip_nor_repeat_one() {
    let f = fixture().await;
    let schedule = f.create("busy", &hourly(), &json!({}), None).await;
    let base = OffsetDateTime::now_utc();
    // (origin, nominal hours ago, decided hours ago): three decisions in the same instant, whose
    // key tells them apart, among older ones. The gap between the two is what names each below.
    for (origin, nominal, decided) in [
        ("SPEC", 10, 1),
        ("SPEC", 20, 2),
        ("MANUAL", 12, 2),
        ("SPEC", 13, 2),
        ("SPEC", 15, 3),
        ("SPEC", 17, 4),
    ] {
        decide(
            &f,
            base,
            &schedule,
            origin,
            nominal,
            decided,
            "SKIPPED_OVERLAP",
        )
        .await;
    }

    let all = pages(&f, &schedule, None, 100).await;
    let named: Vec<(&str, i64)> = all
        .iter()
        .map(|action| {
            (
                action.origin.as_str(),
                (action.decided_at - action.nominal_at).whole_hours(),
            )
        })
        .collect();
    assert_eq!(
        named,
        vec![
            ("SPEC", 9),
            // the tie of the instant: by origin, then by nominal time, the later first
            ("SPEC", 11),
            ("SPEC", 18),
            ("MANUAL", 10),
            ("SPEC", 12),
            ("SPEC", 13),
        ]
    );
    for size in [1, 2, 3, 4] {
        assert_eq!(
            pages(&f, &schedule, None, size).await,
            all,
            "{size} at a time gives the listing as a whole: none skipped, none twice"
        );
    }
}

#[tokio::test]
async fn a_page_can_be_asked_for_one_outcome() {
    let f = fixture().await;
    let schedule = f.create("filtered", &hourly(), &json!({}), None).await;
    let base = OffsetDateTime::now_utc();
    for hours in 1..=6 {
        let outcome = if hours % 2 == 0 {
            "SKIPPED_OVERLAP"
        } else {
            "DROPPED_BUFFER_FULL"
        };
        decide(&f, base, &schedule, "SPEC", 10 + hours, hours, outcome).await;
    }

    let skipped = pages(&f, &schedule, Some(Outcome::SkippedOverlap), 2).await;
    assert_eq!(skipped.len(), 3);
    assert!(
        skipped
            .iter()
            .all(|action| action.outcome == "SKIPPED_OVERLAP")
    );
    let dropped = pages(&f, &schedule, Some(Outcome::DroppedBufferFull), 2).await;
    assert_eq!(dropped.len(), 3);
    assert!(
        pages(&f, &schedule, Some(Outcome::Started), 2)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn the_decisions_of_a_schedule_are_not_seen_by_another_tenant_and_are_read_by_a_viewer() {
    let f = fixture().await;
    let schedule = f.create("private", &hourly(), &json!({}), None).await;
    decide(
        &f,
        OffsetDateTime::now_utc(),
        &schedule,
        "SPEC",
        10,
        1,
        "SKIPPED_OVERLAP",
    )
    .await;

    let foreign = f
        .service
        .actions(&f.w.other_admin, schedule.id, None, None, 10)
        .await;
    assert!(matches!(foreign, Err(AppError::NotFound)), "{foreign:?}");

    let viewer = f.w.member("watcher", Role::Viewer).await;
    let seen = f
        .service
        .actions(&viewer, schedule.id, None, None, 10)
        .await
        .expect("a viewer reads the decisions");
    assert_eq!(seen.len(), 1);
}
