//! Catch-up after an outage (`T018-029`, `FR-018-008`, `SC-018-003`, US01 scenario 5): the times that
//! fell due while the platform was away are acted on under `catchupMode` and the ceiling of one
//! recovery, summarized when they are too old or too many, and never one run per missed time. A
//! recovery is planned once and its limits hold across the steps that carry it out, however many
//! there are and wherever the process that takes them restarts. Real PostgreSQL, a real run service,
//! a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::clock::Clock;
use sagat_application::schedule_firing::Fired;
use sagat_domain::schedules::Ceilings;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

use crate::firing::{Fixture, outcomes_of, t0};
use crate::support::schedules::fixture_with;

fn minutely() -> Value {
    json!({"intervals": [{"everyMs": 60_000}]})
}

/// Ceilings with a recovery of at most `acts` actions.
fn recovery_of(acts: u32) -> Ceilings {
    Ceilings {
        catchup_actions: acts,
        ..Ceilings::default()
    }
}

/// The minute `n` after the start of the schedule.
fn minute(n: i64) -> OffsetDateTime {
    t0() + Duration::minutes(n)
}

async fn nominal_times_of_runs(
    f: &Fixture,
    schedule: &sagat_application::schedules::ScheduleRecord,
) -> Vec<OffsetDateTime> {
    sqlx::query_scalar(
        "SELECT schedule_nominal_at FROM runs WHERE schedule_id = $1 ORDER BY schedule_nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("nominal times")
}

/// `(first, count, last)` of the summary rows of `outcome`, oldest first.
async fn summaries(
    f: &Fixture,
    schedule: &sagat_application::schedules::ScheduleRecord,
    outcome: &str,
) -> Vec<(OffsetDateTime, i32, OffsetDateTime)> {
    sqlx::query_as(
        "SELECT nominal_at, summary_count, summary_last_at FROM schedule_actions \
         WHERE schedule_id = $1 AND outcome = $2 ORDER BY nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .bind(outcome)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("summaries")
}

// --- the ceiling of one recovery ------------------------------------------------------------------

#[tokio::test]
async fn a_backlog_longer_than_the_ceiling_in_mode_all_acts_on_the_ceiling_and_summarizes_the_rest()
{
    let f = fixture_with(t0(), recovery_of(5)).await;
    let schedule = f
        .create(
            "backlog-all",
            &minutely(),
            &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    // the platform was away: minutes 1 to 20 are owed
    f.clock
        .advance(Duration::minutes(20) + Duration::seconds(30));

    f.drain(&schedule).await;

    assert_eq!(
        nominal_times_of_runs(&f, &schedule).await,
        (1..=5).map(minute).collect::<Vec<_>>(),
        "the first five, in order, and no more"
    );
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("COALESCED", 1), ("STARTED", 5)])
    );
    assert_eq!(
        summaries(&f, &schedule, "COALESCED").await,
        vec![(minute(6), 15, minute(20))],
        "one summary for the fifteen the recovery did not act on"
    );
    assert_eq!(f.read(&schedule).await.next_action_at, Some(minute(21)));
    assert_eq!(f.drain(&schedule).await, 0, "and nothing is left to do");
}

#[tokio::test]
async fn a_backlog_in_mode_one_acts_on_the_earliest_time_and_summarizes_all_the_others() {
    let f = fixture_with(t0(), recovery_of(5)).await;
    let schedule = f
        .create(
            "backlog-one",
            &minutely(),
            &json!({"catchupMode": "ONE", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock
        .advance(Duration::minutes(20) + Duration::seconds(30));

    f.drain(&schedule).await;

    assert_eq!(
        nominal_times_of_runs(&f, &schedule).await,
        vec![minute(1)],
        "one run for the whole outage, however many times were owed"
    );
    assert_eq!(
        summaries(&f, &schedule, "COALESCED").await,
        vec![(minute(2), 19, minute(20))]
    );
    assert_eq!(f.read(&schedule).await.next_action_at, Some(minute(21)));
}

#[tokio::test]
async fn the_limits_of_a_recovery_are_kept_between_its_steps() {
    let f = fixture_with(t0(), recovery_of(5)).await;
    let schedule = f
        .create(
            "between-steps",
            &minutely(),
            &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock
        .advance(Duration::minutes(20) + Duration::seconds(30));
    let outage_ends = f.clock.now();

    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    let (until, acts_left): (Option<OffsetDateTime>, Option<i32>) =
        sqlx::query_as("SELECT recovery_until, recovery_acts FROM schedules WHERE id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("recovery");
    assert_eq!(
        (until, acts_left),
        (Some(outage_ends), Some(3)),
        "two of five actions are used and the recovery remembers when its times end"
    );

    // times that fall due while the recovery is carried out belong to the next one
    f.clock.advance(Duration::minutes(10));
    for _ in 0..3 {
        assert_eq!(
            f.fire(&schedule).await,
            Fired::Acted,
            "the other three actions"
        );
    }
    assert_eq!(
        nominal_times_of_runs(&f, &schedule).await,
        (1..=5).map(minute).collect::<Vec<_>>(),
        "the fifth action ends the recovery"
    );
    assert_eq!(
        summaries(&f, &schedule, "COALESCED").await,
        vec![(minute(6), 15, minute(20))],
        "with the summary of the times the recovery began with, not the ten that fell due since"
    );
    assert_eq!(f.read(&schedule).await.next_action_at, Some(minute(21)));
    let (until, acts_left): (Option<OffsetDateTime>, Option<i32>) =
        sqlx::query_as("SELECT recovery_until, recovery_acts FROM schedules WHERE id = $1")
            .bind(schedule.id.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("recovery");
    assert_eq!(
        (until, acts_left),
        (None, None),
        "a finished recovery is forgotten"
    );
}

// --- a long outage is summarized in a few steps -------------------------------------------------

#[tokio::test]
async fn an_outage_much_longer_than_the_window_is_summarized_in_a_few_steps() {
    let f = fixture_with(t0(), Ceilings::default()).await;
    let schedule = f
        .create(
            "long-outage",
            &minutely(),
            &json!({"catchupMode": "ONE", "catchupWindowMs": 86_400_000, "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    // 20,000 minutes (nearly two weeks): 18,560 of them are older than the one-day window
    f.clock
        .advance(Duration::minutes(20_000) + Duration::seconds(30));

    let steps = f.drain(&schedule).await;

    assert!(
        steps <= 6,
        "{steps} steps: a summary takes up to 10,000 times, not a hundred"
    );
    let missed = summaries(&f, &schedule, "MISSED_CATCHUP_WINDOW").await;
    assert_eq!(
        missed.iter().map(|(_, n, _)| i64::from(*n)).sum::<i64>(),
        18_560,
        "every time older than the window is counted once"
    );
    assert_eq!(missed.first().map(|m| m.0), Some(minute(1)));
    assert_eq!(missed.last().map(|m| m.2), Some(minute(18_560)));
    assert_eq!(
        nominal_times_of_runs(&f, &schedule).await,
        vec![minute(18_561)],
        "one run for what is inside the window"
    );
    assert_eq!(
        summaries(&f, &schedule, "COALESCED").await,
        vec![(minute(18_562), 1_439, minute(20_000))],
        "and one summary for the rest of it, counted past the slice a step plans over"
    );
    assert_eq!(f.read(&schedule).await.next_action_at, Some(minute(20_001)));
}

// --- US01 scenario 5: a time that cannot start stays owed until the window ends ------------------

#[tokio::test]
async fn a_time_that_cannot_start_stays_owed_until_the_window_ends_and_is_then_missed() {
    let f = fixture_with(t0(), Ceilings::default()).await;
    let schedule = f
        .create(
            "owed",
            &json!({"intervals": [{"everyMs": 3_600_000}]}),
            &json!({"catchupWindowMs": 5_400_000, "catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 0, 'admin')",
    )
    .bind(schedule.tenant_id.as_uuid())
    .bind(schedule.namespace_id.as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("no room");

    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    assert!(
        matches!(f.fire(&schedule).await, Fired::Refused(_)),
        "13:00 cannot start"
    );
    assert!(f.runs(&schedule).await.is_empty());

    // 14:35: 13:00 is older than ninety minutes and is given up; 14:00 is inside the window and stays owed
    f.clock.advance(Duration::minutes(94));
    f.drain(&schedule).await;
    assert!(f.runs(&schedule).await.is_empty(), "still no room");
    assert_eq!(
        summaries(&f, &schedule, "MISSED_CATCHUP_WINDOW").await,
        vec![(minute(60), 1, minute(60))],
        "the first time was given up with a summary"
    );
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        Some(minute(120)),
        "and the second is the one still owed"
    );

    sqlx::query("DELETE FROM quota_limits WHERE tenant_id = $1 AND resource = 'active_runs'")
        .bind(schedule.tenant_id.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("room again");
    f.clock.advance(Duration::minutes(6));
    f.drain(&schedule).await;
    assert_eq!(
        nominal_times_of_runs(&f, &schedule).await,
        vec![minute(120)],
        "the owed time starts under its own key once there is room"
    );
}

#[tokio::test]
async fn a_workflow_without_a_published_version_keeps_the_time_owed_with_a_backoff() {
    let f = fixture_with(t0(), Ceilings::default()).await;
    let draft =
        f.w.create_draft(crate::support::engine::chain("not-published-yet"))
            .await;
    let schedule = f
        .create(
            "no-version",
            &json!({"intervals": [{"everyMs": 3_600_000}]}),
            &json!({}),
            Some(json!({"workflowId": draft.to_string()})),
        )
        .await;

    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    assert!(matches!(f.fire(&schedule).await, Fired::Refused(_)));
    let owed = f.read(&schedule).await;
    assert_eq!(
        owed.next_action_at, schedule.next_action_at,
        "the time is still owed"
    );
    assert!(
        owed.retry_at.is_some_and(|t| t > f.clock.now()),
        "and it is tried again later, not on every round"
    );
    assert_eq!(f.fire(&schedule).await, Fired::Idle, "not before the retry");

    f.w.publish_new_version(draft, crate::support::engine::chain("not-published-yet"))
        .await;
    f.clock.advance(Duration::minutes(6));
    f.drain(&schedule).await;
    assert_eq!(
        nominal_times_of_runs(&f, &schedule).await,
        vec![minute(60)],
        "once a version is published the time starts, once"
    );
}

// --- the migration that remembers a recovery ------------------------------------------------------

const BEFORE_RECOVERY: i64 = 20_261_230_000_200;

async fn recovery_columns(f: &Fixture) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_name = 'schedules' AND column_name LIKE 'recovery\\_%'",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("columns")
}

#[tokio::test]
async fn a_rollback_is_refused_while_a_schedule_is_in_the_middle_of_a_recovery() {
    let f = fixture_with(t0(), recovery_of(5)).await;
    let schedule = f
        .create(
            "rollback",
            &minutely(),
            &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock
        .advance(Duration::minutes(20) + Duration::seconds(30));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(recovery_columns(&f).await, 2);

    let refused = crate::migration::undo_to(&f.w.db, BEFORE_RECOVERY)
        .await
        .expect_err("the older pass would act on the backlog again, past the ceiling");
    assert!(
        format!("{refused:?}").contains("recovery") || format!("{refused:?}").contains("55000"),
        "{refused:?}"
    );
    assert_eq!(recovery_columns(&f).await, 2, "nothing was dropped");

    // let the recovery finish; then there is nothing to lose
    f.drain(&schedule).await;
    crate::migration::undo_to(&f.w.db, BEFORE_RECOVERY)
        .await
        .expect("rollback with no recovery in flight");
    assert_eq!(recovery_columns(&f).await, 0);

    sagat_infra_postgres::migrate(&f.w.db.pool)
        .await
        .expect("forward again with the schedule present");
    assert_eq!(recovery_columns(&f).await, 2);
    assert_eq!(
        f.read(&schedule).await.recovery,
        None,
        "a schedule that exists is not given a recovery"
    );
}

#[tokio::test]
async fn a_recovery_is_whole_and_bounded() {
    let f = fixture_with(t0(), recovery_of(5)).await;
    let schedule = f.create("bounds", &minutely(), &json!({}), None).await;
    for (statement, why) in [
        (
            "UPDATE schedules SET recovery_until = now() WHERE id = $1",
            "an end without the actions that are left",
        ),
        (
            "UPDATE schedules SET recovery_until = now(), recovery_acts = -1 WHERE id = $1",
            "a negative count",
        ),
        (
            "UPDATE schedules SET recovery_until = now(), recovery_acts = 1001 WHERE id = $1",
            "more actions than the hard ceiling",
        ),
    ] {
        let refused = sqlx::query(statement)
            .bind(schedule.id.as_uuid())
            .execute(&f.w.db.pool)
            .await;
        assert!(refused.is_err(), "{why} is refused");
    }
}
