//! What a waiting action keeps for itself (`FR-018-012`, `D1042`, `D2190`, `G1017`): with `remainingActions`, a waiting
//! **scheduled** action holds one of the remaining actions, because it uses one up when it starts; a waiting trigger-now or
//! step of a backfill uses none up (`FR-018-012`: the count is decremented by `SCHEDULED` actions that start a run), so it
//! holds none. Found by the independent correctness review of the business-id slice: with the reservation counting every waiting
//! row, a waiting trigger-now made the next scheduled time "exhausted" (no row, no next time), and nothing ever gave the time back,
//! whether the waiting action then started or found its business id held: the schedule stayed `ACTIVE` with an action left and
//! no time to act on for ever.
//!
//! Real PostgreSQL, the real run service and firing, a clock the test moves by hand (`tests/support/schedules.rs`).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{Fired, TriggerRequest};
use sagat_application::schedules::{BackfillRequest, DueBackfill, ScheduleRecord, ScheduleState};
use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::schedules::{ActionKind, Overlap, RemainingActions};
use serde_json::{Value, json};
use time::Duration;
use uuid::Uuid;

use crate::business_ids::{business, holder_of, key, start};
use crate::firing::{Fixture, fixture, hourly, outcomes_of, t0};

const PREFIX: &str = "reserve";

/// A schedule with one remaining action and `BUFFER_ONE`, one run that is open, and one waiting action of `kind` (a trigger-now,
/// or the second time of a backfill), which uses no remaining action up.
struct Waiting {
    f: Fixture,
    schedule: ScheduleRecord,
    /// The run the waiting action waits for.
    open: Uuid,
    /// The business id the waiting action asks for when it is released.
    id: String,
}

fn target(f: &Fixture) -> Value {
    json!({
        "workflowId": f.workflow.to_string(),
        "businessIdPrefix": PREFIX,
        "businessIdConflictPolicy": "FAIL",
        "businessIdReusePolicy": "ALLOW_DUPLICATE"
    })
}

async fn waiting_manual(kind: ActionKind) -> Waiting {
    let f = fixture().await;
    let mut write = f.write_of(
        "manual-waits",
        &hourly(),
        &json!({"overlap": "BUFFER_ONE"}),
        Some(target(&f)),
    );
    write.remaining_actions = Some(RemainingActions::new(1).expect("remaining"));
    let schedule = f.try_create(write).await.expect("create");
    let ctx = RequestContext::default();
    let id = match kind {
        ActionKind::Triggered => {
            // 12:01:40.250: nothing is open, so it starts the run the next one waits for
            f.clock.set(t0() + Duration::milliseconds(100_250));
            let first = f
                .firing
                .trigger(
                    &f.w.admin,
                    schedule.id,
                    &TriggerRequest {
                        overlap: None,
                        key: "reservation-key-0001",
                    },
                    &ctx,
                )
                .await
                .expect("the trigger-now that starts the open run");
            assert_eq!(first.outcome, "STARTED");
            // 12:03:20.250: a run is open, so it waits (the id of the second it was asked in)
            f.clock.set(t0() + Duration::milliseconds(200_250));
            let second = f
                .firing
                .trigger(
                    &f.w.admin,
                    schedule.id,
                    &TriggerRequest {
                        overlap: None,
                        key: "reservation-key-0002",
                    },
                    &ctx,
                )
                .await
                .expect("the trigger-now that waits");
            assert_eq!(second.outcome, "BUFFERED");
            format!("{PREFIX}-2026-10-28T12:03:20Z")
        }
        ActionKind::Backfill => {
            let backfill = f
                .service
                .create_backfill(
                    &f.w.admin,
                    schedule.id,
                    &BackfillRequest {
                        start: t0() - Duration::hours(2),
                        end: t0(),
                        overlap: Some(Overlap::BufferOne),
                    },
                    &ctx,
                )
                .await
                .expect("backfill");
            let due = DueBackfill {
                tenant_id: backfill.tenant_id,
                schedule: backfill.schedule,
                id: backfill.id,
            };
            for what in ["10:00 starts", "11:00 waits"] {
                assert_eq!(
                    f.firing.backfill_step(&due, None).await.expect("step"),
                    Fired::Acted,
                    "{what}"
                );
            }
            format!("{PREFIX}-2026-10-28T11:00:00Z")
        }
        ActionKind::Scheduled => panic!("a scheduled action holds a remaining action: see D2182"),
    };
    let after = f.read(&schedule).await;
    assert_eq!(
        (after.buffered, after.remaining_actions),
        (1, Some(1)),
        "the premise: one action waits, the schedule still has its one remaining action"
    );
    let open = f.runs(&schedule).await[0];
    Waiting {
        f,
        schedule,
        open,
        id,
    }
}

impl Waiting {
    /// 13:00 falls due while the action waits.
    async fn the_next_time_falls_due(&self) {
        self.f
            .clock
            .set(t0() + Duration::hours(1) + Duration::seconds(1));
        self.f.drain(&self.schedule).await;
    }

    /// The open run ends and the waiting action is released.
    async fn the_open_run_ends(&self) {
        self.f.end(self.open, "SUCCEEDED").await;
        self.f.clock.advance(Duration::seconds(6));
        self.f.drain(&self.schedule).await;
    }

    /// A person takes the id the waiting action will ask for.
    async fn somebody_holds_the_id(&self) -> Uuid {
        start(
            &self.f,
            business(&self.id, ConflictPolicy::Fail, ReusePolicy::AllowDuplicate),
            &key(),
        )
        .await
        .expect("a person holds the id of the waiting action")
        .run
        .info
        .run_id
        .as_uuid()
    }
}

/// A time that falls due while a manual action waits is a decision like any other: the manual action keeps no remaining action,
/// so the schedule is not exhausted, and the buffer of `BUFFER_ONE` is full.
#[tokio::test]
async fn a_time_that_falls_due_while_a_trigger_now_waits_is_decided_not_passed_over() {
    let w = waiting_manual(ActionKind::Triggered).await;
    w.the_next_time_falls_due().await;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT kind, outcome, nominal_at::text FROM schedule_actions WHERE schedule_id = $1 ORDER BY decided_at, nominal_at",
    )
    .bind(w.schedule.id.as_uuid())
    .fetch_all(&w.f.w.db.pool)
    .await
    .expect("rows");
    let kinds: Vec<(&str, &str)> = rows
        .iter()
        .map(|(k, o, _)| (k.as_str(), o.as_str()))
        .collect();
    assert!(
        kinds.contains(&("SCHEDULED", "DROPPED_BUFFER_FULL")),
        "13:00 is dropped because the buffer is full, not passed over without a row: {kinds:?}"
    );
    let after = w.f.read(&w.schedule).await;
    assert_eq!(
        after.next_action_at,
        Some(t0() + Duration::hours(2)),
        "and the schedule goes on to 14:00"
    );
    assert_eq!(after.remaining_actions, Some(1));
}

async fn a_waiting_manual_action_that_starts_leaves_the_schedule_going(kind: ActionKind) {
    let w = waiting_manual(kind).await;
    w.the_next_time_falls_due().await;
    w.the_open_run_ends().await;
    let after = w.f.read(&w.schedule).await;
    assert_eq!(
        (
            after.state,
            after.remaining_actions,
            after.buffered,
            after.next_action_at
        ),
        (
            ScheduleState::Active,
            Some(1),
            0,
            Some(t0() + Duration::hours(2))
        ),
        "the waiting action used no remaining action up and the schedule has a next time: it is not stranded"
    );
    assert_eq!(
        w.f.outcomes(&w.schedule).await,
        outcomes_of(&[("DROPPED_BUFFER_FULL", 1), ("STARTED", 2)]),
        "the due time was decided, and the waiting action started"
    );
    assert_eq!(w.f.runs(&w.schedule).await.len(), 2);

    // the schedule goes on: its one remaining action is used by the next time
    let released = w.f.runs(&w.schedule).await[1];
    w.f.end(released, "SUCCEEDED").await;
    w.f.clock
        .set(t0() + Duration::hours(2) + Duration::seconds(1));
    w.f.drain(&w.schedule).await;
    let last = w.f.read(&w.schedule).await;
    assert_eq!(
        w.f.runs(&w.schedule).await.len(),
        3,
        "14:00 started its run"
    );
    assert_eq!(
        last.remaining_actions,
        Some(0),
        "the one remaining action is used up by it"
    );
}

async fn a_waiting_manual_action_whose_id_is_held_leaves_the_schedule_going(kind: ActionKind) {
    let w = waiting_manual(kind).await;
    let holder = w.somebody_holds_the_id().await;
    w.the_next_time_falls_due().await;
    w.the_open_run_ends().await;
    let after = w.f.read(&w.schedule).await;
    assert_eq!(
        (
            after.state,
            after.remaining_actions,
            after.buffered,
            after.next_action_at
        ),
        (
            ScheduleState::Active,
            Some(1),
            0,
            Some(t0() + Duration::hours(2))
        ),
        "the settled action used no remaining action up and the schedule has a next time: it is not stranded"
    );
    assert_eq!(
        w.f.outcomes(&w.schedule).await,
        outcomes_of(&[
            ("BUSINESS_ID_CONFLICT", 1),
            ("DROPPED_BUFFER_FULL", 1),
            ("STARTED", 1)
        ]),
        "the waiting action became the refused one, not a run"
    );
    assert_eq!(w.f.runs(&w.schedule).await.len(), 1, "no run for it");
    assert_eq!(holder_of(&w.f, &w.id).await, Some(holder));
}

#[tokio::test]
async fn a_waiting_trigger_now_that_starts_leaves_the_schedule_going() {
    a_waiting_manual_action_that_starts_leaves_the_schedule_going(ActionKind::Triggered).await;
}

#[tokio::test]
async fn a_waiting_backfill_step_that_starts_leaves_the_schedule_going() {
    a_waiting_manual_action_that_starts_leaves_the_schedule_going(ActionKind::Backfill).await;
}

#[tokio::test]
async fn a_waiting_trigger_now_whose_id_is_held_leaves_the_schedule_going() {
    a_waiting_manual_action_whose_id_is_held_leaves_the_schedule_going(ActionKind::Triggered).await;
}

#[tokio::test]
async fn a_waiting_backfill_step_whose_id_is_held_leaves_the_schedule_going() {
    a_waiting_manual_action_whose_id_is_held_leaves_the_schedule_going(ActionKind::Backfill).await;
}
