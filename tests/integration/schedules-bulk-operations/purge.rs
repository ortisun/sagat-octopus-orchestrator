//! Retention of the actions of a schedule (`T018-028`, `FR-018-013`): an action is kept while it is
//! among the last 1,000 of its schedule or younger than 30 days, a waiting action is never deleted,
//! and a purge that visits schedules in turn reaches every one of them however many others have
//! nothing to purge. The purge is one bounded call of a function the runtime role may run, since the
//! role may not delete from the table. Real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::maintenance::MaintenanceStore;
use sagat_application::schedules::ScheduleRecord;
use serde_json::json;

use crate::firing::{Fixture, fixture, hourly};

/// `count` skipped actions of the schedule, one every day from `first` days ago to `last` days ago.
async fn seed(f: &Fixture, schedule: &ScheduleRecord, first: i32, last: i32, outcome: &str) {
    sqlx::query(
        "INSERT INTO schedule_actions \
         (tenant_id, schedule_id, origin, nominal_at, kind, revision, outcome, decided_at) \
         SELECT $1, $2, 'SPEC', now() - g * interval '1 day', 'SCHEDULED', 1, $5, now() - g * interval '1 day' \
         FROM generate_series($3::int, $4::int) g",
    )
    .bind(schedule.tenant_id.as_uuid())
    .bind(schedule.id.as_uuid())
    .bind(first)
    .bind(last)
    .bind(outcome)
    .execute(&f.w.db.pool)
    .await
    .expect("seed the actions");
}

async fn actions_of(f: &Fixture, schedule: &ScheduleRecord) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1")
        .bind(schedule.id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("count")
}

/// How many actions of the schedule are more than `days` days old.
async fn older_than(f: &Fixture, schedule: &ScheduleRecord, days: i32) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM schedule_actions WHERE schedule_id = $1 \
         AND decided_at < now() - $2 * interval '1 day'",
    )
    .bind(schedule.id.as_uuid())
    .bind(days)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count")
}

async fn purge(f: &Fixture, schedules: u32, rows: u32) -> u64 {
    f.w.store
        .purge_schedule_actions(schedules, rows)
        .await
        .expect("purge")
}

#[tokio::test]
async fn an_action_beyond_the_last_thousand_and_older_than_thirty_days_is_purged() {
    let f = fixture().await;
    let schedule = f.create("old-actions", &hourly(), &json!({}), None).await;
    // one a day, 1,300 days back: the newest thousand stay, and so does everything younger than 30 days
    seed(&f, &schedule, 1, 1_300, "SKIPPED_OVERLAP").await;

    let removed = purge(&f, 100, 10_000).await;

    assert_eq!(removed, 300);
    assert_eq!(actions_of(&f, &schedule).await, 1_000);
    assert_eq!(
        older_than(&f, &schedule, 1_001).await,
        0,
        "the ones that went are the oldest"
    );
    assert_eq!(
        purge(&f, 100, 10_000).await,
        0,
        "and a second purge finds nothing more"
    );
}

#[tokio::test]
async fn a_schedule_with_few_actions_keeps_them_however_old() {
    let f = fixture().await;
    let schedule = f.create("few", &hourly(), &json!({}), None).await;
    seed(&f, &schedule, 400, 409, "SKIPPED_OVERLAP").await;

    assert_eq!(purge(&f, 100, 10_000).await, 0);
    assert_eq!(actions_of(&f, &schedule).await, 10);
}

#[tokio::test]
async fn a_waiting_action_is_never_purged() {
    let f = fixture().await;
    let schedule = f.create("waiting", &hourly(), &json!({}), None).await;
    seed(&f, &schedule, 1, 1_000, "SKIPPED_OVERLAP").await;
    seed(&f, &schedule, 1_100, 1_110, "SKIPPED_OVERLAP").await;
    seed(&f, &schedule, 2_000, 2_000, "BUFFERED").await;

    let removed = purge(&f, 100, 10_000).await;

    assert_eq!(removed, 11, "the eleven final ones past the last thousand");
    let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'BUFFERED'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count");
    assert_eq!(waiting, 1, "the action that still waits is kept");
}

#[tokio::test]
async fn schedules_are_visited_in_turn_so_none_is_starved_by_those_with_nothing_to_purge() {
    let f = fixture().await;
    let quiet = f.create("quiet", &hourly(), &json!({}), None).await;
    let busy = f.create("busy", &hourly(), &json!({}), None).await;
    seed(&f, &quiet, 400, 409, "SKIPPED_OVERLAP").await;
    seed(&f, &busy, 1, 1_100, "SKIPPED_OVERLAP").await;

    // one schedule per call: the schedule with nothing to purge must not be looked at again before
    // the other has been looked at once
    purge(&f, 1, 10_000).await;
    purge(&f, 1, 10_000).await;

    assert_eq!(
        actions_of(&f, &busy).await,
        1_000,
        "the busy one was reached"
    );
    assert_eq!(actions_of(&f, &quiet).await, 10);
}

#[tokio::test]
async fn a_purge_deletes_at_most_the_rows_it_is_given_and_goes_on_where_it_stopped() {
    let f = fixture().await;
    let schedule = f.create("bounded", &hourly(), &json!({}), None).await;
    seed(&f, &schedule, 1, 1_300, "SKIPPED_OVERLAP").await;

    assert_eq!(purge(&f, 10, 120).await, 120);
    assert_eq!(actions_of(&f, &schedule).await, 1_180);
    assert_eq!(purge(&f, 10, 120).await, 120);
    assert_eq!(purge(&f, 10, 120).await, 60, "the rest");
    assert_eq!(actions_of(&f, &schedule).await, 1_000);
}

#[tokio::test]
async fn the_runtime_role_may_run_the_purge_and_may_not_delete_an_action_itself() {
    let f = fixture().await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app')")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("role");
    if !exists {
        return;
    }
    let (may_run, may_delete): (bool, bool) = sqlx::query_as(
        "SELECT has_function_privilege('sagat_app', 'schedule_purge_actions(integer, integer)', 'EXECUTE'), \
                has_table_privilege('sagat_app', 'schedule_actions', 'DELETE')",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("privileges");
    assert!(may_run, "the role runs the bounded purge");
    assert!(!may_delete, "and has no DELETE of its own on the table");
}

const BEFORE_PURGE: i64 = 20_261_230_001_000;

#[tokio::test]
async fn a_rollback_drops_the_purge_and_applying_again_brings_it_back() {
    let f = fixture().await;
    let schedule = f.create("rollback", &hourly(), &json!({}), None).await;
    seed(&f, &schedule, 1, 1_050, "SKIPPED_OVERLAP").await;
    assert_eq!(purge(&f, 100, 10_000).await, 50);

    crate::migration::undo_to(&f.w.db, BEFORE_PURGE)
        .await
        .expect("nothing a schedule did is lost by the rollback");
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_proc WHERE proname = 'schedule_purge_actions'")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("function");
    assert_eq!(left, 0, "the function is gone");
    assert_eq!(
        actions_of(&f, &schedule).await,
        1_000,
        "and the actions are as they were"
    );

    sagat_infra_postgres::migrate(&f.w.db.pool)
        .await
        .expect("forward again");
    assert_eq!(
        purge(&f, 100, 10_000).await,
        0,
        "the purge works again, with nothing left to do"
    );
}
