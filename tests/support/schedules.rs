//! The fixture of the schedule suites (feature 018): a world over a real PostgreSQL, a schedule
//! service and a firing that share a clock the test moves by hand. It lives here, and not in one
//! suite, so that the integration and the fault binaries read the same code.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use sagat_application::clock::Clock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedule_firing::{
    Fired, ScheduleFiring, ScheduledRun, ScheduledStart, ScheduledStarts,
};
use sagat_application::schedules::{
    DueSchedule, ScheduleAdmission, ScheduleRecord, ScheduleService, ScheduleStore, ScheduleWrite,
};
use sagat_domain::definition::WorkflowId;
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
    ) -> Result<ScheduledRun, AppError> {
        ScheduledStarts::start_scheduled(&self.0.runs, principal, request, admission, ctx).await
    }

    async fn cancel_run(
        &self,
        principal: &Principal,
        run: Uuid,
        reason: &str,
        key: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        ScheduledStarts::cancel_run(&self.0.runs, principal, run, reason, key, ctx).await
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
    fixture_with(start, Ceilings::default()).await
}

/// A fixture whose firing acts under `ceilings` (a recovery, a buffer, the runs it tracks) instead of
/// the defaults, so that a test reaches a ceiling with a handful of times.
pub(crate) async fn fixture_with(start: OffsetDateTime, ceilings: Ceilings) -> Fixture {
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
        ceilings,
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
    /// The write of a schedule, decoded under the hard ceilings so that a test can build a spec the
    /// service of a namespace with lower ceilings must still refuse.
    pub(crate) fn write_of(
        &self,
        name: &str,
        spec: &Value,
        policies: &Value,
        target: Option<Value>,
    ) -> ScheduleWrite {
        ScheduleWrite {
            name: ScheduleName::new(name).expect("name"),
            spec: spec_from_json(spec, &Ceilings::HARD).expect("spec"),
            policies: policies_from_json(policies, &Ceilings::HARD).expect("policies"),
            target: target_from_json(
                &target.unwrap_or_else(|| json!({"workflowId": self.workflow.to_string()})),
            )
            .expect("target"),
            input: json!({"region": "eu"}),
            paused: false,
            note: None,
            remaining_actions: None,
        }
    }

    /// Creates the schedule, or says why the service refused.
    pub(crate) async fn try_create(
        &self,
        write: ScheduleWrite,
    ) -> Result<ScheduleRecord, AppError> {
        self.service
            .create(
                &self.w.admin,
                self.w.namespace.id(),
                write,
                &RequestContext::default(),
            )
            .await
    }

    pub(crate) async fn create(
        &self,
        name: &str,
        spec: &Value,
        policies: &Value,
        target: Option<Value>,
    ) -> ScheduleRecord {
        self.try_create(self.write_of(name, spec, policies, target))
            .await
            .expect("create the schedule")
    }

    /// Creates the schedule as `who`, who becomes its authority.
    pub(crate) async fn create_as(
        &self,
        who: &Principal,
        name: &str,
        spec: &Value,
        policies: &Value,
        target: Option<Value>,
    ) -> ScheduleRecord {
        self.service
            .create(
                who,
                self.w.namespace.id(),
                self.write_of(name, spec, policies, target),
                &RequestContext::default(),
            )
            .await
            .expect("create the schedule")
    }

    /// Runs `action` while another transaction holds the row of `schedule`; once the action is
    /// waiting for that row, `subject` is demoted to a viewer and committed, and then the row is
    /// released. Whatever the action read about `subject` before it started is out of date when it
    /// admits its run: a revocation that has returned must stop it (`FR-018-015`, ADR-0050 decision 7).
    ///
    /// The wait is for a backend that waits on a lock (`pg_stat_activity`), not for a time.
    pub(crate) async fn revoked_while_the_action_waits<T>(
        &self,
        schedule: &ScheduleRecord,
        subject: &str,
        action: impl std::future::Future<Output = T>,
    ) -> T {
        let mut held = self.w.db.pool.begin().await.expect("hold the schedule");
        sqlx::query("SELECT id FROM schedules WHERE id = $1 FOR UPDATE")
            .bind(schedule.id.as_uuid())
            .fetch_one(&mut *held)
            .await
            .expect("lock the row of the schedule");
        let revoke = async {
            self.wait_for_a_backend_on_a_lock("the action waits for the row of the schedule")
                .await;
            sqlx::query(
                "UPDATE memberships SET role = 'viewer' WHERE tenant_id = $1 AND subject = $2",
            )
            .bind(schedule.tenant_id.as_uuid())
            .bind(subject)
            .execute(&self.w.db.pool)
            .await
            .expect("revoke");
            held.commit().await.expect("release the schedule");
        };
        let (done, ()) = tokio::join!(action, revoke);
        done
    }

    /// Runs `action` while another transaction holds the row of `run` and has already finished it
    /// as `status`; once the action is waiting for that row the transaction commits. The first
    /// commit wins: the action finds a run that is over (`FR-018-007`).
    pub(crate) async fn finished_while_the_action_waits<T>(
        &self,
        run: Uuid,
        status: &str,
        action: impl std::future::Future<Output = T>,
    ) -> T {
        let mut held = self.w.db.pool.begin().await.expect("hold the run");
        sqlx::query("UPDATE runs SET status = $2, finished_at = now() WHERE id = $1")
            .bind(run)
            .bind(status)
            .execute(&mut *held)
            .await
            .expect("finish the run, uncommitted");
        let finish = async {
            self.wait_for_a_backend_on_a_lock("the action waits for the row of the run")
                .await;
            held.commit().await.expect("commit the end of the run");
        };
        let (done, ()) = tokio::join!(action, finish);
        done
    }

    /// Waits until some backend waits on a lock (`pg_stat_activity`, not a time), for at most two
    /// minutes, then panics naming `stage`.
    pub(crate) async fn wait_for_a_backend_on_a_lock(&self, stage: &str) {
        super::bounded::within(stage, std::time::Duration::from_secs(120), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid() \
                       AND wait_event_type = 'Lock'",
                )
                .fetch_one(&self.w.db.pool)
                .await
                .expect("look at the backends");
                if waiting > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
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

/// Fires, once, a schedule that was created through the REST API of `api`, at the nominal time `nominal` and without waiting for
/// it: the times the schedule keeps are moved to `nominal` (what the pass finds as due) and the service the pass calls acts over
/// the stores of the API and under the authority of the member who created the schedule. **The firing decides by a clock fixed
/// five seconds after `nominal`**, not by the wall clock, so that no other time of the schedule is due while the test runs (a
/// test that straddled the next time would otherwise find the firing acting on that one too, or only on the latest of the two under
/// the catch-up mode `ONE`). What it starts is a run like any other, read back through the same API. The nominal time is the one the
/// caller gives: what this proves is that it is passed through to the run, not how a schedule calculates it (the callers take it
/// from the schedule's own `nextActionAt` less one period, and check the schedule's calculation of the next one afterwards).
pub(crate) async fn fire_at(
    api: &super::api::TestApi,
    tenant: sagat_domain::identity::TenantId,
    schedule: &str,
    nominal: OffsetDateTime,
) -> Fired {
    use sagat_application::clock::SystemClock;
    use sagat_application::execution::{DefinitionCache, Engine};
    use sagat_application::runs::RunService;
    use sagat_domain::schedules::ScheduleId;
    use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};

    let id: Uuid = schedule.parse().expect("a schedule id");
    sqlx::query(
        "UPDATE schedules SET next_action_at = $2, next_actual_at = $2, state_version = state_version + 1 WHERE id = $1",
    )
    .bind(id)
    .bind(nominal)
    .execute(&api.db.pool)
    .await
    .expect("the schedule is due");
    let firing = ScheduleFiring::new(
        PgScheduleStore::new(api.api_pool.clone()),
        SpecCalendar,
        TestClock::at(nominal + Duration::seconds(5)),
        PgIdentityStore::new(api.api_pool.clone()),
        RunService::new(
            api.executions.clone(),
            SystemClock,
            Engine::new(JsonEvaluator),
            DefinitionCache::new(Arc::new(JsonIrCodec), 64),
        ),
        Ceilings::default(),
    );
    firing
        .fire(
            &DueSchedule {
                tenant_id: tenant,
                id: ScheduleId::new(id),
            },
            None,
        )
        .await
        .expect("fire")
}

/// Marks a run as started by a schedule action, for a test that built the run another way (a seeded chain, a loop, a collection)
/// and needs to see what the paths that finish a run say about it: the columns the admission writes with the run. The provenance
/// of a run never changes (a trigger refuses it), so the guard is switched off, by the owner, for this one statement (each of the
/// three is its own transaction: the table cannot be altered while the update has events pending) and on again after it.
pub(crate) async fn mark_as_scheduled(
    pool: &sqlx::PgPool,
    run: Uuid,
    schedule: Uuid,
    nominal: OffsetDateTime,
) {
    sqlx::query("ALTER TABLE runs DISABLE TRIGGER runs_schedule_provenance")
        .execute(pool)
        .await
        .expect("switch the guard off");
    let marked = sqlx::query(
        "UPDATE runs SET schedule_id = $2, schedule_nominal_at = $3, schedule_action_kind = 'SCHEDULED', \
         schedule_revision = 1 WHERE id = $1",
    )
    .bind(run)
    .bind(schedule)
    .bind(nominal)
    .execute(pool)
    .await
    .expect("mark the run")
    .rows_affected();
    assert_eq!(marked, 1, "the run exists");
    sqlx::query("ALTER TABLE runs ENABLE ALWAYS TRIGGER runs_schedule_provenance")
        .execute(pool)
        .await
        .expect("switch the guard on");
}

/// The body the one delivery of a subscription carries: what the subscriber is sent for a finished run.
pub(crate) async fn delivered_body(pool: &sqlx::PgPool, subscription: Uuid) -> Value {
    let bytes: Vec<u8> =
        sqlx::query_scalar("SELECT payload FROM webhook_deliveries WHERE subscription_id = $1")
            .bind(subscription)
            .fetch_one(pool)
            .await
            .expect("one delivery");
    serde_json::from_slice(&bytes).expect("a JSON body")
}

/// An hour boundary a few hours ago. The pass finds what is due by the database clock and the
/// firing decides by the clock of the test, so the test clock starts where the first time of an
/// hourly schedule is already in the past for both.
pub(crate) fn some_hours_ago() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    OffsetDateTime::from_unix_timestamp(now - now % 3_600 - 3 * 3_600).expect("instant")
}

/// An ad-hoc database run of the world's admin (feature 017's private document), ended: what a batch `RERUN` repeats
/// through `rerun_private`. Its connection is declared and never reached: the run is cancelled before any worker asks.
#[allow(dead_code)]
pub(crate) async fn ad_hoc_run(w: &World, tag: &str) -> sagat_domain::execution::RunId {
    use sagat_application::database::{ConnectionService, ConnectionWrite};
    use sagat_application::runs::{AdHocMode, RunOperation, StartDatabaseExecution};
    use sagat_domain::database::{ConnectionConfig, ConnectionStatus, DatabaseLimits, TlsPolicy};
    use sagat_domain::identity::Slug;
    use sagat_domain::secrets::SecretName;

    let name = format!("ad-hoc-{tag}");
    // the credential the connection names must exist, and is never read: the run is cancelled before a worker asks
    let _ = w
        .secrets
        .put(
            &w.admin,
            w.namespace.id(),
            "db-credentials",
            b"never-read".to_vec(),
            &RequestContext::default(),
        )
        .await;
    // as if it had been moved to OpenBao (no OpenBao runs here): the admission of a connection asks only that
    sqlx::query(
        "UPDATE secrets SET authority = 'openbao' WHERE tenant_id = $1 AND namespace_id = $2 AND name = 'db-credentials'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("the authority of the credential");
    ConnectionService::new(w.store.clone(), sagat_application::clock::SystemClock)
        .put(
            &w.admin,
            w.namespace.id(),
            ConnectionWrite {
                name: Slug::parse(&name).expect("a slug"),
                expected_revision: 0,
                request_id: Uuid::new_v4(),
                status: ConnectionStatus::Active,
                config: ConnectionConfig::postgres(
                    "db.example.com",
                    5432,
                    "orders",
                    TlsPolicy::VerifyFull { ca_pem: None },
                    SecretName::parse("db-credentials").expect("a name"),
                    DatabaseLimits::default(),
                )
                .expect("a config"),
            },
            &RequestContext::default(),
        )
        .await
        .expect("the connection");
    let bytes = serde_json::to_vec(&super::engine::document(
        &format!("database-ad-hoc-{tag}"),
        "query",
        vec![json!({
            "id": "query", "type": "database", "connectionRef": {"name": name},
            "operation": "query", "statement": "SELECT 1::bigint"
        })],
    ))
    .expect("the document");
    let key = format!("ad-hoc-{tag}-{}", Uuid::now_v7().simple());
    let run = w
        .runs
        .start_database(
            &w.admin,
            &StartDatabaseExecution {
                namespace: w.namespace.id(),
                mode: AdHocMode::Step,
                document: &bytes,
                input: b"{}",
                timeout_ms: None,
                key: &key,
            },
            &RequestContext::default(),
        )
        .await
        .expect("the ad-hoc run")
        .info
        .run_id;
    w.operate(run, RunOperation::Cancel)
        .await
        .expect("cancel it");
    run
}
