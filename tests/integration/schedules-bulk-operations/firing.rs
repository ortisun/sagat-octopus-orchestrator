//! The firing of due schedules (`T018-016`, `T018-019`, `T018-021`, `T018-026`, `T018-027`,
//! `T018-029`): what a pass does with a schedule whose time has come, over a real PostgreSQL, a real
//! run service and a clock the test moves by hand.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedule_firing::{Fired, ScheduleFiring, ScheduledStart, ScheduledStarts};
use sagat_application::schedules::{
    DueSchedule, ScheduleAdmission, ScheduleRecord, ScheduleService, ScheduleState, ScheduleStore,
    ScheduleWrite,
};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::Principal;
use sagat_domain::schedules::{Ceilings, ScheduleName};
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgIdentityStore, PgScheduleStore};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::support::engine::{World, chain, world};

#[derive(Clone)]
pub(crate) struct TestClock(Arc<Mutex<OffsetDateTime>>);

impl TestClock {
    pub(crate) fn advance(&self, by: Duration) {
        let mut now = self.0.lock().expect("clock");
        *now += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> OffsetDateTime {
        *self.0.lock().expect("clock")
    }
}

/// The run service of the world, shared with the firing.
#[derive(Clone)]
pub(crate) struct Starts(pub(crate) Arc<World>);

impl ScheduledStarts for Starts {
    async fn start_scheduled(
        &self,
        principal: &Principal,
        request: &ScheduledStart<'_>,
        admission: ScheduleAdmission,
        ctx: &RequestContext,
    ) -> Result<RunId, AppError> {
        ScheduledStarts::start_scheduled(&self.0.runs, principal, request, admission, ctx).await
    }
}

pub(crate) type Service = ScheduleService<PgScheduleStore, SpecCalendar, TestClock>;
pub(crate) type Firing =
    ScheduleFiring<PgScheduleStore, SpecCalendar, TestClock, PgIdentityStore, Starts>;

pub(crate) struct Fixture {
    pub(crate) w: Arc<World>,
    pub(crate) clock: TestClock,
    pub(crate) service: Service,
    pub(crate) firing: Firing,
    pub(crate) workflow: WorkflowId,
}

/// Wednesday 2026-10-28 12:00 UTC, a whole hour since the epoch.
pub(crate) fn t0() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_793_188_800).expect("instant")
}

pub(crate) async fn fixture() -> Fixture {
    fixture_from(t0()).await
}

/// A fixture whose clock starts at `start`.
pub(crate) async fn fixture_from(start: OffsetDateTime) -> Fixture {
    let w = world().await;
    let (workflow, _) = w.publish(chain("scheduled")).await;
    let w = Arc::new(w);
    let clock = TestClock(Arc::new(Mutex::new(start)));
    let store = PgScheduleStore::new(w.db.pool.clone());
    let service = ScheduleService::new(
        store.clone(),
        SpecCalendar,
        clock.clone(),
        Ceilings::default(),
    );
    let firing = ScheduleFiring::new(
        store,
        SpecCalendar,
        clock.clone(),
        PgIdentityStore::new(w.db.pool.clone()),
        Starts(Arc::clone(&w)),
        Ceilings::default(),
    );
    Fixture {
        w,
        clock,
        service,
        firing,
        workflow,
    }
}

pub(crate) fn hourly() -> Value {
    json!({"intervals": [{"everyMs": 3_600_000}]})
}

impl Fixture {
    pub(crate) async fn create(
        &self,
        name: &str,
        spec: &Value,
        policies: &Value,
        target: Option<Value>,
    ) -> ScheduleRecord {
        let write = ScheduleWrite {
            name: ScheduleName::new(name).expect("name"),
            spec: spec_from_json(spec, &Ceilings::default()).expect("spec"),
            policies: policies_from_json(policies, &Ceilings::default()).expect("policies"),
            target: target_from_json(
                &target.unwrap_or_else(|| json!({"workflowId": self.workflow.to_string()})),
            )
            .expect("target"),
            input: json!({"region": "eu"}),
            paused: false,
            note: None,
            remaining_actions: None,
        };
        self.service
            .create(
                &self.w.admin,
                self.w.namespace.id(),
                write,
                &RequestContext::default(),
            )
            .await
            .expect("create the schedule")
    }

    pub(crate) fn due(&self, schedule: &ScheduleRecord) -> DueSchedule {
        DueSchedule {
            tenant_id: schedule.tenant_id,
            id: schedule.id,
        }
    }

    pub(crate) async fn fire(&self, schedule: &ScheduleRecord) -> Fired {
        self.firing
            .fire(&self.due(schedule), None)
            .await
            .expect("fire")
    }

    /// Fires until the schedule has nothing more to do now; returns how many steps were taken.
    pub(crate) async fn drain(&self, schedule: &ScheduleRecord) -> u32 {
        let mut steps = 0;
        for _ in 0..50 {
            match self.fire(schedule).await {
                Fired::Acted => steps += 1,
                Fired::Idle | Fired::Refused(_) => return steps,
            }
        }
        panic!("the schedule never became idle");
    }

    pub(crate) async fn read(&self, schedule: &ScheduleRecord) -> ScheduleRecord {
        PgScheduleStore::new(self.w.db.pool.clone())
            .schedule(schedule.tenant_id, schedule.id)
            .await
            .expect("read")
            .expect("exists")
    }

    pub(crate) async fn outcomes(&self, schedule: &ScheduleRecord) -> Vec<(String, i64)> {
        sqlx::query_as(
            "SELECT outcome, count(*) FROM schedule_actions WHERE schedule_id = $1 GROUP BY outcome ORDER BY outcome",
        )
        .bind(schedule.id.as_uuid())
        .fetch_all(&self.w.db.pool)
        .await
        .expect("outcomes")
    }

    pub(crate) async fn runs(&self, schedule: &ScheduleRecord) -> Vec<Uuid> {
        sqlx::query_scalar("SELECT id FROM runs WHERE schedule_id = $1 ORDER BY created_at, id")
            .bind(schedule.id.as_uuid())
            .fetch_all(&self.w.db.pool)
            .await
            .expect("runs")
    }

    /// Ends a run the way the schedule sees it: only its status matters here.
    pub(crate) async fn end(&self, run: Uuid, status: &str) {
        sqlx::query("UPDATE runs SET status = $2, finished_at = now() WHERE id = $1")
            .bind(run)
            .bind(status)
            .execute(&self.w.db.pool)
            .await
            .expect("end the run");
    }
}

pub(crate) fn outcomes_of(pairs: &[(&str, i64)]) -> Vec<(String, i64)> {
    pairs.iter().map(|(o, n)| ((*o).to_owned(), *n)).collect()
}

// --- US01: a schedule starts runs at its times --------------------------------------------------

#[tokio::test]
async fn nothing_happens_before_the_time_and_one_run_starts_at_it() {
    let f = fixture().await;
    let schedule = f.create("hourly", &hourly(), &json!({}), None).await;
    assert_eq!(schedule.next_action_at, Some(t0() + Duration::hours(1)));
    assert_eq!(f.fire(&schedule).await, Fired::Idle, "it is not 13:00 yet");

    f.clock.advance(Duration::hours(1) + Duration::seconds(5));
    assert_eq!(f.fire(&schedule).await, Fired::Acted);
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(f.outcomes(&schedule).await, outcomes_of(&[("STARTED", 1)]));
    let after = f.read(&schedule).await;
    assert_eq!(after.next_action_at, Some(t0() + Duration::hours(2)));
    assert_eq!(
        f.fire(&schedule).await,
        Fired::Idle,
        "the same time is never acted on twice"
    );
}

#[tokio::test]
async fn the_run_gets_the_input_of_the_schedule_and_its_nominal_time_in_the_declared_field() {
    let f = fixture().await;
    let target = json!({"workflowId": f.workflow.to_string(), "nominalTimeField": "scheduledFor"});
    let schedule = f
        .create("stamped", &hourly(), &json!({}), Some(target))
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(3));
    f.drain(&schedule).await;
    let run = f.runs(&schedule).await[0];
    let content: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest WHERE r.id = $1",
    )
    .bind(run)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("input");
    let input: Value = serde_json::from_slice(&content).expect("json");
    assert_eq!(
        input,
        json!({"region": "eu", "scheduledFor": "2026-10-28T13:00:00Z"})
    );
}

#[tokio::test]
async fn a_paused_schedule_is_not_due_and_a_deleted_one_is_gone() {
    let f = fixture().await;
    let schedule = f.create("quiet", &hourly(), &json!({}), None).await;
    f.clock.advance(Duration::hours(2));
    sqlx::query("UPDATE schedules SET state = 'PAUSED' WHERE id = $1")
        .bind(schedule.id.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("pause");
    assert_eq!(f.fire(&schedule).await, Fired::Idle);
    assert!(f.runs(&schedule).await.is_empty());
    let unknown = DueSchedule {
        tenant_id: schedule.tenant_id,
        id: sagat_domain::schedules::ScheduleId::generate(),
    };
    assert_eq!(
        f.firing.fire(&unknown, None).await.expect("fire"),
        Fired::Idle
    );
}

// --- US02: overlap ------------------------------------------------------------------------------

#[tokio::test]
async fn skip_records_the_time_it_skipped_while_a_run_is_open() {
    let f = fixture().await;
    let schedule = f
        .create("skipper", &hourly(), &json!({"overlap": "SKIP"}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("SKIPPED_OVERLAP", 1), ("STARTED", 1)])
    );
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        Some(t0() + Duration::hours(3))
    );
}

#[tokio::test]
async fn buffer_one_keeps_one_waiting_drops_the_next_and_starts_the_waiting_one_when_the_run_ends()
{
    let f = fixture().await;
    let schedule = f
        .create("buffer", &hourly(), &json!({"overlap": "BUFFER_ONE"}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 15:00 is dropped: one is waiting already
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUFFERED", 1), ("DROPPED_BUFFER_FULL", 1), ("STARTED", 1)])
    );
    assert_eq!(f.read(&schedule).await.buffered, 1);

    let first = f.runs(&schedule).await[0];
    f.end(first, "SUCCEEDED").await;
    assert_eq!(
        f.fire(&schedule).await,
        Fired::Idle,
        "the open run was looked at a moment ago; it is looked at again in five seconds"
    );
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await; // forgets the run, then starts the waiting action
    assert_eq!(f.runs(&schedule).await.len(), 2);
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("DROPPED_BUFFER_FULL", 1), ("STARTED", 2)]),
        "the waiting action became the started one"
    );
    let after = f.read(&schedule).await;
    assert_eq!((after.buffered, after.tracked_runs.len()), (0, 1));
}

/// The hour before the last whole hour: near the database's clock, in the past.
fn an_hour_ago() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    OffsetDateTime::from_unix_timestamp(now - now % 3_600 - 3_600).expect("instant")
}

#[tokio::test]
async fn buffer_all_keeps_each_time_up_to_its_limit_and_starts_them_in_order() {
    // the store sets the time to look at a waiting action again from the database's clock, so the
    // clock of the test starts near it: an hour ago, and moves ahead
    let started = an_hour_ago();
    let f = fixture_from(started).await;
    let schedule = f
        .create(
            "queue",
            &hourly(),
            &json!({"overlap": "BUFFER_ALL", "bufferLimit": 2}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts
    for _ in 0..3 {
        f.clock.advance(Duration::hours(1));
        f.drain(&schedule).await; // 14:00 and 15:00 wait, 16:00 finds the buffer full
    }
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("BUFFERED", 2), ("DROPPED_BUFFER_FULL", 1), ("STARTED", 1)])
    );
    assert_eq!(f.read(&schedule).await.buffered, 2);

    for expected_runs in [2, 3] {
        let open = *f.runs(&schedule).await.last().expect("a run");
        f.end(open, "SUCCEEDED").await;
        f.clock.advance(Duration::seconds(6));
        f.drain(&schedule).await; // forgets the run, then starts the oldest waiting action
        assert_eq!(f.runs(&schedule).await.len(), expected_runs);
    }
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("DROPPED_BUFFER_FULL", 1), ("STARTED", 3)])
    );
    let nominal: Vec<OffsetDateTime> = sqlx::query_scalar(
        "SELECT schedule_nominal_at FROM runs WHERE schedule_id = $1 ORDER BY created_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("nominal times");
    assert_eq!(
        nominal,
        vec![
            started + Duration::hours(1),
            started + Duration::hours(2),
            started + Duration::hours(3)
        ],
        "in the order the times came"
    );
    assert_eq!(f.read(&schedule).await.buffered, 0);
}

#[tokio::test]
async fn a_waiting_action_keeps_the_revision_it_was_taken_under() {
    let f = fixture().await;
    let schedule = f
        .create(
            "revised",
            &hourly(),
            &json!({"overlap": "BUFFER_ONE"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await; // 13:00 starts
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await; // 14:00 waits, under revision 1

    let replacement = ScheduleWrite {
        name: ScheduleName::new("revised").expect("name"),
        spec: spec_from_json(&hourly(), &Ceilings::default()).expect("spec"),
        policies: policies_from_json(&json!({"overlap": "BUFFER_ONE"}), &Ceilings::default())
            .expect("policies"),
        target: target_from_json(&json!({"workflowId": f.workflow.to_string()})).expect("target"),
        input: json!({"region": "us"}),
        paused: false,
        note: None,
        remaining_actions: None,
    };
    let updated = f
        .service
        .update(
            &f.w.admin,
            schedule.id,
            replacement,
            1,
            &RequestContext::default(),
        )
        .await
        .expect("update");
    assert_eq!(updated.revision, 2);

    let first = f.runs(&schedule).await[0];
    f.end(first, "SUCCEEDED").await;
    f.clock.advance(Duration::seconds(6));
    f.drain(&schedule).await;
    let revisions: Vec<i32> = sqlx::query_scalar(
        "SELECT schedule_revision FROM runs WHERE schedule_id = $1 ORDER BY created_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("revisions");
    assert_eq!(
        revisions,
        vec![1, 1],
        "the waiting action starts under the revision it was taken under, not the head"
    );
    assert_eq!(f.read(&schedule).await.revision, 2);
}

#[tokio::test]
async fn allow_all_starts_beside_an_open_run() {
    let f = fixture().await;
    let schedule = f
        .create(
            "parallel",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    for _ in 0..3 {
        f.clock.advance(Duration::hours(1) + Duration::seconds(1));
        f.drain(&schedule).await;
    }
    assert_eq!(f.runs(&schedule).await.len(), 3);
    assert_eq!(f.read(&schedule).await.tracked_runs.len(), 3);
}

// --- US02: an outage ----------------------------------------------------------------------------

#[tokio::test]
async fn after_an_outage_mode_one_acts_on_the_earliest_time_and_coalesces_the_rest() {
    let f = fixture().await;
    let schedule = f
        .create(
            "outage-one",
            &hourly(),
            &json!({"catchupMode": "ONE"}),
            None,
        )
        .await;
    // the platform was away from 12:00 until 15:30: the times 13:00, 14:00 and 15:00 are owed
    f.clock.advance(Duration::hours(3) + Duration::minutes(30));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("COALESCED", 1), ("STARTED", 1)])
    );
    let (count, last): (i32, OffsetDateTime) = sqlx::query_as(
        "SELECT summary_count, summary_last_at FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'COALESCED'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("summary");
    assert_eq!((count, last), (2, t0() + Duration::hours(3)));
    assert_eq!(
        f.read(&schedule).await.next_action_at,
        Some(t0() + Duration::hours(4))
    );
}

#[tokio::test]
async fn after_an_outage_mode_all_acts_on_each_time_in_order() {
    let f = fixture().await;
    let schedule = f
        .create(
            "outage-all",
            &hourly(),
            &json!({"catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(3) + Duration::minutes(30));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 3);
    let nominals: Vec<OffsetDateTime> = sqlx::query_scalar(
        "SELECT schedule_nominal_at FROM runs WHERE schedule_id = $1 ORDER BY schedule_nominal_at",
    )
    .bind(schedule.id.as_uuid())
    .fetch_all(&f.w.db.pool)
    .await
    .expect("nominal times");
    assert_eq!(
        nominals,
        vec![
            t0() + Duration::hours(1),
            t0() + Duration::hours(2),
            t0() + Duration::hours(3)
        ]
    );
}

#[tokio::test]
async fn times_older_than_the_catch_up_window_are_one_missed_summary() {
    let f = fixture().await;
    // a window of 90 minutes
    let schedule = f
        .create(
            "window",
            &hourly(),
            &json!({"catchupWindowMs": 5_400_000, "catchupMode": "ALL", "overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(4) + Duration::minutes(10)); // 16:10: due 13, 14, 15, 16
    f.drain(&schedule).await;
    // 13:00 and 14:00 are older than 90 minutes before 16:10 (14:40); 15:00 and 16:00 are inside it
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("MISSED_CATCHUP_WINDOW", 1), ("STARTED", 2)])
    );
    let (count, last): (i32, OffsetDateTime) = sqlx::query_as(
        "SELECT summary_count, summary_last_at FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'MISSED_CATCHUP_WINDOW'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("summary");
    assert_eq!((count, last), (2, t0() + Duration::hours(2)));
}

// --- US02: remaining actions and failures -------------------------------------------------------

#[tokio::test]
async fn a_schedule_with_two_actions_left_stops_after_them() {
    let f = fixture().await;
    let created = f
        .create("limited", &hourly(), &json!({"overlap": "ALLOW_ALL"}), None)
        .await;
    sqlx::query("UPDATE schedules SET remaining_actions = 2 WHERE id = $1")
        .bind(created.id.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("limit");
    for _ in 0..3 {
        f.clock.advance(Duration::hours(1) + Duration::seconds(1));
        f.drain(&created).await;
    }
    assert_eq!(f.runs(&created).await.len(), 2);
    let done = f.read(&created).await;
    assert_eq!(done.remaining_actions, Some(0));
    assert_eq!(
        done.next_action_at, None,
        "an exhausted schedule has no next time"
    );
    assert_eq!(done.state, ScheduleState::Active);
    f.clock.advance(Duration::hours(5));
    assert_eq!(f.fire(&created).await, Fired::Idle);
}

#[tokio::test]
async fn a_failed_run_pauses_a_schedule_that_asks_for_it_with_a_system_note() {
    let f = fixture().await;
    let schedule = f
        .create("careful", &hourly(), &json!({"pauseOnFailure": true}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    let run = f.runs(&schedule).await[0];
    f.end(run, "FAILED").await;
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    let paused = f.read(&schedule).await;
    assert_eq!(paused.state, ScheduleState::Paused);
    assert_eq!(paused.note_by.as_deref(), Some("system"));
    assert!(paused.tracked_runs.is_empty());
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'schedule.auto_paused' AND resource_id = $1",
    )
    .bind(schedule.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1);
    assert_eq!(
        f.runs(&schedule).await.len(),
        1,
        "no run starts once it is paused"
    );
}

#[tokio::test]
async fn a_cancelled_run_does_not_pause_a_schedule_that_pauses_on_failure() {
    let f = fixture().await;
    let schedule = f
        .create("calm", &hourly(), &json!({"pauseOnFailure": true}), None)
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    f.end(f.runs(&schedule).await[0], "CANCELLED").await;
    f.clock.advance(Duration::hours(1));
    f.drain(&schedule).await;
    assert_eq!(f.read(&schedule).await.state, ScheduleState::Active);
    assert_eq!(f.runs(&schedule).await.len(), 2);
}

// --- authority ---------------------------------------------------------------------------------

#[tokio::test]
async fn when_the_authorizing_subject_loses_run_start_the_schedule_pauses_and_no_run_starts() {
    let f = fixture().await;
    let schedule = f.create("revoked", &hourly(), &json!({}), None).await;
    sqlx::query("UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2")
        .bind(schedule.tenant_id.as_uuid())
        .bind(&schedule.authorized_by)
        .execute(&f.w.db.pool)
        .await
        .expect("demote");
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    f.drain(&schedule).await;
    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("AUTHORITY_REVOKED", 1)])
    );
    let paused = f.read(&schedule).await;
    assert_eq!(paused.state, ScheduleState::Paused);
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM schedule_actions WHERE schedule_id = $1 AND outcome = 'AUTHORITY_REVOKED'",
    )
    .bind(schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("reason");
    assert_eq!(reason.as_deref(), Some("AUTHORITY_REVOKED"));
}

// --- refusals and races ------------------------------------------------------------------------

#[tokio::test]
async fn a_run_the_platform_cannot_admit_keeps_the_time_owed_and_tries_again_later() {
    let f = fixture().await;
    let schedule = f.create("owed", &hourly(), &json!({}), None).await;
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
    assert!(matches!(f.fire(&schedule).await, Fired::Refused(_)));
    let owed = f.read(&schedule).await;
    assert_eq!(
        owed.next_action_at, schedule.next_action_at,
        "the time is still owed"
    );
    assert!(
        owed.retry_at.is_some_and(|t| t > f.clock.now()),
        "and it will be tried again later"
    );
    assert!(f.runs(&schedule).await.is_empty());
    assert_eq!(f.fire(&schedule).await, Fired::Idle, "not before the retry");

    sqlx::query("DELETE FROM quota_limits WHERE resource = 'active_runs'")
        .execute(&f.w.db.pool)
        .await
        .expect("room again");
    f.clock.advance(Duration::minutes(6));
    f.drain(&schedule).await;
    assert_eq!(f.runs(&schedule).await.len(), 1);
    assert_eq!(
        f.outcomes(&schedule).await,
        outcomes_of(&[("STARTED", 1)]),
        "the owed time started under its own key; the minutes of delay are not a missed time"
    );
}

#[tokio::test]
async fn two_processes_acting_on_the_same_time_start_exactly_one_run() {
    let f = fixture().await;
    let schedule = f
        .create(
            "contended",
            &hourly(),
            &json!({"overlap": "ALLOW_ALL"}),
            None,
        )
        .await;
    f.clock.advance(Duration::hours(1) + Duration::seconds(1));
    let firing = Arc::new(f.firing);
    let due = DueSchedule {
        tenant_id: schedule.tenant_id,
        id: schedule.id,
    };
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let firing = Arc::clone(&firing);
        let due = due.clone();
        tasks.push(tokio::spawn(async move { firing.fire(&due, None).await }));
    }
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.expect("task").expect("fire"));
    }
    let acted = results.iter().filter(|r| **r == Fired::Acted).count();
    assert_eq!(
        acted, 1,
        "one process acted, the others found it done or lost the swap: {results:?}"
    );
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE schedule_id = $1")
        .bind(schedule.id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("runs");
    assert_eq!(runs, 1);
}
