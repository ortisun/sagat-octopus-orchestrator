//! The fixture of the suites about trigger-owned schedules (feature 018, US07, `T018-060`, `T018-061`): a world with
//! cron workflows, the schedule service and the firing over the actual (runtime-role in the sweep) pool, the
//! reconciliation, and the legacy cron pass of the world's dispatcher, which share a clock the test moves by hand.
//! It lives here so that the integration and the fault binaries read the same code.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use sagat_application::identity::RequestContext;
use sagat_application::schedule_firing::{Fired, ScheduleFiring};
use sagat_application::schedule_triggers::{
    Drift, DriftWindow, TriggerReconciler, TriggerScheduleStore,
};
use sagat_application::schedules::{
    DueSchedule, ScheduleFilter, ScheduleOrigin, ScheduleRecord, ScheduleService, ScheduleStore,
};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::schedules::Ceilings;
use sagat_domain::sharding::Shard;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{PgIdentityStore, PgScheduleStore};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::engine::{World, activity, document, world};
use super::schedules::{Starts, TestClock};

pub(crate) type Service = ScheduleService<PgScheduleStore, SpecCalendar, TestClock>;
pub(crate) type Firing =
    ScheduleFiring<PgScheduleStore, SpecCalendar, TestClock, PgIdentityStore, Starts>;
pub(crate) type Reconciler = TriggerReconciler<PgScheduleStore, SpecCalendar, TestClock>;

pub(crate) struct Triggers {
    pub(crate) w: Arc<World>,
    pub(crate) clock: TestClock,
    pub(crate) store: PgScheduleStore,
    pub(crate) service: Service,
    pub(crate) firing: Firing,
    pub(crate) reconciler: Reconciler,
}

/// The world, with the clock at the database's now (the legacy pass is due by the database's clock and the schedule pass
/// by the test's, so the two meet on one tick).
pub(crate) async fn triggers() -> Triggers {
    let w = Arc::new(world().await);
    let clock = TestClock::at(OffsetDateTime::now_utc());
    // the actual pool: the runtime role when the sweep is on, so that every statement of the new code is looked at as it
    let store = PgScheduleStore::new(w.execution_pool.clone());
    let service = ScheduleService::new(
        store.clone(),
        SpecCalendar,
        clock.clone(),
        Ceilings::default(),
    );
    let firing = ScheduleFiring::new(
        store.clone(),
        SpecCalendar,
        clock.clone(),
        PgIdentityStore::new(w.execution_pool.clone()),
        Starts(Arc::clone(&w)),
        Ceilings::default(),
    );
    let reconciler = TriggerReconciler::new(store.clone(), SpecCalendar, clock.clone());
    Triggers {
        w,
        clock,
        store,
        service,
        firing,
        reconciler,
    }
}

/// A workflow with one cron trigger.
pub(crate) fn cron_workflow(name: &str, expression: &str, zone: &str) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = json!([{"kind": "cron", "cron": expression, "timezone": zone}]);
    doc
}

/// A second version of a workflow that keeps its cron trigger (a new entry node makes it another version).
pub(crate) fn second_version(name: &str, expression: &str, zone: &str) -> Value {
    let mut doc = document(name, "work2", vec![activity("work2")]);
    doc["triggers"] = json!([{"kind": "cron", "cron": expression, "timezone": zone}]);
    doc
}

/// A workflow with no trigger.
pub(crate) fn plain_workflow(name: &str) -> Value {
    document(name, "work", vec![activity("work")])
}

/// Every shard: what a replica that holds them all looks at.
pub(crate) fn all_shards() -> Vec<Shard> {
    Shard::all().collect()
}

pub(crate) fn window() -> DriftWindow {
    DriftWindow {
        shards: all_shards(),
        exclude: Vec::new(),
        per_tenant: 200,
        limit: 1_000,
        scan: 100_000,
        after: None,
    }
}

impl Triggers {
    /// The cron trigger rows, by workflow name: id, version in force, expression, zone.
    pub(crate) async fn trigger_of(&self, workflow: &str) -> (Uuid, Uuid, String, String) {
        sqlx::query_as(
            "SELECT t.id, t.version_id, t.cron, t.timezone FROM workflow_triggers t \
             JOIN workflows w ON w.id = t.workflow_id WHERE w.name = $1 AND t.kind = 'cron'",
        )
        .bind(workflow)
        .fetch_one(&self.w.db.pool)
        .await
        .expect("the cron trigger of the workflow")
    }

    /// The schedule of a trigger, in any state.
    pub(crate) async fn schedule_of(&self, trigger: Uuid) -> Option<ScheduleRecord> {
        let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM schedules WHERE trigger_id = $1")
            .bind(trigger)
            .fetch_optional(&self.w.db.pool)
            .await
            .expect("the schedule of the trigger");
        match id {
            None => None,
            Some(id) => self
                .store
                .schedule(
                    self.w.admin.tenant_id(),
                    sagat_domain::schedules::ScheduleId::new(id),
                )
                .await
                .expect("read"),
        }
    }

    /// A reconciliation of its own, for a task that outlives a borrow of the fixture.
    pub(crate) fn reconciler_for_task(&self) -> Reconciler {
        TriggerReconciler::new(self.store.clone(), SpecCalendar, self.clock.clone())
    }

    /// Every trigger-owned schedule of the first tenant, as the API lists them.
    pub(crate) async fn listed(&self) -> Vec<ScheduleRecord> {
        self.service
            .list(
                &self.w.admin,
                self.w.namespace.id(),
                &ScheduleFilter {
                    origin: Some(ScheduleOrigin::Trigger),
                    ..ScheduleFilter::default()
                },
                None,
                200,
            )
            .await
            .expect("list")
    }

    /// How the schedules differ from the triggers now.
    pub(crate) async fn drift(&self) -> Vec<Drift> {
        self.store.drift(&window()).await.expect("drift").drift
    }

    /// Reconciles every drift once, as a replica that holds every shard does in a round, without a fence.
    pub(crate) async fn reconcile(&self) -> usize {
        let handover = self.store.handover().await.expect("handover");
        let drift = self.drift().await;
        for one in &drift {
            self.reconciler
                .reconcile(one, &handover, None)
                .await
                .expect("reconcile");
        }
        drift.len()
    }

    /// Makes the tick `at` due for both passes: the trigger's next time (what the legacy pass fires) and the schedule's
    /// (what the schedule pass fires), both the same instant.
    pub(crate) async fn due_at(
        &self,
        trigger: Uuid,
        schedule: &ScheduleRecord,
        at: OffsetDateTime,
    ) {
        sqlx::query(
            "UPDATE workflow_triggers SET next_fire_at = $2, retry_at = NULL WHERE id = $1",
        )
        .bind(trigger)
        .bind(at)
        .execute(&self.w.db.pool)
        .await
        .expect("the trigger is due");
        sqlx::query(
            "UPDATE schedules SET next_action_at = $2, next_actual_at = $2, state_version = state_version + 1 \
             WHERE id = $1",
        )
        .bind(schedule.id.as_uuid())
        .bind(at)
        .execute(&self.w.db.pool)
        .await
        .expect("the schedule is due");
    }

    pub(crate) fn due(&self, schedule: &ScheduleRecord) -> DueSchedule {
        DueSchedule {
            tenant_id: schedule.tenant_id,
            id: schedule.id,
        }
    }

    /// One step of the schedule pass for the schedule, without a fence.
    pub(crate) async fn fire(&self, schedule: &ScheduleRecord) -> Fired {
        self.firing
            .fire(&self.due(schedule), None)
            .await
            .expect("fire")
    }

    pub(crate) async fn read(&self, schedule: &ScheduleRecord) -> ScheduleRecord {
        self.store
            .schedule(schedule.tenant_id, schedule.id)
            .await
            .expect("read")
            .expect("exists")
    }

    /// The runs of a workflow, oldest first.
    pub(crate) async fn runs_of(&self, workflow: WorkflowId) -> Vec<Uuid> {
        sqlx::query_scalar("SELECT id FROM runs WHERE workflow_id = $1 ORDER BY created_at, id")
            .bind(workflow.as_uuid())
            .fetch_all(&self.w.db.pool)
            .await
            .expect("runs")
    }

    /// What was decided for each nominal time of a schedule, oldest first: nominal, outcome, run.
    pub(crate) async fn actions_of(
        &self,
        schedule: &ScheduleRecord,
    ) -> Vec<(OffsetDateTime, String, Option<Uuid>)> {
        sqlx::query_as(
            "SELECT nominal_at, outcome, run_id FROM schedule_actions WHERE schedule_id = $1 \
             ORDER BY nominal_at, origin",
        )
        .bind(schedule.id.as_uuid())
        .fetch_all(&self.w.db.pool)
        .await
        .expect("actions")
    }

    /// Hands the cron triggers over the way the command does: the owner records the requirement and the state in one
    /// transaction (no engine is registered in these suites, so none can lack the capability).
    pub(crate) async fn hand_over(&self) {
        self.hand_over_now().await;
        // the control of a trigger-owned schedule opens a few seconds after the commit (D2046); the suites that are not
        // about that window move the commit back in time, the way the guard of the table lets the owner do it
        self.move_the_handover_back(10).await;
    }

    /// Moves the instant of the handover back by `seconds`. The guard of the table refuses every change of a handed-over
    /// row, so the test switches it off for the statement, as `old_binary_round` of the fault suite does, and puts it back.
    pub(crate) async fn move_the_handover_back(&self, seconds: i32) {
        let pool = &self.w.db.pool;
        sqlx::query(
            "ALTER TABLE schedule_trigger_handover DISABLE TRIGGER schedule_trigger_handover_guard",
        )
        .execute(pool)
        .await
        .expect("disable the guard");
        sqlx::query(
            "UPDATE schedule_trigger_handover SET changed_at = changed_at - make_interval(secs => $1::float8)",
        )
        .bind(seconds)
        .execute(pool)
        .await
        .expect("move it back");
        sqlx::query("ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_guard")
            .execute(pool)
            .await
            .expect("enable the guard");
    }

    /// Writes the handover row as the owner, with the guard off for the statement: the states the command never writes (a state a
    /// newer build wrote) and the committed handover without the census, for the tests that are about something else.
    pub(crate) async fn force_handover_row(&self, state: &str, changed_by: Option<&str>) {
        let pool = &self.w.db.pool;
        sqlx::query(
            "ALTER TABLE schedule_trigger_handover DISABLE TRIGGER schedule_trigger_handover_guard",
        )
        .execute(pool)
        .await
        .expect("disable the guard");
        sqlx::query(
            "UPDATE schedule_trigger_handover SET state = $1, changed_by = $2, \
                 changed_at = CASE WHEN $2::text IS NULL THEN NULL ELSE now() - interval '1 minute' END, \
                 changed_db_role = CASE WHEN $2::text IS NULL THEN NULL ELSE 'test' END",
        )
        .bind(state)
        .bind(changed_by)
        .execute(pool)
        .await
        .expect("write the row");
        sqlx::query("ALTER TABLE schedule_trigger_handover ENABLE ALWAYS TRIGGER schedule_trigger_handover_guard")
            .execute(pool)
            .await
            .expect("enable the guard");
    }

    /// The handover, with the schedules converged first (the handover refuses triggers that have none, D2046), and the
    /// instant of its commit left where it is. It calls the function that moves the state and nothing of the command: no census
    /// of the engines, no check that they list `schedules-v1`, no `--expect-live`, no refusal in words, no lock timeout. A suite
    /// that uses it tests what happens after a handover, not the handover: the command is tested end to end in
    /// `trigger_handover.rs` (the command line, the registry) and `trigger_round3.rs` (`the_apply_path_of_the_command...`,
    /// the census under the lock, the lock timeout) (D2078).
    pub(crate) async fn hand_over_now(&self) {
        self.reconcile().await;
        use sagat_application::instances::{Activation, Capability, InstanceKind};
        let operator = sagat_domain::identity::Subject::parse("test-operator").expect("subject");
        sagat_infra_postgres::PgInstanceStore::new(self.w.db.pool.clone())
            .activate(
                Activation {
                    capability: Capability::SchedulesV1,
                    kinds: &[InstanceKind::ExecutionEngine],
                    actor: &operator,
                },
                |conn| {
                    Box::pin(async move {
                        sagat_infra_postgres::complete_handover(conn, "test-operator")
                            .await
                            .map(|_| ())
                    })
                },
            )
            .await
            .expect("the handover");
    }

    pub(crate) async fn publish(&self, document: Value) -> (WorkflowId, WorkflowVersionId) {
        self.w.publish(document).await
    }

    pub(crate) fn ctx(&self) -> RequestContext {
        RequestContext::default()
    }

    /// The last whole hour: an occurrence of an hourly cron that is due by every clock and is the only one that is.
    pub(crate) fn last_hour() -> OffsetDateTime {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        OffsetDateTime::from_unix_timestamp(now - now % 3_600).expect("instant")
    }

    pub(crate) fn minute() -> Duration {
        Duration::minutes(1)
    }
}
