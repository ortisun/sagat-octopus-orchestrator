//! `CANCEL_OTHER` (`T018-030`, `FR-018-006`, `FR-018-007`): a new time that finds a run of the schedule
//! open waits, and the open run is owed an ordinary cancellation. The debt is recorded in the same
//! transaction as the waiting action, paid by the pass of the schedule's shard through the cancel
//! use case of the run service under the key `schedule:{id}:cancel:{nominal}:{run}`, and the waiting
//! action starts once the run is over. Real PostgreSQL, a real run service, a clock the test moves by
//! hand. The races and the crashes are in the fault suite.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::ScheduleRecord;
use sagat_domain::identity::Role;
use sagat_domain::schedules::Overlap;
use serde_json::json;
use time::Duration;
use uuid::Uuid;

use crate::firing::{Fixture, fixture, hourly, outcomes_of};

/// `(state, run, key)` of the obligations of the schedule, oldest first.
async fn obligations(f: &Fixture, schedule: &ScheduleRecord) -> Vec<(String, Uuid, String)> {
    sqlx::query_as(
        "SELECT state, run_id, idempotency_key FROM schedule_obligations \
         WHERE schedule_id = $1 ORDER BY nominal_at, run_id",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("obligations")
}

async fn status_of(f: &Fixture, run: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("status")
}

/// The audit records of cancellation requests of `run`: `(actor, reason)`.
async fn cancellations(f: &Fixture, run: Uuid) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'run.cancel_requested' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit")
}

/// A schedule with `CANCEL_OTHER` whose first time has started a run, and whose second has found it
/// open: the second waits and the run is owed a cancellation.
async fn waiting_behind_an_open_run(f: &Fixture, name: &str) -> (ScheduleRecord, Uuid) {
    let schedule = f
        .create(name, &hourly(), &json!({"overlap": "CANCEL_OTHER"}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "13:00 starts");
    let first = f.runs(&schedule).await[0];
    f.clock.advance(Duration::hours(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "14:00 waits");
    (schedule, first)
}

#[tokio::test]
async fn a_time_that_finds_a_run_open_waits_and_owes_it_a_cancellation_in_the_same_transaction() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "owes").await;

    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUFFERED", 1), ("STARTED", 1)])
    );
    let owed = obligations(&f, &schedule).await;
    assert_eq!(owed.len(), 1, "one open run, one debt");
    assert_eq!((owed[0].0.as_str(), owed[0].1), ("PENDING", first));
    assert!(
        owed[0]
            .2
            .starts_with(&format!("schedule:{}:cancel:", schedule.id)),
        "{}",
        owed[0].2
    );
    assert_eq!(
        status_of(&f, first).await,
        "RUNNING",
        "the cancellation is the next step, not this transaction's"
    );
    assert_eq!(f.read(&schedule).await.buffered, 1);
}

#[tokio::test]
async fn the_pass_pays_the_debt_through_the_ordinary_cancel_and_the_waiting_time_then_starts() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "pays").await;

    assert_eq!(f.fire(&schedule).await, Fired::Acted, "the debt is paid");
    let owed = obligations(&f, &schedule).await;
    assert_eq!(owed[0].0, "DONE");
    assert_eq!(status_of(&f, first).await, "CANCELLED");
    assert_eq!(
        cancellations(&f, first).await,
        vec![(
            format!("system:schedule:{}", schedule.id),
            Some("the overlap rule of the schedule is CANCEL_OTHER".to_owned())
        )],
        "one cancellation, by the schedule, with the reason and no payload"
    );

    f.drain(&schedule).await;
    let runs = f.runs(&schedule).await;
    assert_eq!(runs.len(), 2, "the time that waited has its run");
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("STARTED", 2)]),
        "the waiting action became the started one"
    );
    assert_eq!(f.read(&schedule).await.buffered, 0);
}

#[tokio::test]
async fn a_run_that_ended_by_itself_first_is_recorded_done_and_cancelled_by_nobody() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "self-completion").await;
    f.end(first, "SUCCEEDED").await;

    // the pass first forgets the run that ended, then pays the debt: nothing is left to cancel
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(f.fire(&schedule).await, Fired::Acted);

    assert_eq!(obligations(&f, &schedule).await[0].0, "DONE");
    assert_eq!(
        status_of(&f, first).await,
        "SUCCEEDED",
        "a finished run is not cancelled"
    );
    assert!(cancellations(&f, first).await.is_empty());
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 2);
}

#[tokio::test]
async fn every_open_run_is_owed_its_own_cancellation() {
    let f = fixture().await;
    // under ALLOW_ALL two runs are open at once; the policy is then changed to CANCEL_OTHER
    let schedule = f
        .create(
            "two-open",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    let open = f.runs(&schedule).await;
    assert_eq!(open.len(), 2);
    f.service
        .update(
            &f.w.admin,
            schedule.id,
            f.write_of(
                "two-open",
                &hourly(),
                &json!({"overlap": "CANCEL_OTHER"}),
                None,
            ),
            1,
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("the rule is now CANCEL_OTHER");
    f.clock.advance(Duration::hours(1));

    assert_eq!(f.fire(&schedule).await, Fired::Acted, "15:00 waits");
    let owed = obligations(&f, &schedule).await;
    assert_eq!(owed.len(), 2);

    f.drain(&schedule).await;

    for run in open {
        assert_eq!(status_of(&f, run).await, "CANCELLED");
    }
    assert!(
        obligations(&f, &schedule)
            .await
            .iter()
            .all(|o| o.0 == "DONE")
    );
    assert_eq!(
        f.runs(&schedule).await.len(),
        3,
        "and the time that waited has its run"
    );
}

#[tokio::test]
async fn trigger_now_with_the_rule_overridden_owes_the_open_run_a_cancellation_too() {
    let f = fixture().await;
    let schedule = f
        .create(
            "override",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    let first = f.runs(&schedule).await[0];

    let action = f
        .firing
        .trigger(
            &f.w.admin,
            schedule.id,
            &TriggerRequest {
                overlap: Some(Overlap::CancelOther),
                key: "cancel-other-override",
            },
            &sagat_application::identity::RequestContext::default(),
        )
        .await
        .expect("trigger");
    assert_eq!(action.outcome, "BUFFERED");
    let owed = obligations(&f, &schedule).await;
    assert_eq!((owed[0].0.as_str(), owed[0].1), ("PENDING", first));
}

// --- who may write such a schedule --------------------------------------------------------------

#[tokio::test]
async fn cancelling_runs_needs_run_operate_of_whoever_writes_the_rule() {
    let f = fixture().await;
    let developer = f.w.member("developer-cancels", Role::Developer).await;
    let refused = f
        .service
        .create(
            &developer,
            f.w.namespace.id(),
            f.write_of(
                "cancels",
                &hourly(),
                &json!({"overlap": "CANCEL_OTHER"}),
                None,
            ),
            &sagat_application::identity::RequestContext::default(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::Forbidden { .. })),
        "a developer cannot cancel runs one by one, so a schedule written by one may not: {refused:?}"
    );
    // the same developer may write a schedule with any other rule
    f.create_as(
        &developer,
        "does-not-cancel",
        &hourly(),
        &json!({"overlap": "SKIP"}),
        None,
    )
    .await;
    // and an administrator, who may cancel, may write this one
    f.create(
        "admin-cancels",
        &hourly(),
        &json!({"overlap": "CANCEL_OTHER"}),
        None,
    )
    .await;
}

#[tokio::test]
async fn an_authority_that_can_no_longer_operate_runs_pauses_a_cancel_other_schedule() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "demoted").await;
    // the authorizing admin becomes a developer: still able to start runs, no longer to cancel them
    sqlx::query("UPDATE memberships SET role = 'developer' WHERE tenant_id = $1 AND subject = $2")
        .bind(schedule.tenant_id.as_uuid())
        .bind(&schedule.authorized_by)
        .execute(&f.w.db.pool)
        .await
        .expect("demote");

    f.drain(&schedule).await;

    assert_eq!(
        status_of(&f, first).await,
        "RUNNING",
        "nothing is cancelled"
    );
    assert_eq!(
        f.read(&schedule).await.state,
        sagat_application::schedules::ScheduleState::Paused
    );
    assert!(
        f.outcomes(&schedule)
            .await
            .contains(&("AUTHORITY_REVOKED".to_owned(), 1))
    );
}
