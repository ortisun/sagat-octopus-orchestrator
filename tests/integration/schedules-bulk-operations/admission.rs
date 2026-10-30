//! A scheduled action admits its run in one transaction (`T018-015`, `FR-018-004`, `FR-018-005`):
//! the fence, the compare-and-swap on the schedule, the run with its provenance, the action row and
//! the new state of the schedule commit together or not at all.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::error::AppError;
use sagat_application::runs::StartRun;
use sagat_application::schedules::{
    NewSchedule, ScheduleAdmission, ScheduleAfter, ScheduleRecord, ScheduleStore, ShardFence,
};
use sagat_application::shards::Lease;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{Principal, Role, Subject};
use sagat_domain::schedules::ActionKind;
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::PgScheduleStore;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::store_support::record;
use crate::support::engine::{World, chain, ctx, world};

struct Fixture {
    w: World,
    store: PgScheduleStore,
    schedule: ScheduleRecord,
    workflow: sagat_domain::definition::WorkflowId,
}

async fn fixture() -> Fixture {
    let w = world().await;
    let (workflow, _version) = w.publish(chain("scheduled")).await;
    let store = PgScheduleStore::new(w.db.pool.clone());
    let mut schedule = record(w.admin.tenant_id(), w.namespace.id(), "morning");
    schedule.target = sagat_application::schedule_codec::target_from_json(
        &serde_json::json!({"workflowId": workflow.to_string()}),
    )
    .expect("target");
    schedule.next_action_at = Some(OffsetDateTime::now_utc() - Duration::minutes(1));
    schedule.next_actual_at = schedule.next_action_at;
    schedule.remaining_actions = Some(3);
    store
        .create_schedule(
            &NewSchedule {
                record: schedule.clone(),
                input: None,
                request_id: None,
            },
            100,
            &AuditRecord::by(
                &w.admin,
                AuditAction::ScheduleCreated,
                ResourceType::Schedule,
                schedule.id.to_string(),
                None,
                schedule.created_at,
            ),
        )
        .await
        .expect("create the schedule");
    let schedule = store
        .schedule(w.admin.tenant_id(), schedule.id)
        .await
        .expect("read")
        .expect("exists");
    Fixture {
        w,
        store,
        schedule,
        workflow,
    }
}

impl Fixture {
    /// The identity of an action: its schedule and its nominal time.
    fn principal(&self) -> Principal {
        Principal::new(
            self.w.admin.tenant_id(),
            Subject::schedule(self.schedule.id.as_uuid()),
            Role::Developer,
        )
    }

    fn admission(&self, nominal: OffsetDateTime) -> ScheduleAdmission {
        let next = nominal + Duration::days(1);
        ScheduleAdmission {
            schedule: self.schedule.id,
            nominal_at: nominal,
            kind: ActionKind::Scheduled,
            revision: self.schedule.revision,
            expected_state_version: self.schedule.state_version,
            fence: None,
            after: ScheduleAfter {
                next_action_at: Some(next),
                next_actual_at: Some(next),
                remaining_actions: self.schedule.remaining_actions.map(|n| n - 1),
                extra: vec![],
            },
        }
    }

    async fn fire(
        &self,
        admission: ScheduleAdmission,
    ) -> Result<sagat_application::execution::RunRecord, AppError> {
        let key = admission.run_key();
        self.w
            .runs
            .start_scheduled(
                &self.principal(),
                &StartRun {
                    namespace: "payments",
                    workflow: self.workflow,
                    version: None,
                    input: br#"{"region":"eu"}"#,
                    correlation_id: None,
                    deadline: None,
                    key: &key,
                },
                admission,
                &ctx(),
            )
            .await
    }

    async fn actions(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM schedule_actions WHERE schedule_id = $1")
            .bind(self.schedule.id.as_uuid())
            .fetch_one(&self.w.db.pool)
            .await
            .expect("count")
    }

    async fn runs_of_the_schedule(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE schedule_id = $1")
            .bind(self.schedule.id.as_uuid())
            .fetch_one(&self.w.db.pool)
            .await
            .expect("count")
    }

    async fn reread(&self) -> ScheduleRecord {
        self.store
            .schedule(self.w.admin.tenant_id(), self.schedule.id)
            .await
            .expect("read")
            .expect("exists")
    }
}

fn nominal() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_793_160_000).expect("instant")
}

#[tokio::test]
async fn a_scheduled_action_starts_its_run_and_moves_the_schedule_in_one_commit() {
    let f = fixture().await;
    let started = f.fire(f.admission(nominal())).await.expect("fire");
    let run = started.info.run_id;

    // the run says who started it and why
    assert_eq!(
        started.info.started_by,
        format!("system:schedule:{}", f.schedule.id)
    );
    let (id, at, kind, revision): (Uuid, OffsetDateTime, String, i32) = sqlx::query_as(
        "SELECT schedule_id, schedule_nominal_at, schedule_action_kind, schedule_revision FROM runs WHERE id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("provenance");
    assert_eq!(
        (id, at, kind.as_str(), revision),
        (f.schedule.id.as_uuid(), nominal(), "SCHEDULED", 1)
    );

    // the action is recorded with the run
    let (outcome, run_id, origin): (String, Option<Uuid>, String) = sqlx::query_as(
        "SELECT outcome, run_id, origin FROM schedule_actions WHERE schedule_id = $1",
    )
    .bind(f.schedule.id.as_uuid())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("action");
    assert_eq!(
        (outcome.as_str(), run_id, origin.as_str()),
        ("STARTED", Some(run.as_uuid()), "SPEC")
    );

    // and the schedule has moved on: next time, one action fewer, the run tracked, the counter up
    let after = f.reread().await;
    assert_eq!(after.state_version, f.schedule.state_version + 1);
    assert_eq!(after.next_action_at, Some(nominal() + Duration::days(1)));
    assert_eq!(after.remaining_actions, Some(2));
    assert_eq!(after.tracked_runs, vec![run.as_uuid()]);
    let started_count: i64 =
        sqlx::query_scalar("SELECT (counters->>'STARTED')::bigint FROM schedules WHERE id = $1")
            .bind(f.schedule.id.as_uuid())
            .fetch_one(&f.w.db.pool)
            .await
            .expect("counter");
    assert_eq!(started_count, 1);
}

#[tokio::test]
async fn the_same_nominal_time_never_starts_a_second_run() {
    let f = fixture().await;
    let first = f.fire(f.admission(nominal())).await.expect("first");
    // a second process, with the state it read before the first committed, acts on the same time:
    // the key of the time finds the run the first made, and the second is told it did not act
    let again = f
        .fire(f.admission(nominal()))
        .await
        .expect_err("the time was already acted on");
    assert!(matches!(again, AppError::StateConflict(_)), "{again:?}");
    assert_eq!(f.runs_of_the_schedule().await, 1, "{first:?}");
    assert_eq!(f.actions().await, 1);
    assert_eq!(f.runs_of_the_schedule().await, 1);
    assert_eq!(
        f.reread().await.state_version,
        f.schedule.state_version + 1,
        "the schedule moved once"
    );
}

#[tokio::test]
async fn a_decision_made_on_a_stale_schedule_commits_nothing() {
    let f = fixture().await;
    let mut stale = f.admission(nominal());
    stale.expected_state_version += 5;
    let refused = f
        .fire(stale)
        .await
        .expect_err("the schedule changed since the decision");
    assert!(matches!(refused, AppError::StateConflict(_)), "{refused:?}");
    assert_eq!((f.actions().await, f.runs_of_the_schedule().await), (0, 0));
    assert_eq!(f.reread().await.state_version, f.schedule.state_version);
}

#[tokio::test]
async fn a_paused_schedule_admits_no_scheduled_run() {
    let f = fixture().await;
    sqlx::query("UPDATE schedules SET state = 'PAUSED' WHERE id = $1")
        .bind(f.schedule.id.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("pause");
    let mut paused = f.admission(nominal());
    paused.expected_state_version = f.reread().await.state_version;
    let refused = f.fire(paused).await.expect_err("paused");
    assert!(matches!(refused, AppError::StateConflict(_)), "{refused:?}");
    assert_eq!((f.actions().await, f.runs_of_the_schedule().await), (0, 0));
}

#[tokio::test]
async fn a_run_the_admission_refuses_takes_the_action_and_the_schedule_change_with_it() {
    let f = fixture().await;
    sqlx::query(
        "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
         VALUES ($1, $2, 'active_runs', 0, 'admin')",
    )
    .bind(f.w.admin.tenant_id().as_uuid())
    .bind(f.w.namespace.id().as_uuid())
    .execute(&f.w.db.pool)
    .await
    .expect("no room for runs");
    let refused = f.fire(f.admission(nominal())).await.expect_err("no room");
    assert!(matches!(refused, AppError::QuotaExceeded(_)), "{refused:?}");
    assert_eq!((f.actions().await, f.runs_of_the_schedule().await), (0, 0));
    let unchanged = f.reread().await;
    assert_eq!(
        unchanged.state_version, f.schedule.state_version,
        "the time is still owed"
    );
    assert_eq!(unchanged.next_action_at, f.schedule.next_action_at);
    assert_eq!(unchanged.tracked_runs, Vec::<Uuid>::new());
}

// --- the fence ---------------------------------------------------------------------------------

async fn hold_shard(f: &Fixture, owner: &str, generation: i64, valid_for: Duration) -> Lease {
    let shard = Shard::of(f.schedule.id.as_uuid());
    sqlx::query(
        "UPDATE scheduler_shards SET owner = $2, generation = $3, expires_at = clock_timestamp() + make_interval(secs => $4) WHERE shard = $1",
    )
    .bind(i16::from(shard.index()))
    .bind(owner)
    .bind(generation)
    .bind(valid_for.as_seconds_f64())
    .execute(&f.w.db.pool)
    .await
    .expect("shard");
    Lease {
        shard,
        generation: u64::try_from(generation).expect("generation"),
    }
}

fn fenced(mut admission: ScheduleAdmission, owner: &str, lease: Lease) -> ScheduleAdmission {
    admission.fence = Some(ShardFence {
        owner: owner.to_owned(),
        lease,
    });
    admission
}

#[tokio::test]
async fn the_owner_of_the_shard_may_act_and_a_former_owner_may_not() {
    let f = fixture().await;
    let lease = hold_shard(&f, "engine-a", 7, Duration::minutes(5)).await;
    let former = Lease {
        generation: lease.generation - 1,
        ..lease
    };
    let refused = f
        .fire(fenced(f.admission(nominal()), "engine-a", former))
        .await
        .expect_err("a generation behind");
    assert!(matches!(refused, AppError::StateConflict(_)), "{refused:?}");
    let other = f
        .fire(fenced(f.admission(nominal()), "engine-b", lease))
        .await
        .expect_err("another owner");
    assert!(matches!(other, AppError::StateConflict(_)), "{other:?}");
    assert_eq!((f.actions().await, f.runs_of_the_schedule().await), (0, 0));

    f.fire(fenced(f.admission(nominal()), "engine-a", lease))
        .await
        .expect("the owner acts");
    assert_eq!(f.actions().await, 1);
}

#[tokio::test]
async fn a_lease_that_has_run_out_fences_its_owner_off_by_the_database_clock() {
    let f = fixture().await;
    let lease = hold_shard(&f, "engine-a", 3, Duration::seconds(-5)).await;
    let refused = f
        .fire(fenced(f.admission(nominal()), "engine-a", lease))
        .await
        .expect_err("expired");
    assert!(matches!(refused, AppError::StateConflict(_)), "{refused:?}");
    assert_eq!(f.actions().await, 0);
}

#[tokio::test]
async fn a_lease_on_another_shard_is_not_a_lease_on_this_one() {
    let f = fixture().await;
    let mine = hold_shard(&f, "engine-a", 3, Duration::minutes(5)).await;
    let elsewhere = Lease {
        shard: Shard::new((mine.shard.index() + 1) % 64).expect("shard"),
        generation: mine.generation,
    };
    let refused = f
        .fire(fenced(f.admission(nominal()), "engine-a", elsewhere))
        .await
        .expect_err("not this schedule's shard");
    assert!(matches!(refused, AppError::StateConflict(_)), "{refused:?}");
}

#[tokio::test]
async fn a_scheduled_run_is_audited_as_started_by_the_schedule() {
    let f = fixture().await;
    let started = f.fire(f.admission(nominal())).await.expect("fire");
    let actor: String = sqlx::query_scalar(
        "SELECT actor FROM audit_records WHERE action = 'run.started' AND resource_id = $1",
    )
    .bind(started.info.run_id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, format!("system:schedule:{}", f.schedule.id));
}

#[tokio::test]
async fn the_provenance_of_a_run_never_changes() {
    let f = fixture().await;
    let started = f.fire(f.admission(nominal())).await.expect("fire");
    let error = sqlx::query("UPDATE runs SET schedule_nominal_at = schedule_nominal_at + interval '1 hour' WHERE id = $1")
        .bind(started.info.run_id.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect_err("immutable");
    assert!(error.to_string().contains("provenance"), "{error}");
    // and an ordinary update of the run still works
    sqlx::query("UPDATE runs SET updated_at = now() WHERE id = $1")
        .bind(started.info.run_id.as_uuid())
        .execute(&f.w.db.pool)
        .await
        .expect("other columns are not affected");
}

const BEFORE_PROVENANCE: i64 = 20_261_230_000_100;

async fn provenance_columns(f: &Fixture) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_name = 'runs' AND column_name LIKE 'schedule\\_%'",
    )
    .fetch_one(&f.w.db.pool)
    .await
    .expect("columns")
}

#[tokio::test]
async fn a_rollback_is_refused_while_a_run_carries_the_provenance_of_a_schedule() {
    let f = fixture().await;
    let started = f.fire(f.admission(nominal())).await.expect("fire");
    assert_eq!(provenance_columns(&f).await, 4);

    let refused = crate::migration::undo_to(&f.w.db, BEFORE_PROVENANCE)
        .await
        .expect_err("a run keeps saying which schedule started it");
    assert!(
        format!("{refused:?}").contains("retained") || format!("{refused:?}").contains("55000"),
        "{refused:?}"
    );
    assert_eq!(provenance_columns(&f).await, 4, "nothing was dropped");

    // an operator who accepts the loss clears the provenance first; the trigger that keeps it fixed
    // is switched off on purpose, in the same transaction
    let mut tx = f.w.db.pool.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE runs DISABLE TRIGGER runs_schedule_provenance",
        "UPDATE runs SET schedule_id = NULL, schedule_nominal_at = NULL, \
         schedule_action_kind = NULL, schedule_revision = NULL WHERE schedule_id IS NOT NULL",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    tx.commit().await.expect("the drain commits");
    crate::migration::undo_to(&f.w.db, BEFORE_PROVENANCE)
        .await
        .expect("rollback with no provenance left");
    assert_eq!(provenance_columns(&f).await, 0);
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE id = $1")
        .bind(started.info.run_id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("run");
    assert_eq!(kept, 1, "the run itself is kept");

    sagat_infra_postgres::migrate(&f.w.db.pool)
        .await
        .expect("forward again with a run present");
    assert_eq!(provenance_columns(&f).await, 4);
    let provenance: Option<Uuid> = sqlx::query_scalar("SELECT schedule_id FROM runs WHERE id = $1")
        .bind(started.info.run_id.as_uuid())
        .fetch_one(&f.w.db.pool)
        .await
        .expect("run");
    assert_eq!(
        provenance, None,
        "a run that exists is not given a schedule"
    );
}

/// The constraint that refuses `statement` once the trigger that would refuse it first is off.
async fn refused_by(f: &Fixture, run: Uuid, statement: &'static str) -> Option<String> {
    let mut tx = f.w.db.pool.begin().await.expect("begin");
    sqlx::query("ALTER TABLE runs DISABLE TRIGGER runs_schedule_provenance")
        .execute(&mut *tx)
        .await
        .expect("trigger off, for this transaction only");
    match sqlx::query(statement).bind(run).execute(&mut *tx).await {
        Ok(_) => None,
        Err(sqlx::Error::Database(e)) => e.constraint().map(str::to_owned),
        Err(other) => panic!("not a database error: {other}"),
    }
}

#[tokio::test]
async fn the_provenance_of_a_run_is_all_or_nothing_and_of_a_known_kind() {
    let f = fixture().await;
    let started = f.fire(f.admission(nominal())).await.expect("fire");
    let run = started.info.run_id.as_uuid();
    for (statement, constraint) in [
        (
            "UPDATE runs SET schedule_revision = NULL WHERE id = $1",
            "runs_schedule_provenance_whole",
        ),
        (
            "UPDATE runs SET schedule_id = NULL WHERE id = $1",
            "runs_schedule_provenance_whole",
        ),
        (
            "UPDATE runs SET schedule_action_kind = 'WHENEVER' WHERE id = $1",
            "runs_schedule_action_kind_valid",
        ),
        (
            "UPDATE runs SET schedule_revision = 0 WHERE id = $1",
            "runs_schedule_revision_positive",
        ),
    ] {
        assert_eq!(
            refused_by(&f, run, statement).await.as_deref(),
            Some(constraint),
            "{statement}"
        );
    }
}
