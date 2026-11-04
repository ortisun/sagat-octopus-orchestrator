//! The fixture of the schedule suites (feature 018): a world over a real PostgreSQL, a schedule
//! service and a firing that share a clock the test moves by hand. It lives here, and not in one
//! suite, so that the integration and the fault binaries read the same code.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedule_firing::{Fired, ScheduleFiring, ScheduledStart, ScheduledStarts};
use sagat_application::schedules::{
    DueSchedule, ScheduleAdmission, ScheduleRecord, ScheduleService, ScheduleStore, ScheduleWrite,
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

use super::engine::{World, chain, world};

#[derive(Clone)]
pub(crate) struct TestClock(Arc<Mutex<OffsetDateTime>>);

impl TestClock {
    pub(crate) fn at(now: OffsetDateTime) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    pub(crate) fn advance(&self, by: Duration) {
        let mut now = self.0.lock().expect("clock");
        *now += by;
    }

    /// Moves the clock to `to`, backwards too: a test that needs several schedules created at the
    /// same instant sets the clock back before it creates the next.
    pub(crate) fn set(&self, to: OffsetDateTime) {
        *self.0.lock().expect("clock") = to;
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

/// An hour boundary a few hours ago. The pass finds what is due by the database clock and the
/// firing decides by the clock of the test, so the test clock starts where the first time of an
/// hourly schedule is already in the past for both.
pub(crate) fn some_hours_ago() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    OffsetDateTime::from_unix_timestamp(now - now % 3_600 - 3 * 3_600).expect("instant")
}
