//! How late the schedule and batch passes are, as the monitor samples it (`T018-017`, `FR-018-044`):
//! the signal the engine's replicas scale on and `SagatSchedulerLate` watches. A pass is late by the
//! time its oldest item has been due, and an item the pass would not take yet is not due. Real
//! PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::operations::MonitorStore;
use sagat_domain::batch::Action;
use serde_json::json;
use uuid::Uuid;

use crate::batch_executor::{create, everything, live, write};
use crate::firing::{Fixture, fixture, hourly};

/// Puts the next time of a schedule `seconds` ago (a negative number: ahead), by the clock of the
/// database, which the monitor reads. The fixture's own clock is a fixed day.
async fn age_schedule(f: &Fixture, id: Uuid, seconds: i32) {
    sqlx::query(
        "UPDATE schedules SET next_actual_at = clock_timestamp() - make_interval(secs => $2) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(f64::from(seconds))
    .execute(&f.w.db.pool)
    .await
    .expect("age the schedule");
}

async fn age_job(f: &Fixture, id: Uuid, seconds: i32) {
    sqlx::query(
        "UPDATE batch_jobs SET next_slot_at = clock_timestamp() - make_interval(secs => $2) \
         WHERE id = $1",
    )
    .bind(id)
    .bind(f64::from(seconds))
    .execute(&f.w.db.pool)
    .await
    .expect("age the job");
}

#[tokio::test]
async fn a_schedule_is_late_by_how_long_it_has_been_due_and_not_otherwise() {
    let f = fixture().await;
    let quiet = f.w.store.sample().await.expect("sample").overdue;
    assert_eq!(
        (quiet.schedules, quiet.batches),
        (0.0, 0.0),
        "nothing is due"
    );

    let schedule = f.create("lagging", &hourly(), &json!({}), None).await;
    let id = schedule.id.as_uuid();
    age_schedule(&f, id, -3_600).await;
    let waiting = f.w.store.sample().await.expect("sample").overdue;
    assert_eq!(waiting.schedules, 0.0, "its first time is still to come");

    age_schedule(&f, id, 600).await;
    let late = f.w.store.sample().await.expect("sample").overdue;
    assert!(
        (599.0..700.0).contains(&late.schedules),
        "ten minutes due: {}",
        late.schedules
    );
    assert_eq!(late.batches, 0.0, "the other pass is not late");
    assert_eq!(late.timers, 0.0, "nor are the passes of the runs");

    // A paused schedule acts at no time, however long ago the last one was due.
    sqlx::query("UPDATE schedules SET state = 'PAUSED' WHERE id = $1")
        .bind(id)
        .execute(&f.w.db.pool)
        .await
        .expect("pause");
    let paused = f.w.store.sample().await.expect("sample").overdue;
    assert_eq!(paused.schedules, 0.0, "a paused schedule is not late");
}

#[tokio::test]
async fn a_batch_job_is_late_when_the_pass_would_take_it_and_has_not() {
    let f = fixture().await;
    live(&f);
    let job = create(&f, write(Action::Cancel, everything()), "lag-job-is-due").await;
    let id = job.id;
    age_job(&f, id, 45).await;
    let late = f.w.store.sample().await.expect("sample");
    assert!(
        (44.0..120.0).contains(&late.overdue.batches),
        "a job due 45 s ago: {}",
        late.overdue.batches
    );
    assert_eq!(late.batch_jobs_active, 1, "the job is active");
    assert_eq!(late.overdue.schedules, 0.0, "the other pass is not late");

    // A job that waits for a transaction older than itself to end is not due: the pass would not
    // take it, and no replica added would change that. Its wait is not the scheduler's lag.
    let mut holder = f.w.db.pool.begin().await.expect("a transaction");
    let _held: sqlx::postgres::PgRow = sqlx::query("SELECT pg_current_xact_id()::text")
        .fetch_one(&mut *holder)
        .await
        .expect("take an identity");
    let blocked = create(
        &f,
        write(Action::Cancel, everything()),
        "lag-job-held-by-a-transaction",
    )
    .await;
    age_job(&f, blocked.id, 3_000).await;
    let held = f.w.store.sample().await.expect("sample");
    assert!(
        held.overdue.batches < 120.0,
        "a job held back by an open transaction is not the lag: {}",
        held.overdue.batches
    );
    assert_eq!(held.batch_jobs_active, 2, "but it is an active job");
    holder.rollback().await.expect("end the transaction");

    // Once the job is over, nothing is late and nothing is active.
    sqlx::query("UPDATE batch_jobs SET state = 'CANCELLED', finished_at = clock_timestamp()")
        .execute(&f.w.db.pool)
        .await
        .expect("end the jobs");
    let done = f.w.store.sample().await.expect("sample");
    assert_eq!(done.overdue.batches, 0.0);
    assert_eq!(done.batch_jobs_active, 0);
}
