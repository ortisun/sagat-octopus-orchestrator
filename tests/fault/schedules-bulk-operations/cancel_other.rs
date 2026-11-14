//! `CANCEL_OTHER` under failure (`T018-030`, `FR-018-006`, `FR-018-007`, plan rows "timeout vs signal"
//! and "failed compensation"): the cancellation a waiting action owes an open run is paid through the
//! ordinary cancel use case, and the first commit wins. A run that finishes by itself while the
//! schedule is about to cancel it is left as it ended; a cancel that committed before the debt was
//! recorded is not asked twice; a replica that lost its lease cannot record the debt as paid; a
//! cancelled run whose compensation fails keeps the new time waiting instead of starting it on top.
//! Real PostgreSQL, a real run service and a real worker finishing a real run.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{
    Fired, ScheduleFiring, ScheduledRun, ScheduledStart, ScheduledStarts,
};
use sagat_application::schedules::{ScheduleAdmission, ScheduleRecord, ScheduleState, ShardFence};
use sagat_application::shards::Lease;
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_domain::schedules::Ceilings;
use sagat_domain::sharding::Shard;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgIdentityStore, PgScheduleStore};
use serde_json::{Value, json};
use time::Duration;
use uuid::Uuid;

use crate::support::engine::World;
use crate::support::schedules::{Fixture, Starts, TestClock, fixture, hourly, outcomes_of};

const REASON: &str = "the overlap rule of the schedule is CANCEL_OTHER";

/// The cancellations asked of `run` in the audit trail: `(actor, reason)`.
async fn cancellations(f: &Fixture, run: Uuid) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'run.cancel_requested' AND resource_id = $1",
    )
    .bind(run.to_string())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit")
}

async fn debts(f: &Fixture, schedule: &ScheduleRecord) -> Vec<(String, Uuid, String)> {
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

/// A `CANCEL_OTHER` schedule whose 13:00 run is open and whose 14:00 waits, owing it a cancellation.
pub(crate) async fn waiting_behind_an_open_run(
    f: &Fixture,
    name: &str,
    policies: Value,
) -> (ScheduleRecord, Uuid) {
    waiting_behind_an_open_run_of(f, name, policies, None).await
}

/// [`waiting_behind_an_open_run`] for a schedule whose target is `target` (the published workflow of the fixture when none).
pub(crate) async fn waiting_behind_an_open_run_of(
    f: &Fixture,
    name: &str,
    policies: Value,
    target: Option<Value>,
) -> (ScheduleRecord, Uuid) {
    let schedule = f.create(name, &hourly(), &policies, target).await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "13:00 starts");
    let first = f.runs(&schedule).await[0];
    f.clock.advance(Duration::hours(1));
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "14:00 waits");
    assert_eq!(debts(f, &schedule).await.len(), 1);
    (schedule, first)
}

pub(crate) fn cancel_other() -> Value {
    json!({"overlap": "CANCEL_OTHER"})
}

/// Finishes the workflow of `run` (`a -> b`) the way a worker does, through the engine.
async fn finish_by_itself(w: &World) {
    let session = w.session(&["act.a", "act.b"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("a completes");
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("b completes");
}

/// The run service of the world, except that the run finishes by itself just before the schedule's
/// cancel reaches it: the run was open when the pass looked and is over when the cancel arrives.
#[derive(Clone)]
struct FinishesFirst {
    inner: Starts,
}

impl ScheduledStarts for FinishesFirst {
    async fn start_scheduled(
        &self,
        principal: &Principal,
        request: &ScheduledStart<'_>,
        admission: ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<ScheduledRun, AppError> {
        self.inner
            .start_scheduled(principal, request, admission, ctx)
            .await
    }

    async fn cancel_run(
        &self,
        principal: &Principal,
        run: Uuid,
        reason: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        finish_by_itself(&self.inner.0).await;
        self.inner
            .cancel_run(principal, run, reason, key, ctx)
            .await
    }
}

fn firing_whose_runs_finish_first(
    f: &Fixture,
) -> ScheduleFiring<PgScheduleStore, SpecCalendar, TestClock, PgIdentityStore, FinishesFirst> {
    ScheduleFiring::new(
        PgScheduleStore::new(f.w.db.pool.clone()),
        SpecCalendar,
        f.clock.clone(),
        PgIdentityStore::new(f.w.db.pool.clone()),
        FinishesFirst {
            inner: Starts(Arc::clone(&f.w)),
        },
        Ceilings::default(),
    )
}

// --- the first commit wins ----------------------------------------------------------------------

#[tokio::test]
async fn a_run_that_finishes_by_itself_just_before_the_cancel_is_left_as_it_ended() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "finishes-first", cancel_other()).await;
    let firing = firing_whose_runs_finish_first(&f);

    // the pass sees the run open, pays the debt, and the run is over when the cancel reaches it
    let fired = firing.fire(&f.due(&schedule), None).await.expect("fire");

    assert_eq!(fired, Fired::Acted, "the debt is paid, not an error");
    assert_eq!(debts(&f, &schedule).await[0].0, "DONE");
    assert_eq!(
        status_of(&f, first).await,
        "SUCCEEDED",
        "the run kept the outcome it reached first"
    );
    assert!(
        cancellations(&f, first).await.is_empty(),
        "and nobody recorded a cancellation of it"
    );
    f.drain(&schedule).await;
    assert_eq!(
        f.runs(&schedule).await.len(),
        2,
        "the time that waited starts"
    );
}

#[tokio::test]
async fn a_cancel_that_committed_before_the_debt_was_recorded_is_not_asked_twice() {
    let f = fixture().await;
    let (schedule, first) =
        waiting_behind_an_open_run(&f, "crash-after-cancel", cancel_other()).await;
    // the process that paid the debt died after the cancel committed and before it was recorded
    let key = debts(&f, &schedule).await[0].2.clone();
    let as_schedule = Principal::new(
        schedule.tenant_id,
        Subject::schedule(schedule.id.as_uuid()),
        Role::Admin,
    );
    Starts(Arc::clone(&f.w))
        .cancel_run(
            &as_schedule,
            first,
            REASON,
            &key,
            &RequestContext::default(),
        )
        .await
        .expect("the cancel that committed");
    assert_eq!(debts(&f, &schedule).await[0].0, "PENDING");

    f.drain(&schedule).await;

    assert_eq!(debts(&f, &schedule).await[0].0, "DONE");
    assert_eq!(status_of(&f, first).await, "CANCELLED");
    assert_eq!(
        cancellations(&f, first).await.len(),
        1,
        "the replay of the key changed nothing"
    );
    assert_eq!(f.runs(&schedule).await.len(), 2);
}

#[tokio::test]
async fn a_replica_that_lost_its_lease_cannot_record_the_debt_and_the_new_owner_pays_it_once() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "stale-lease", cancel_other()).await;
    let shard = Shard::of(schedule.id.as_uuid());
    sqlx::query(
        "UPDATE scheduler_shards SET owner = 'replica-new', generation = 7, \
         expires_at = clock_timestamp() + interval '1 hour' WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .execute(&f.w.db.pool)
    .await
    .expect("the shard has a new owner");
    let stale = ShardFence {
        owner: "replica-old".to_owned(),
        lease: Lease {
            shard,
            generation: 6,
        },
    };
    let current = ShardFence {
        owner: "replica-new".to_owned(),
        lease: Lease {
            shard,
            generation: 7,
        },
    };

    let refused = f
        .firing
        .fire(&f.due(&schedule), Some(stale))
        .await
        .expect("fire");

    assert!(matches!(refused, Fired::Refused(_)), "{refused:?}");
    assert_eq!(
        debts(&f, &schedule).await[0].0,
        "PENDING",
        "a replica that does not hold the shard records nothing"
    );
    for _ in 0..10 {
        if f.firing
            .fire(&f.due(&schedule), Some(current.clone()))
            .await
            .expect("fire")
            == Fired::Idle
        {
            break;
        }
    }
    assert_eq!(debts(&f, &schedule).await[0].0, "DONE");
    assert_eq!(status_of(&f, first).await, "CANCELLED");
    assert_eq!(
        cancellations(&f, first).await.len(),
        1,
        "the stale replica and the new owner cancelled the run once between them"
    );
    assert_eq!(f.runs(&schedule).await.len(), 2);
}

// --- failed compensation ---------------------------------------------------------------------------

#[tokio::test]
async fn a_cancelled_run_that_waits_for_review_keeps_the_new_time_waiting() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(&f, "manual-review", cancel_other()).await;
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "the debt is paid");
    assert_eq!(cancellations(&f, first).await.len(), 1);
    // the compensation of the cancelled run failed and it waits for an operator: it is not over
    sqlx::query("UPDATE runs SET status = 'MANUAL_REVIEW', finished_at = NULL WHERE id = $1")
        .bind(first)
        .execute(&f.w.db.pool)
        .await
        .expect("the run needs review");

    f.clock.advance(Duration::minutes(10));
    f.drain(&schedule).await;

    assert_eq!(
        f.runs(&schedule).await.len(),
        1,
        "no second run on top of it"
    );
    assert_eq!(f.read(&schedule).await.buffered, 1, "the time still waits");
    assert_eq!(
        cancellations(&f, first).await.len(),
        1,
        "and the run is not cancelled again"
    );

    // an operator settles it; the waiting time starts
    sqlx::query(
        "UPDATE runs SET status = 'COMPENSATION_FAILED', finished_at = now() WHERE id = $1",
    )
    .bind(first)
    .execute(&f.w.db.pool)
    .await
    .expect("settled");
    // the pass looked at the run a moment ago and looks again in five seconds
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 2);
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 2)]));
}

#[tokio::test]
async fn a_cancelled_run_whose_compensation_failed_pauses_a_schedule_that_pauses_on_failure() {
    let f = fixture().await;
    let (schedule, first) = waiting_behind_an_open_run(
        &f,
        "pause-on-failure",
        json!({"overlap": "CANCEL_OTHER", "pauseOnFailure": true}),
    )
    .await;
    assert_eq!(f.fire(&schedule).await, Fired::Acted, "the debt is paid");
    sqlx::query(
        "UPDATE runs SET status = 'COMPENSATION_FAILED', finished_at = now() WHERE id = $1",
    )
    .bind(first)
    .execute(&f.w.db.pool)
    .await
    .expect("compensation failed");

    f.drain(&schedule).await;

    let after = f.read(&schedule).await;
    assert_eq!(after.state, ScheduleState::Paused);
    assert_eq!(after.note.as_deref(), Some("a run of the schedule failed"));
    assert_eq!(
        f.runs(&schedule).await.len(),
        1,
        "the waiting time does not start while paused"
    );
    assert_eq!(f.read(&schedule).await.buffered, 1);
}
