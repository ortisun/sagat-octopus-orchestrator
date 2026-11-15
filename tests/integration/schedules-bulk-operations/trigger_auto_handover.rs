//! The automatic handover of the cron triggers (`T018-082`, `T018-084`, `T018-085`, ADR-0091): `control-plane
//! schedule-trigger-handover --auto`, run as the role `sagat_handover` (every test of this file connects the job as that role,
//! with its password set apart from the URL, D2273), against real PostgreSQL. The database decides: the round function computes
//! every condition and counts the stable rounds, the wrapper computes them again under the epoch lock.
//!
//! Nothing here sleeps for a condition: the waits of the job are instant ([`Clockwork`]) and the times the database counts by
//! (the first round, the last stable round, the marks of the reconciliation, the memberships) are moved back as the owner. Races are
//! held by an open transaction or a held lock and watched in `pg_stat_activity`.
#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sagat_application::instances::{
    Capability, InstanceKind, InstanceStore, Registered, Registration,
};
use sagat_application::schedule_triggers::HandoverBlock;
use sagat_control_plane::RunError;
use sagat_control_plane::config::DatabaseConfig;
use sagat_control_plane::handover::{HandoverOutcome, HandoverRequest, schedule_trigger_handover};
use sagat_control_plane::handover_auto::{
    AutoOutcome, AutoRequest, Interrupt, Pace, Paced, auto_handover,
};
use sagat_infra_postgres::PgInstanceStore;
use sqlx::PgPool;
use uuid::Uuid;

use crate::support::instances::TEST_TOKEN;
use crate::support::trigger_schedules::{cron_workflow, triggers};
use crate::support::{
    HANDOVER_ROLE, HANDOVER_ROLE_PASSWORD, RUNTIME_ROLE, TestDb, as_runtime_role, migrated_db,
};

// --- the fixture ----------------------------------------------------------------------------------------------------------

/// A database and the pools the tests use: the owner (fixtures, backdating), the processes (the runtime role in the sweep) and the
/// job's role.
struct Fleet {
    owner: PgPool,
    process: PgInstanceStore,
    handover: PgPool,
    url: String,
}

impl Fleet {
    async fn on(db: &TestDb) -> Self {
        let process_pool = if as_runtime_role() {
            db.runtime_pool_with(4).await
        } else {
            db.pool.clone()
        };
        Self {
            owner: db.pool.clone(),
            process: PgInstanceStore::new(process_pool),
            handover: db.handover_pool().await,
            url: db.url_without_password(HANDOVER_ROLE),
        }
    }

    fn database(&self) -> DatabaseConfig {
        DatabaseConfig {
            url: self.url.clone(),
            max_connections: 2,
            statement_timeout: Duration::from_secs(5),
        }
    }

    async fn register(
        &self,
        kind: InstanceKind,
        owner: Option<&str>,
        capabilities: &[Capability],
    ) -> Uuid {
        let instance = Uuid::new_v4();
        let got = self
            .process
            .register(&Registration {
                instance,
                token: &TEST_TOKEN,
                kind,
                scheduler_owner: owner,
                binary_version: "0.3.0-test",
                capabilities,
            })
            .await
            .expect("register");
        assert!(matches!(got, Registered::Serving { .. }), "{got:?}");
        instance
    }

    async fn engine(&self) -> Uuid {
        self.register(
            InstanceKind::ExecutionEngine,
            None,
            &[Capability::SchedulesV1],
        )
        .await
    }

    async fn control_plane(&self) -> Uuid {
        self.register(InstanceKind::ControlPlane, None, &[]).await
    }

    /// `engines` engines that list `schedules-v1` and one control plane: a fleet the job hands over.
    async fn ready(&self, engines: usize) -> Vec<Uuid> {
        let mut ids = Vec::new();
        for _ in 0..engines {
            ids.push(self.engine().await);
        }
        self.control_plane().await;
        ids
    }

    async fn sql(&self, statement: &str) {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&self.owner)
            .await
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }

    /// A scheduler membership as a 0.2 engine's keeper writes it, `expires` seconds from now (negative: expired).
    async fn membership(&self, owner: &str, expires: i64) {
        sqlx::query(
            "INSERT INTO scheduler_members (owner, started_at, expires_at) \
             VALUES ($1, clock_timestamp() - interval '1 minute', clock_timestamp() + make_interval(secs => $2::float8))",
        )
        .bind(owner)
        .bind(expires)
        .execute(&self.owner)
        .await
        .expect("membership");
    }

    async fn handover_row(
        &self,
    ) -> (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    ) {
        sqlx::query_as(
            "SELECT state, changed_by, changed_db_role, changed_mode, changed_epoch FROM schedule_trigger_handover",
        )
        .fetch_one(&self.owner)
        .await
        .expect("handover row")
    }

    async fn count(&self, query: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(query.to_owned()))
            .fetch_one(&self.owner)
            .await
            .unwrap_or_else(|error| panic!("{query}: {error}"))
    }

    async fn last_reason(&self, run: Uuid) -> Option<String> {
        sqlx::query_scalar(
            "SELECT last_reason FROM schedule_trigger_handover_attempts WHERE run_id = $1",
        )
        .bind(run)
        .fetch_optional(&self.owner)
        .await
        .expect("attempt")
        .flatten()
    }

    /// One round as the job makes it, through the round function, as `sagat_handover`.
    async fn round(&self, run: Uuid) -> (String, Option<String>, i32) {
        sqlx::query_as(
            "SELECT out_outcome, out_reason, out_stable \
               FROM schedule_trigger_handover_round($1, 3, interval '10 minutes', 1, interval '2 hours')",
        )
        .bind(run)
        .fetch_one(&self.handover)
        .await
        .expect("round")
    }

    /// Makes the run's rounds look old enough: the first round a horizon ago and the last stable one a heartbeat ago.
    async fn age_run(&self, run: Uuid) {
        sqlx::query(
            "UPDATE schedule_trigger_handover_attempts \
                SET first_round_at = least(first_round_at, clock_timestamp() - horizon - interval '1 minute'), \
                    last_stable_at = last_stable_at - interval '16 seconds' \
              WHERE run_id = $1",
        )
        .bind(run)
        .execute(&self.owner)
        .await
        .expect("age the run");
    }

    /// Three stable rounds of `run`, a horizon after its first: what the wrapper requires.
    async fn stable(&self, run: Uuid) {
        for _ in 0..3 {
            let _ = self.round(run).await;
            self.age_run(run).await;
        }
        let (outcome, reason, stable) = self.round(run).await;
        assert_eq!(outcome, "STABLE", "{reason:?} after {stable} round(s)");
    }

    /// The wrapper, called alone, as `sagat_handover`.
    async fn wrapper(&self, run: Uuid) -> (String, Option<String>) {
        sqlx::query_as("SELECT out_outcome, out_reason FROM schedule_trigger_handover_auto($1)")
            .bind(run)
            .fetch_one(&self.handover)
            .await
            .expect("wrapper")
    }
}

fn request(run: Uuid) -> AutoRequest {
    AutoRequest {
        ask: sagat_infra_postgres::AutoAsk {
            run,
            stable: 3,
            horizon: Duration::from_secs(600),
            min_live: 1,
            wait: Duration::from_secs(7_200),
        },
        retire_role: false,
        dry_run: false,
        operator: Some("release-test".to_owned()),
        activation: sagat_infra_postgres::OperatorBounds::default(),
    }
}

/// Bounds of the activation that let a test hold it on a lock it watches, instead of racing the two seconds of its lock wait.
const HELD: sagat_infra_postgres::OperatorBounds = sagat_infra_postgres::OperatorBounds {
    lock_timeout: Duration::from_secs(120),
    statement_timeout: Duration::from_secs(120),
    step: Duration::from_secs(120),
};

/// Something the test does after the n-th wait of the job.
type Step = Box<dyn Fn(usize, Option<&str>) -> Option<String> + Send + Sync>;

/// The waits of the job, instant. Every wait records the reason of the round before it, moves the run's times back so that the
/// next round counts (unless `keep_first_round`), runs what the test scripted for it, and raises the interrupt after `max_waits`
/// so that a job that never hands over ends the test instead of hanging it.
struct Clockwork {
    owner: PgPool,
    run: Uuid,
    keep_first_round: bool,
    reasons: Mutex<Vec<Option<String>>>,
    /// What the job asked to wait each time (the spacing, the backoff, the clamp to its deadline).
    asked: Mutex<Vec<Duration>>,
    waits: AtomicUsize,
    max_waits: usize,
    step: Option<Step>,
    /// Holds the job between its stable round and its activation: it arrives, then waits to be released.
    gate: Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>,
    /// Stands for SIGTERM at the activation's barrier.
    interrupt_at_activation: Option<Interrupt>,
}

impl Clockwork {
    fn new(fleet: &Fleet, run: Uuid) -> Self {
        Self {
            owner: fleet.owner.clone(),
            run,
            keep_first_round: false,
            reasons: Mutex::new(Vec::new()),
            asked: Mutex::new(Vec::new()),
            waits: AtomicUsize::new(0),
            max_waits: 40,
            step: None,
            gate: None,
            interrupt_at_activation: None,
        }
    }

    fn reasons(&self) -> Vec<Option<String>> {
        self.reasons.lock().expect("reasons").clone()
    }

    fn asked(&self) -> Vec<Duration> {
        self.asked.lock().expect("asked").clone()
    }
}

impl Pace for Clockwork {
    fn wait<'a>(&'a self, wait: Duration, interrupt: &'a Interrupt) -> Paced<'a> {
        Box::pin(async move {
            self.asked.lock().expect("asked").push(wait);
            let n = self.waits.fetch_add(1, Ordering::SeqCst) + 1;
            let reason: Option<String> = sqlx::query_scalar(
                "SELECT last_reason FROM schedule_trigger_handover_attempts WHERE run_id = $1",
            )
            .bind(self.run)
            .fetch_optional(&self.owner)
            .await
            .expect("reason")
            .flatten();
            self.reasons.lock().expect("reasons").push(reason.clone());
            let first = if self.keep_first_round {
                "first_round_at"
            } else {
                "least(first_round_at, clock_timestamp() - horizon - interval '1 minute')"
            };
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE schedule_trigger_handover_attempts SET first_round_at = {first}, \
                        last_stable_at = last_stable_at - interval '16 seconds' WHERE run_id = $1"
            )))
            .bind(self.run)
            .execute(&self.owner)
            .await
            .expect("move the run back");
            if let Some(step) = &self.step
                && let Some(statement) = step(n, reason.as_deref())
            {
                sqlx::query(sqlx::AssertSqlSafe(statement.clone()))
                    .execute(&self.owner)
                    .await
                    .unwrap_or_else(|error| panic!("{statement}: {error}"));
            }
            if n >= self.max_waits {
                interrupt.raise();
            }
        })
    }

    fn before_activation(&self) -> Paced<'_> {
        Box::pin(async move {
            if let Some(interrupt) = &self.interrupt_at_activation {
                interrupt.raise();
            }
            if let Some((arrive, release)) = &self.gate {
                arrive.wait().await;
                release.wait().await;
            }
        })
    }
}

async fn job(
    fleet: &Fleet,
    request: &AutoRequest,
    pace: &Clockwork,
) -> Result<AutoOutcome, RunError> {
    tokio::time::timeout(
        Duration::from_secs(240),
        auto_handover(
            &fleet.database(),
            Some(HANDOVER_ROLE_PASSWORD),
            request,
            pace,
            &Interrupt::default(),
        ),
    )
    .await
    .expect("the job ends")
}

async fn job_with(
    fleet: &Fleet,
    request: &AutoRequest,
    pace: &Clockwork,
    interrupt: &Interrupt,
) -> Result<AutoOutcome, RunError> {
    tokio::time::timeout(
        Duration::from_secs(240),
        auto_handover(
            &fleet.database(),
            Some(HANDOVER_ROLE_PASSWORD),
            request,
            pace,
            interrupt,
        ),
    )
    .await
    .expect("the job ends")
}

fn exit_code(result: &Result<AutoOutcome, RunError>) -> u8 {
    match result {
        Ok(_) => 0,
        Err(error) => error.exit_code(),
    }
}

async fn waiting_on_locks(owner: &PgPool, n: i64) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                  WHERE datname = current_database() AND pid <> pg_backend_pid() AND wait_event_type = 'Lock'",
            )
            .fetch_one(owner)
            .await
            .expect("pg_stat_activity");
            if waiting >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the lock waiters appear");
}

// --- the happy path, the mode and the actor -------------------------------------------------------------------------------

#[tokio::test]
async fn the_job_hands_over_once_the_fleet_is_stable_a_horizon_after_its_first_round_as_automatic()
{
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let engines = fleet.ready(2).await;
    let run = Uuid::new_v4();
    let pace = Clockwork::new(&fleet, run);

    let done = job(&fleet, &request(run), &pace)
        .await
        .expect("handed over");
    let AutoOutcome::HandedOver { live, epoch, .. } = done else {
        panic!("{done:?}")
    };
    assert_eq!(live, 2);
    // the rounds are a heartbeat apart
    assert!(
        !pace.asked().is_empty()
            && pace
                .asked()
                .iter()
                .all(|wait| *wait == Duration::from_secs(15)),
        "{:?}",
        pace.asked()
    );
    // the first round is within the first horizon; the next two count
    assert_eq!(
        pace.reasons().first().cloned().flatten().as_deref(),
        Some("FIRST_HORIZON")
    );

    let (state, by, role, mode, changed_epoch) = fleet.handover_row().await;
    assert_eq!(state, "HANDED_OVER");
    assert_eq!(
        by,
        Some(format!("system:handover-job:{run}")),
        "the actor is the run's, not an argument"
    );
    assert_eq!(role.as_deref(), Some(HANDOVER_ROLE));
    assert_eq!(
        mode.as_deref(),
        Some("AUTOMATIC"),
        "the mode comes from the session_user"
    );
    assert_eq!(changed_epoch, Some(epoch));
    let (actor, user): (String, String) = sqlx::query_as(
        "SELECT actor, database_user FROM platform_requirement_history WHERE capability = 'schedules-v1'",
    )
    .fetch_one(&fleet.owner)
    .await
    .expect("one history row");
    assert_eq!(
        (actor, user.as_str()),
        (format!("system:handover-job:{run}"), HANDOVER_ROLE)
    );
    // the census copy holds every serving instance seen within the horizon
    let census: Vec<Uuid> =
        sqlx::query_scalar("SELECT instance_id FROM schedule_trigger_handover_census WHERE epoch = $1 AND kind = 'EXECUTION_ENGINE' ORDER BY 1")
            .bind(epoch)
            .fetch_all(&fleet.owner)
            .await
            .expect("census");
    let mut expected = engines;
    expected.sort();
    assert_eq!(census, expected);
    assert_eq!(
        fleet
            .count(
                "SELECT count(*) FROM schedule_trigger_handover_census WHERE kind = 'CONTROL_PLANE'"
            )
            .await,
        1
    );

    // a second run finds it handed over and exits 0
    let again = job(
        &fleet,
        &request(Uuid::new_v4()),
        &Clockwork::new(&fleet, run),
    )
    .await;
    assert!(
        matches!(again, Ok(AutoOutcome::AlreadyHandedOver { .. })),
        "{again:?}"
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirement_history")
            .await,
        1
    );
}

#[tokio::test]
async fn a_fleet_with_no_backlog_waits_a_horizon_after_the_first_round_and_an_interrupt_ends_it_with_nothing_changed()
 {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    let mut pace = Clockwork::new(&fleet, run);
    pace.keep_first_round = true;
    pace.max_waits = 5;

    let result = job(&fleet, &request(run), &pace).await;
    assert_eq!(exit_code(&result), 4, "{result:?}");
    assert!(
        pace.reasons()
            .iter()
            .all(|reason| reason.as_deref() == Some("FIRST_HORIZON")),
        "{:?}",
        pace.reasons()
    );
    assert_eq!(fleet.last_reason(run).await.as_deref(), Some("INTERRUPTED"));
    assert_eq!(fleet.handover_row().await.0, "LEGACY");
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirements")
            .await,
        0
    );
}

// --- the detectors --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_live_and_a_recently_expired_membership_no_engine_names_block_engine_unregistered() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();

    fleet.membership("engine-0.2-live", 15).await;
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_UNREGISTERED")
    );
    fleet.sql("DELETE FROM scheduler_members").await;
    // expired five minutes ago, within the horizon: it still counts
    fleet.membership("engine-0.2-expired", -300).await;
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_UNREGISTERED")
    );
    // expired beyond the horizon: no longer
    fleet
        .sql("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '11 minutes'")
        .await;
    assert_ne!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_UNREGISTERED")
    );
    // a membership a registered engine names is that engine's
    fleet.membership("engine-0.3", 15).await;
    fleet
        .register(
            InstanceKind::ExecutionEngine,
            Some("engine-0.3"),
            &[Capability::SchedulesV1],
        )
        .await;
    assert_ne!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_UNREGISTERED")
    );
}

#[tokio::test]
async fn an_engine_without_the_capability_within_the_horizon_too_few_engines_and_no_control_plane_each_block()
 {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let run = Uuid::new_v4();
    // nothing live
    assert_eq!(fleet.round(run).await.1.as_deref(), Some("TOO_FEW_ENGINES"));
    let crashed = fleet
        .register(InstanceKind::ExecutionEngine, None, &[])
        .await;
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_LACKS_CAPABILITY")
    );
    // it crashed five minutes ago without schedules-v1: within the horizon it still blocks
    sqlx::query("UPDATE serving_instances SET last_seen = clock_timestamp() - interval '5 minutes' WHERE instance_id = $1")
        .bind(crashed)
        .execute(&fleet.owner)
        .await
        .expect("age");
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_LACKS_CAPABILITY")
    );
    sqlx::query("UPDATE serving_instances SET last_seen = clock_timestamp() - interval '11 minutes' WHERE instance_id = $1")
        .bind(crashed)
        .execute(&fleet.owner)
        .await
        .expect("age");
    fleet.engine().await;
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("NO_CONTROL_PLANE")
    );
    fleet.control_plane().await;
    assert_eq!(fleet.round(run).await.1.as_deref(), Some("FIRST_HORIZON"));

    // --min-live, floored at one, is the run's from its first round
    let strict = Uuid::new_v4();
    let (_, reason, _): (String, Option<String>, i32) = sqlx::query_as(
        "SELECT out_outcome, out_reason, out_stable FROM schedule_trigger_handover_round($1, 3, interval '10 minutes', 2, interval '2 hours')",
    )
    .bind(strict)
    .fetch_one(&fleet.handover)
    .await
    .expect("round");
    assert_eq!(reason.as_deref(), Some("TOO_FEW_ENGINES"));
    let (min_live, stable, horizon): (i32, i32, f64) = sqlx::query_as(
        "SELECT min_live, stable_wanted, extract(epoch FROM horizon)::float8 FROM schedule_trigger_handover_attempts WHERE run_id = $1",
    )
    .bind(strict)
    .fetch_one(&fleet.owner)
    .await
    .expect("attempt");
    assert_eq!((min_live, stable, horizon), (2, 3, 600.0));
    // a weaker ask is floored by the database
    let weak = Uuid::new_v4();
    sqlx::query("SELECT * FROM schedule_trigger_handover_round($1, 1, interval '1 minute', 0, interval '48 hours')")
        .bind(weak)
        .execute(&fleet.handover)
        .await
        .expect("round");
    let (min_live, stable, horizon, wait): (i32, i32, f64, f64) = sqlx::query_as(
        "SELECT min_live, stable_wanted, extract(epoch FROM horizon)::float8, extract(epoch FROM deadline - first_round_at)::float8 \
           FROM schedule_trigger_handover_attempts WHERE run_id = $1",
    )
    .bind(weak)
    .fetch_one(&fleet.owner)
    .await
    .expect("attempt");
    assert_eq!((min_live, stable, horizon, wait), (1, 3, 600.0, 86_400.0));
    let too_far = sqlx::query("SELECT * FROM schedule_trigger_handover_round($1, 3, interval '2 hours', 1, interval '2 hours')")
        .bind(Uuid::new_v4())
        .execute(&fleet.handover)
        .await
        .expect_err("a horizon over an hour");
    assert_eq!(crate::support::db_error(too_far).0, "22023");
}

/// The mark the job waits a horizon after: set by every change of the reconciliation (make, respec, resume, remove, revive), never by
/// a publication or by planning a published schedule (D2243, D2258).
#[tokio::test]
async fn each_change_of_the_reconciliation_marks_the_schedule_and_a_publication_does_not() {
    let t = triggers().await;
    t.publish(cron_workflow("marked", "0 * * * *", "UTC")).await;
    let pool = t.w.db.pool.clone();
    let marked = || {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM schedules WHERE reconciled_at IS NOT NULL",
            )
            .fetch_one(&pool)
            .await
            .expect("marks")
        }
    };
    let clear = || {
        let pool = pool.clone();
        async move {
            sqlx::query("UPDATE schedules SET reconciled_at = NULL")
                .execute(&pool)
                .await
                .expect("clear");
        }
    };
    assert_eq!(
        marked().await,
        0,
        "a publication makes the schedule and leaves no mark"
    );
    assert!(t.reconcile().await > 0, "the published schedule is planned");
    assert_eq!(
        marked().await,
        0,
        "planning a published schedule is not a change of the reconciliation"
    );

    // make: an older control plane publishes a trigger and makes no schedule
    let old = Uuid::now_v7();
    crate::trigger_reconcile::old_control_plane_publishes(&t, "marked", old, "15 * * * *", "UTC")
        .await;
    t.reconcile().await;
    let mark: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT reconciled_at FROM schedules WHERE trigger_id = $1")
            .bind(old)
            .fetch_one(&pool)
            .await
            .expect("made");
    assert!(mark.is_some(), "make");
    assert_eq!(marked().await, 1);
    clear().await;

    // respec: the trigger's expression is not the schedule's
    sqlx::query("UPDATE workflow_triggers SET cron = '30 * * * *' WHERE id = $1")
        .bind(old)
        .execute(&pool)
        .await
        .expect("drift");
    t.reconcile().await;
    assert_eq!(marked().await, 1, "respec");
    clear().await;

    // resume: a schedule paused while LEGACY
    sqlx::query("UPDATE schedules SET state = 'PAUSED' WHERE trigger_id = $1")
        .bind(old)
        .execute(&pool)
        .await
        .expect("pause");
    t.reconcile().await;
    assert_eq!(marked().await, 1, "resume");
    clear().await;

    // remove: the older control plane drops the trigger
    sqlx::query("DELETE FROM workflow_triggers WHERE id = $1")
        .bind(old)
        .execute(&pool)
        .await
        .expect("dropped");
    t.reconcile().await;
    let state: String = sqlx::query_scalar("SELECT state FROM schedules WHERE trigger_id = $1")
        .bind(old)
        .fetch_one(&pool)
        .await
        .expect("tombstone");
    assert_eq!(state, "DELETED");
    assert_eq!(marked().await, 1, "remove");
    clear().await;

    // revive: the trigger is in force again over its tombstone
    crate::trigger_reconcile::old_control_plane_publishes(&t, "marked", old, "15 * * * *", "UTC")
        .await;
    t.reconcile().await;
    assert_eq!(marked().await, 1, "revive");
    assert!(t.drift().await.is_empty());
}

#[tokio::test]
async fn work_of_the_reconciliation_within_the_horizon_blocks_and_a_horizon_later_does_not() {
    let t = triggers().await;
    t.publish(cron_workflow("recent", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet
        .sql("UPDATE schedules SET reconciled_at = clock_timestamp() - interval '5 minutes'")
        .await;
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("RECONCILIATION_RECENT")
    );
    fleet
        .sql("UPDATE schedules SET reconciled_at = clock_timestamp() - interval '11 minutes'")
        .await;
    assert_eq!(fleet.round(run).await.1.as_deref(), Some("FIRST_HORIZON"));
    // a trigger with no schedule is unconverged
    fleet
        .sql("UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL")
        .await;
    assert_eq!(fleet.round(run).await.1.as_deref(), Some("UNCONVERGED"));
}

// --- the departure case ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_engine_that_stops_between_two_rounds_is_fleet_changing_and_the_count_starts_over() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let engines = fleet.ready(2).await;
    let run = Uuid::new_v4();
    fleet.round(run).await;
    fleet.age_run(run).await;
    let (_, _, stable) = fleet.round(run).await;
    assert_eq!(stable, 2);
    fleet.age_run(run).await;
    // a clean stop deletes its row
    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(engines[0])
        .execute(&fleet.owner)
        .await
        .expect("stop");
    let (outcome, reason, stable) = fleet.round(run).await;
    assert_eq!(
        (outcome.as_str(), reason.as_deref(), stable),
        ("BLOCKED", Some("FLEET_CHANGING"), 1)
    );
}

#[tokio::test]
async fn an_engine_that_departs_between_the_last_round_and_the_lock_is_refused_under_the_lock_and_nothing_is_recorded()
 {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let engines = fleet.ready(2).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(engines[1])
        .execute(&fleet.owner)
        .await
        .expect("stop");
    assert_eq!(
        fleet.wrapper(run).await,
        ("REFUSED".to_owned(), Some("FLEET_CHANGING".to_owned()))
    );
    assert_eq!(fleet.handover_row().await.0, "LEGACY");
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirement_history")
            .await,
        0
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM schedule_trigger_handover_census")
            .await,
        0
    );
}

#[tokio::test]
async fn an_engine_departing_while_the_job_is_held_before_its_activation_makes_it_retry_and_hand_over_with_the_rest()
 {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let engines = fleet.ready(2).await;
    let run = Uuid::new_v4();
    let arrive = Arc::new(tokio::sync::Barrier::new(2));
    let release = Arc::new(tokio::sync::Barrier::new(2));
    let mut pace = Clockwork::new(&fleet, run);
    pace.gate = Some((Arc::clone(&arrive), Arc::clone(&release)));
    let pace = Arc::new(pace);
    let task = {
        let pace = Arc::clone(&pace);
        let database = fleet.database();
        tokio::spawn(async move {
            auto_handover(
                &database,
                Some(HANDOVER_ROLE_PASSWORD),
                &request(run),
                &*pace,
                &Interrupt::default(),
            )
            .await
        })
    };
    arrive.wait().await;
    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(engines[0])
        .execute(&fleet.owner)
        .await
        .expect("stop");
    release.wait().await;
    // the next stable round brings it back to the barrier, with one engine
    arrive.wait().await;
    assert_eq!(
        fleet.handover_row().await.0,
        "LEGACY",
        "the first activation was refused under the lock"
    );
    release.wait().await;
    let done = tokio::time::timeout(Duration::from_secs(240), task)
        .await
        .expect("ends")
        .expect("joined")
        .expect("handed over");
    assert!(
        matches!(done, AutoOutcome::HandedOver { live: 1, .. }),
        "{done:?}"
    );
}

// --- the races ------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn two_jobs_at_once_hand_over_once_with_one_history_row_one_census_one_tenant_set_and_one_audit_row_per_tenant()
 {
    let t = triggers().await;
    t.publish(cron_workflow("race-a", "0 * * * *", "UTC")).await;
    t.w.publish_as_other_tenant(cron_workflow("race-b", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.ready(1).await;
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    let arrive = Arc::new(tokio::sync::Barrier::new(3));
    let release = Arc::new(tokio::sync::Barrier::new(3));
    let mut tasks = Vec::new();
    for run in [first, second] {
        let mut pace = Clockwork::new(&fleet, run);
        pace.gate = Some((Arc::clone(&arrive), Arc::clone(&release)));
        let database = fleet.database();
        tasks.push(tokio::spawn(async move {
            auto_handover(
                &database,
                Some(HANDOVER_ROLE_PASSWORD),
                &request(run),
                &pace,
                &Interrupt::default(),
            )
            .await
        }));
    }
    // both are stable and held before their activations; released together, they serialise on the epoch: the handover row is
    // held so that the order is seen, one job on the row with the epoch, the other on the epoch (correctness review)
    arrive.wait().await;
    let mut row = fleet.owner.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM schedule_trigger_handover FOR UPDATE")
        .execute(&mut *row)
        .await
        .expect("hold the row");
    release.wait().await;
    waiting_on_locks(&fleet.owner, 2).await;
    row.commit().await.expect("release the row");
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(
            tokio::time::timeout(Duration::from_secs(240), task)
                .await
                .expect("ends")
                .expect("joined")
                .expect("exit 0"),
        );
    }
    let made = outcomes
        .iter()
        .filter(|o| matches!(o, AutoOutcome::HandedOver { .. }))
        .count();
    let already = outcomes
        .iter()
        .filter(|o| matches!(o, AutoOutcome::AlreadyHandedOver { .. }))
        .count();
    assert_eq!((made, already), (1, 1), "{outcomes:?}");
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirement_history")
            .await,
        1
    );
    assert_eq!(
        fleet
            .count("SELECT count(DISTINCT epoch) FROM schedule_trigger_handover_census")
            .await,
        1
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM schedule_trigger_handover_tenants")
            .await,
        2
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM audit_records WHERE action = 'schedule.trigger_handover'")
            .await,
        2,
        "one audit record per tenant with a cron trigger"
    );
}

#[tokio::test]
async fn the_job_and_the_operator_at_once_make_one_handover() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    // the operator holds the epoch in its activation
    let mut operator = fleet.owner.begin().await.expect("begin");
    sqlx::query("SELECT platform_requirement_lock()")
        .execute(&mut *operator)
        .await
        .expect("epoch");
    let wrapper = {
        let pool = fleet.handover.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            sqlx::query("SET LOCAL lock_timeout = '60s'")
                .execute(&mut *tx)
                .await
                .expect("bound");
            let got: (String, Option<String>) = sqlx::query_as(
                "SELECT out_outcome, out_reason FROM schedule_trigger_handover_auto($1)",
            )
            .bind(run)
            .fetch_one(&mut *tx)
            .await
            .expect("wrapper");
            tx.commit().await.expect("commit");
            got
        })
    };
    waiting_on_locks(&fleet.owner, 1).await;
    // the operator's own handover in its transaction: the requirement and the state, then the commit
    sqlx::query("SELECT * FROM platform_requirement_activate('schedules-v1', ARRAY['EXECUTION_ENGINE'], 'ana')")
        .execute(&mut *operator)
        .await
        .expect("activate");
    sqlx::query("SELECT schedule_trigger_handover_complete('ana')")
        .execute(&mut *operator)
        .await
        .expect("complete");
    operator.commit().await.expect("commit");
    let (outcome, _) = wrapper.await.expect("joined");
    assert_eq!(outcome, "ALREADY_HANDED_OVER");
    let (_, by, _, mode, _) = fleet.handover_row().await;
    assert_eq!(
        (by.as_deref(), mode.as_deref()),
        (Some("ana"), Some("OPERATOR"))
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirement_history")
            .await,
        1
    );
    assert_eq!(
        fleet
            .count("SELECT count(DISTINCT epoch) FROM schedule_trigger_handover_census")
            .await,
        1
    );
}

#[tokio::test]
async fn an_engine_registering_without_the_capability_during_the_check_both_orders() {
    // the registration first: it holds the epoch FOR SHARE, the wrapper waits for it and then sees the engine
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    let mut registering = fleet.owner.begin().await.expect("begin");
    sqlx::query("SELECT * FROM instance_register($1, $2, 'EXECUTION_ENGINE', NULL, '0.3.0', '{}')")
        .bind(Uuid::new_v4())
        .bind(vec![7_u8; 32])
        .execute(&mut *registering)
        .await
        .expect("register");
    let wrapper = {
        let pool = fleet.handover.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            sqlx::query("SET LOCAL lock_timeout = '60s'")
                .execute(&mut *tx)
                .await
                .expect("bound");
            let got: (String, Option<String>) = sqlx::query_as(
                "SELECT out_outcome, out_reason FROM schedule_trigger_handover_auto($1)",
            )
            .bind(run)
            .fetch_one(&mut *tx)
            .await
            .expect("wrapper");
            tx.commit().await.expect("commit");
            got
        })
    };
    waiting_on_locks(&fleet.owner, 1).await;
    registering.commit().await.expect("registered");
    let got = wrapper.await.expect("joined");
    assert_eq!(
        got,
        (
            "REFUSED".to_owned(),
            Some("ENGINE_LACKS_CAPABILITY".to_owned())
        )
    );
    assert_eq!(fleet.handover_row().await.0, "LEGACY");

    // the wrapper first: held on the triggers' table with the epoch taken, the registration waits for it and is then refused
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    // the handover row held (the wrapper takes it after the epoch and the triggers' table, and waits for it with no bound of its
    // own), so the order is held by a lock and not raced against the two seconds of the table's
    let mut publication = fleet.owner.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM schedule_trigger_handover FOR UPDATE")
        .execute(&mut *publication)
        .await
        .expect("the handover row held");
    let wrapper = {
        let pool = fleet.handover.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            let got: (String, Option<String>) = sqlx::query_as(
                "SELECT out_outcome, out_reason FROM schedule_trigger_handover_auto($1)",
            )
            .bind(run)
            .fetch_one(&mut *tx)
            .await
            .expect("wrapper");
            tx.commit().await.expect("commit");
            got
        })
    };
    waiting_on_locks(&fleet.owner, 1).await;
    let registration = {
        let pool = fleet.owner.clone();
        tokio::spawn(async move {
            sqlx::query_scalar::<_, String>(
                "SELECT out_outcome FROM instance_register($1, $2, 'EXECUTION_ENGINE', NULL, '0.3.0', '{}')",
            )
            .bind(Uuid::new_v4())
            .bind(vec![8_u8; 32])
            .fetch_one(&pool)
            .await
            .expect("register")
        })
    };
    waiting_on_locks(&fleet.owner, 2).await;
    publication.commit().await.expect("the publication commits");
    assert_eq!(wrapper.await.expect("joined").0, "HANDED_OVER");
    assert_eq!(registration.await.expect("joined"), "CAPABILITY_MISSING");
}

// --- the wrapper alone, as psql would call it -----------------------------------------------------------------------------

#[tokio::test]
async fn the_wrapper_alone_refuses_each_condition_in_turn_and_records_nothing() {
    let t = triggers().await;
    t.publish(cron_workflow("alone", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    // no round at all
    assert_eq!(fleet.wrapper(run).await.1.as_deref(), Some("FIRST_HORIZON"));
    // rounds, but within the first horizon
    fleet.round(run).await;
    assert_eq!(fleet.wrapper(run).await.1.as_deref(), Some("FIRST_HORIZON"));
    // too few stable rounds
    fleet.age_run(run).await;
    fleet.round(run).await;
    assert_eq!(
        fleet.wrapper(run).await.1.as_deref(),
        Some("FLEET_CHANGING")
    );
    fleet.age_run(run).await;
    fleet.round(run).await;
    // now stable; each condition made false in turn under the lock
    let cases: [(&str, &str, &str); 4] = [
        (
            "INSERT INTO scheduler_members (owner, started_at, expires_at) VALUES ('old', clock_timestamp(), clock_timestamp() + interval '15 seconds')",
            "ENGINE_UNREGISTERED",
            "DELETE FROM scheduler_members",
        ),
        (
            "UPDATE schedules SET reconciled_at = clock_timestamp()",
            "RECONCILIATION_RECENT",
            "UPDATE schedules SET reconciled_at = NULL",
        ),
        (
            "UPDATE serving_instances SET last_seen = clock_timestamp() - interval '1 minute' WHERE kind = 'CONTROL_PLANE'",
            "NO_CONTROL_PLANE",
            "UPDATE serving_instances SET last_seen = clock_timestamp() WHERE kind = 'CONTROL_PLANE'",
        ),
        (
            "UPDATE schedules SET next_action_at = NULL, next_actual_at = NULL",
            "UNCONVERGED",
            "UPDATE schedules SET next_action_at = clock_timestamp() + interval '1 hour', next_actual_at = clock_timestamp() + interval '1 hour'",
        ),
    ];
    for (break_it, reason, mend) in cases {
        fleet.sql(break_it).await;
        assert_eq!(
            fleet.wrapper(run).await,
            ("REFUSED".to_owned(), Some(reason.to_owned())),
            "{reason}"
        );
        fleet.sql(mend).await;
    }
    // fewer than its --min-live live
    sqlx::query("UPDATE schedule_trigger_handover_attempts SET min_live = 2 WHERE run_id = $1")
        .bind(run)
        .execute(&fleet.owner)
        .await
        .expect("min live");
    assert_eq!(
        fleet.wrapper(run).await.1.as_deref(),
        Some("TOO_FEW_ENGINES")
    );
    sqlx::query("UPDATE schedule_trigger_handover_attempts SET min_live = 1 WHERE run_id = $1")
        .bind(run)
        .execute(&fleet.owner)
        .await
        .expect("min live");
    // a live set that changed since the streak
    let extra = fleet.engine().await;
    assert_eq!(
        fleet.wrapper(run).await.1.as_deref(),
        Some("FLEET_CHANGING")
    );
    sqlx::query("DELETE FROM serving_instances WHERE instance_id = $1")
        .bind(extra)
        .execute(&fleet.owner)
        .await
        .expect("stop");
    // a stable round older than a heartbeat is not spent later (D2271)
    sqlx::query("UPDATE schedule_trigger_handover_attempts SET last_stable_at = clock_timestamp() - interval '13 seconds' WHERE run_id = $1")
        .bind(run)
        .execute(&fleet.owner)
        .await
        .expect("stale");
    assert_eq!(
        fleet.wrapper(run).await.1.as_deref(),
        Some("FLEET_CHANGING")
    );
    assert_eq!(fleet.handover_row().await.0, "LEGACY");
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirement_history")
            .await,
        0,
        "nothing recorded"
    );
    // and fresh, it hands over
    sqlx::query("UPDATE schedule_trigger_handover_attempts SET last_stable_at = clock_timestamp() WHERE run_id = $1")
        .bind(run)
        .execute(&fleet.owner)
        .await
        .expect("fresh");
    assert_eq!(fleet.wrapper(run).await.0, "HANDED_OVER");
    let (_, by, _, mode, _) = fleet.handover_row().await;
    assert_eq!(
        (by, mode.as_deref()),
        (
            Some(format!("system:handover-job:{run}")),
            Some("AUTOMATIC")
        )
    );
}

#[tokio::test]
async fn the_wrapper_refuses_a_session_user_other_than_sagat_handover_and_the_runtime_role() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    // the owner
    let owner = sqlx::query("SELECT * FROM schedule_trigger_handover_auto($1)")
        .bind(run)
        .execute(&fleet.owner)
        .await
        .expect_err("the owner is refused");
    assert_eq!(crate::support::db_error(owner).0, "42501");
    // the runtime role cannot even execute it
    let runtime = db.runtime_pool().await;
    let refused = sqlx::query("SELECT * FROM schedule_trigger_handover_auto($1)")
        .bind(run)
        .execute(&runtime)
        .await
        .expect_err("the runtime role is refused");
    assert_eq!(crate::support::db_error(refused).0, "42501");
    assert_eq!(fleet.handover_row().await.0, "LEGACY");
}

// --- the forgeries of G2243: what the job then does -----------------------------------------------------------------------

#[tokio::test]
async fn the_forgeries_of_the_runtime_role_can_make_the_handover_early_and_never_record_it_themselves()
 {
    let t = triggers().await;
    t.publish(cron_workflow("forged", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.engine().await;
    let runtime = t.w.db.runtime_pool().await;
    let run = Uuid::new_v4();

    // a 0.2 membership, a recent mark and no control plane block the job
    fleet.membership("engine-0.2", 15).await;
    fleet
        .sql("UPDATE schedules SET reconciled_at = clock_timestamp()")
        .await;
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("ENGINE_UNREGISTERED")
    );

    // (a) the runtime role deletes the 0.2 membership: the detector no longer sees it (accepted, G2243)
    sqlx::query("DELETE FROM scheduler_members WHERE owner = 'engine-0.2'")
        .execute(&runtime)
        .await
        .expect("sagat_app writes scheduler_members");
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("RECONCILIATION_RECENT")
    );
    // (b) a membership shadowed by a registry row that names it: registered by the runtime role, with the capability
    fleet.membership("engine-0.2-shadowed", 15).await;
    let shadow = PgInstanceStore::new(runtime.clone());
    shadow
        .register(&Registration {
            instance: Uuid::new_v4(),
            token: &TEST_TOKEN,
            kind: InstanceKind::ExecutionEngine,
            scheduler_owner: Some("engine-0.2-shadowed"),
            binary_version: "0.3.0-forged",
            capabilities: &[Capability::SchedulesV1],
        })
        .await
        .expect("a forged registration");
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("RECONCILIATION_RECENT"),
        "the shadowed membership is not seen"
    );
    // (c) the runtime role clears the mark
    sqlx::query("UPDATE schedules SET reconciled_at = NULL")
        .execute(&runtime)
        .await
        .expect("sagat_app updates schedules");
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("NO_CONTROL_PLANE")
    );
    // (d) a control-plane row registered by the runtime role
    shadow
        .register(&Registration {
            instance: Uuid::new_v4(),
            token: &TEST_TOKEN,
            kind: InstanceKind::ControlPlane,
            scheduler_owner: None,
            binary_version: "0.3.0-forged",
            capabilities: &[],
        })
        .await
        .expect("a forged control plane");
    assert_eq!(fleet.round(run).await.1.as_deref(), Some("FIRST_HORIZON"));
    // so the job hands over early; but the runtime role itself can neither record the requirement nor move the state
    for statement in [
        "SELECT * FROM schedule_trigger_handover_auto(gen_random_uuid())",
        "SELECT * FROM platform_requirement_activate('schedules-v1', ARRAY['EXECUTION_ENGINE'], 'x')",
        "SELECT schedule_trigger_handover_complete('x')",
        "SELECT schedule_trigger_handover_record('x', 'AUTOMATIC', interval '10 minutes')",
        "SELECT * FROM schedule_trigger_handover_round(gen_random_uuid(), 3, interval '10 minutes', 1, interval '2 hours')",
    ] {
        let refused = sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&runtime)
            .await
            .expect_err(statement);
        assert_eq!(crate::support::db_error(refused).0, "42501", "{statement}");
    }
    let done = job(&fleet, &request(run), &Clockwork::new(&fleet, run)).await;
    assert!(
        matches!(done, Ok(AutoOutcome::HandedOver { .. })),
        "the early handover the forgeries allow: {done:?}"
    );
}

// --- the audit ------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_runtime_role_cannot_plant_the_audit_and_a_planted_id_that_differs_is_refused_audit_conflict()
 {
    let t = triggers().await;
    t.publish(cron_workflow("audited-a", "0 * * * *", "UTC"))
        .await;
    t.w.publish_as_other_tenant(cron_workflow("audited-b", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.ready(1).await;
    let runtime = t.w.db.runtime_pool().await;
    // the runtime role cannot read the tenant set, the census, nor the retirement
    for table in [
        "schedule_trigger_handover_tenants",
        "schedule_trigger_handover_census",
        "schedule_trigger_handover_retirement",
    ] {
        let refused = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}")))
            .execute(&runtime)
            .await
            .expect_err(table);
        assert_eq!(crate::support::db_error(refused).0, "42501", "{table}");
    }
    // the ids it could derive: the tenant's own, the run's, an epoch's; and a guessed one
    let tenant = t.w.admin.tenant_id().as_uuid();
    let run = Uuid::new_v4();
    for id in [
        tenant,
        run,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        Uuid::new_v4(),
    ] {
        sqlx::query(
            "INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, reason, created_at) \
             VALUES ($1, $2, 'forger', 'schedule.trigger_handover', 'platform', 'schedule_trigger_handover', 'forged', clock_timestamp())",
        )
        .bind(id)
        .bind(tenant)
        .execute(&runtime)
        .await
        .expect("sagat_app inserts audit records (G2252)");
    }
    let done = job(&fleet, &request(run), &Clockwork::new(&fleet, run))
        .await
        .expect("handed over");
    let AutoOutcome::HandedOver {
        tenants, audited, ..
    } = done
    else {
        panic!("{done:?}")
    };
    assert_eq!(
        (tenants, audited),
        (2, 2),
        "its own records, under ids nobody could guess"
    );
    let (by, at): (String, time::OffsetDateTime) =
        sqlx::query_as("SELECT changed_by, changed_at FROM schedule_trigger_handover")
            .fetch_one(&fleet.owner)
            .await
            .expect("row");
    let written: Vec<(String, time::OffsetDateTime, String)> = sqlx::query_as(
        "SELECT a.actor, a.created_at, a.reason FROM audit_records a JOIN schedule_trigger_handover_tenants s ON s.audit_id = a.id",
    )
    .fetch_all(&fleet.owner)
    .await
    .expect("records");
    assert_eq!(written.len(), 2);
    for (actor, created, reason) in written {
        assert_eq!(
            (actor.as_str(), created),
            (by.as_str(), at),
            "the actor and the time of the handover row"
        );
        assert!(reason.contains("mode AUTOMATIC"), "{reason}");
    }

    // a planted record under a drawn id (only the owner can read one) that differs: AUDIT_CONFLICT
    let t2 = triggers().await;
    t2.publish(cron_workflow("conflict", "0 * * * *", "UTC"))
        .await;
    t2.reconcile().await;
    let fleet = Fleet::on(&t2.w.db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    assert_eq!(fleet.wrapper(run).await.0, "HANDED_OVER");
    let id: Uuid = sqlx::query_scalar("SELECT audit_id FROM schedule_trigger_handover_tenants")
        .fetch_one(&fleet.owner)
        .await
        .expect("drawn id");
    sqlx::query(
        "INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, reason, created_at) \
         VALUES ($1, $2, 'forger', 'schedule.trigger_handover', 'platform', 'schedule_trigger_handover', 'forged', clock_timestamp())",
    )
    .bind(id)
    .bind(t2.w.admin.tenant_id().as_uuid())
    .execute(&fleet.owner)
    .await
    .expect("planted");
    let result = job(
        &fleet,
        &request(Uuid::new_v4()),
        &Clockwork::new(&fleet, run),
    )
    .await;
    assert!(
        matches!(
            result,
            Ok(AutoOutcome::AlreadyHandedOver {
                audit_pending: true,
                ..
            })
        ),
        "the handover is made: exit 0, with the audit left pending and said (correctness review): {result:?}"
    );
    // what is left is visible to the engines, which read it as the runtime role
    let store = sagat_infra_postgres::PgScheduleStore::new(t2.w.db.runtime_pool().await);
    let view = sagat_application::schedule_triggers::TriggerScheduleStore::handover_view(&store)
        .await
        .expect("the engines' view");
    assert_eq!(view.audit_pending, Some(1));
    // and the operator's --apply says the same, exit 0
    let owner_db = DatabaseConfig {
        url: t2.w.db.url.clone(),
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    };
    let said = schedule_trigger_handover(
        &owner_db,
        &HandoverRequest {
            operator: Some("ana".to_owned()),
            expect_live: Some(1),
            dry_run: false,
        },
    )
    .await;
    assert!(
        matches!(said, Ok(HandoverOutcome::AlreadyHandedOver { .. })),
        "{said:?}"
    );

    assert_eq!(
        fleet
            .count(
                "SELECT count(*) FROM schedule_trigger_handover_tenants WHERE audited_at IS NULL"
            )
            .await,
        1,
        "nothing of the call is kept"
    );
}

#[tokio::test]
async fn a_crash_after_the_commit_is_completed_once_by_the_next_run_for_the_recorded_set_only() {
    let t = triggers().await;
    t.publish(cron_workflow("crash-a", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;

    // a crash before the commit (the connection dies with the activation open): nothing at all
    {
        let mut conn = <sqlx::PgConnection as sqlx::Connection>::connect_with(
            &<sqlx::postgres::PgConnectOptions as std::str::FromStr>::from_str(&fleet.url)
                .expect("url")
                .password(HANDOVER_ROLE_PASSWORD),
        )
        .await
        .expect("connect");
        let mut tx: sqlx::Transaction<'_, sqlx::Postgres> =
            sqlx::Connection::begin(&mut conn).await.expect("begin");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("pid");
        let (outcome,): (String,) =
            sqlx::query_as("SELECT out_outcome FROM schedule_trigger_handover_auto($1)")
                .bind(run)
                .fetch_one(&mut *tx)
                .await
                .expect("wrapper");
        assert_eq!(outcome, "HANDED_OVER");
        sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .execute(&fleet.owner)
            .await
            .expect("crash");
        drop(tx);
    }
    assert_eq!(
        fleet.handover_row().await.0,
        "LEGACY",
        "the activation rolled back whole"
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM schedule_trigger_handover_tenants")
            .await,
        0
    );

    // a crash after the commit and before the audit: committed, no record yet
    fleet.age_run(run).await;
    fleet.round(run).await;
    assert_eq!(fleet.wrapper(run).await.0, "HANDED_OVER");
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM audit_records WHERE action = 'schedule.trigger_handover'")
            .await,
        0
    );
    // meanwhile the other tenant gains its first cron trigger and this one loses its last: the set is what was recorded
    t.w.publish_as_other_tenant(cron_workflow("crash-b", "0 * * * *", "UTC"))
        .await;
    fleet.sql("DELETE FROM workflow_triggers WHERE kind = 'cron' AND tenant_id = (SELECT tenant_id FROM schedule_trigger_handover_tenants)").await;
    let retry = job(&fleet, &request(run), &Clockwork::new(&fleet, run))
        .await
        .expect("exit 0");
    assert!(
        matches!(retry, AutoOutcome::AlreadyHandedOver { audited: 1, .. }),
        "{retry:?}"
    );
    let again = job(
        &fleet,
        &request(Uuid::new_v4()),
        &Clockwork::new(&fleet, run),
    )
    .await
    .expect("exit 0");
    assert!(
        matches!(again, AutoOutcome::AlreadyHandedOver { audited: 0, .. }),
        "once: {again:?}"
    );
    let recorded: Vec<Uuid> = sqlx::query_scalar(
        "SELECT tenant_id FROM audit_records WHERE action = 'schedule.trigger_handover'",
    )
    .fetch_all(&fleet.owner)
    .await
    .expect("records");
    assert_eq!(
        recorded,
        vec![t.w.admin.tenant_id().as_uuid()],
        "only the tenant the handover affected"
    );
}

#[tokio::test]
async fn the_operators_handover_records_mode_operator_and_its_audit_too() {
    let t = triggers().await;
    t.publish(cron_workflow("by-hand", "0 * * * *", "UTC"))
        .await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.engine().await;
    let db = DatabaseConfig {
        url: t.w.db.url.clone(),
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    };
    let done = schedule_trigger_handover(
        &db,
        &HandoverRequest {
            operator: Some("ana".to_owned()),
            expect_live: Some(1),
            dry_run: false,
        },
    )
    .await
    .expect("handed over");
    assert!(
        matches!(done, HandoverOutcome::HandedOver { .. }),
        "{done:?}"
    );
    let (_, by, _, mode, epoch) = fleet.handover_row().await;
    assert_eq!(
        (by.as_deref(), mode.as_deref()),
        (Some("ana"), Some("OPERATOR"))
    );
    assert!(epoch.is_some());
    assert_eq!(fleet.count("SELECT count(*) FROM audit_records WHERE action = 'schedule.trigger_handover' AND actor = 'ana'").await, 1);
}

// --- timeouts, refusals and the interrupt ---------------------------------------------------------------------------------

/// Kills the session that holds a lock, from the owner: the lock is released whatever the holder's task is doing.
fn release(pid: i32) -> String {
    format!("SELECT pg_terminate_backend({pid})")
}

#[tokio::test]
async fn statement_timeouts_of_the_rounds_are_retried_and_three_in_a_row_are_check_too_slow() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    // the conditions read the memberships: held, every round runs past its 5 s; the job's own notes do not read them
    let mut held = fleet.owner.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("pid");
    sqlx::query("LOCK TABLE scheduler_members IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .expect("hold");
    let mut pace = Clockwork::new(&fleet, run);
    pace.step = Some(Box::new(move |_, reason| {
        (reason == Some("CHECK_TOO_SLOW")).then(|| release(pid))
    }));
    let done = job(&fleet, &request(run), &pace).await;
    assert!(
        matches!(done, Ok(AutoOutcome::HandedOver { .. })),
        "{done:?}"
    );
    let reasons = pace.reasons();
    assert_eq!(
        reasons
            .iter()
            .take(3)
            .map(Option::as_deref)
            .collect::<Vec<_>>(),
        vec![Some("BUSY"), Some("BUSY"), Some("CHECK_TOO_SLOW")],
        "{reasons:?}"
    );
    assert_eq!(
        pace.asked()
            .iter()
            .take(3)
            .map(Duration::as_secs)
            .collect::<Vec<_>>(),
        vec![15, 30, 60],
        "the backoff starts at a heartbeat and doubles"
    );
    drop(held);
}

#[tokio::test]
async fn a_lock_wait_of_the_activation_is_busy_and_retried_with_nothing_recorded() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    // the epoch is held when the job activates: the wrapper waits its 2 s and gives up (55P03), recording nothing
    let mut held = fleet.owner.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("pid");
    sqlx::query("SELECT platform_requirement_lock()")
        .execute(&mut *held)
        .await
        .expect("hold the epoch");
    let owner = fleet.owner.clone();
    let mut pace = Clockwork::new(&fleet, run);
    pace.step = Some(Box::new(move |_, reason| {
        (reason == Some("BUSY")).then(|| release(pid))
    }));
    let done = job(&fleet, &request(run), &pace).await;
    assert!(
        matches!(done, Ok(AutoOutcome::HandedOver { .. })),
        "{done:?}"
    );
    assert!(
        pace.reasons().iter().any(|r| r.as_deref() == Some("BUSY")),
        "{:?}",
        pace.reasons()
    );
    let history: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_requirement_history")
        .fetch_one(&owner)
        .await
        .expect("history");
    assert_eq!(history, 1, "the refused attempt recorded nothing");
    drop(held);
}

#[tokio::test]
async fn an_interrupt_between_the_stable_round_and_the_activation_starts_no_activation_and_exits_four()
 {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    let interrupt = Interrupt::default();
    let mut pace = Clockwork::new(&fleet, run);
    pace.interrupt_at_activation = Some(interrupt.clone());
    let result = job_with(&fleet, &request(run), &pace, &interrupt).await;
    assert_eq!(exit_code(&result), 4, "{result:?}");
    assert_eq!(fleet.last_reason(run).await.as_deref(), Some("INTERRUPTED"));
    assert_eq!(fleet.handover_row().await.0, "LEGACY");
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirements")
            .await,
        0
    );
}

#[tokio::test]
async fn a_retried_pod_with_the_same_run_id_keeps_its_first_round_and_deadline_and_another_run_id_does_not()
 {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.round(run).await;
    fleet.sql("UPDATE schedule_trigger_handover_attempts SET first_round_at = first_round_at - interval '5 minutes', deadline = deadline - interval '5 minutes'").await;
    let read = |run: Uuid| {
        let pool = fleet.owner.clone();
        async move {
            sqlx::query_as::<_, (time::OffsetDateTime, time::OffsetDateTime)>(
                "SELECT first_round_at, deadline FROM schedule_trigger_handover_attempts WHERE run_id = $1",
            )
            .bind(run)
            .fetch_one(&pool)
            .await
            .expect("attempt")
        }
    };
    let before = read(run).await;
    fleet.round(run).await;
    assert_eq!(read(run).await, before, "the same run resumes");
    let other = Uuid::new_v4();
    fleet.round(other).await;
    assert!(read(other).await.0 > before.0, "a new run starts afresh");
}

#[tokio::test]
async fn the_dry_run_checks_the_wrappers_privilege_and_says_the_last_reason() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let run = Uuid::new_v4();
    fleet.round(run).await;
    let mut dry = request(run);
    dry.dry_run = true;
    let said = job(&fleet, &dry, &Clockwork::new(&fleet, run))
        .await
        .expect("may");
    assert!(
        matches!(&said, AutoOutcome::WouldWait { last_reason: Some(reason) } if reason == "TOO_FEW_ENGINES"),
        "{said:?}"
    );
    // the owner may not make the automatic handover
    let owner = DatabaseConfig {
        url: db.url.clone(),
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    };
    let refused = auto_handover(
        &owner,
        None,
        &dry,
        &Clockwork::new(&fleet, run),
        &Interrupt::default(),
    )
    .await;
    assert_eq!(exit_code(&refused), 2, "{refused:?}");
    // the runtime role neither
    let runtime = DatabaseConfig {
        url: db.url_for(RUNTIME_ROLE, crate::support::RUNTIME_ROLE_PASSWORD),
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    };
    let refused = auto_handover(
        &runtime,
        None,
        &dry,
        &Clockwork::new(&fleet, run),
        &Interrupt::default(),
    )
    .await;
    assert_eq!(exit_code(&refused), 2, "{refused:?}");
}

// --- the reasons, one set -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_reason_codes_of_the_sql_the_rust_enum_and_the_runbook_are_one_set() {
    let db = migrated_db().await;
    let sql: Vec<String> = sqlx::query_scalar("SELECT unnest(schedule_trigger_handover_reasons())")
        .fetch_all(&db.pool)
        .await
        .expect("the reasons of the database");
    let rust: Vec<String> = HandoverBlock::ALL
        .iter()
        .map(|r| r.code().to_owned())
        .collect();
    assert_eq!(
        sql, rust,
        "the SQL and the Rust enum, in the order of ADR-0091 decision 5"
    );
    // the attempt row's CHECK takes exactly these: it reads the same function
    let check: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'schedule_trigger_handover_attempts_reason_closed'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the CHECK");
    assert!(
        check.contains("schedule_trigger_handover_reasons()"),
        "{check}"
    );
    // and the note's closed set is part of it
    let note: String = sqlx::query_scalar(
        "SELECT prosrc FROM pg_proc WHERE oid = 'schedule_trigger_handover_note(uuid,text,integer,interval,integer,interval)'::regprocedure",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the note");
    for reason in ["BUSY", "CHECK_TOO_SLOW", "INTERRUPTED", "DEADLINE"] {
        assert!(
            note.contains(&format!("'{reason}'")) && sql.contains(&reason.to_owned()),
            "{reason}"
        );
    }
    for reason in &sql {
        assert!(HandoverBlock::from_code(reason).is_some());
    }
    let runbook = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/runbooks/schedules.md"),
    )
    .expect("the runbook");
    let section = runbook
        .split("### 6.3")
        .nth(1)
        .and_then(|rest| rest.split("\n### ").next())
        .expect("section 6.3 of the runbook");
    let anchors: Vec<String> = section
        .match_indices("<a id=\"handover-reason-")
        .map(|(at, _)| {
            let rest = &section[at + "<a id=\"".len()..];
            rest[..rest.find('"').expect("closed")].to_owned()
        })
        .collect();
    let expected: Vec<String> = HandoverBlock::ALL.iter().map(|r| r.anchor()).collect();
    assert_eq!(
        anchors, expected,
        "one anchored row of runbook 6.3 per reason, in order"
    );
    for reason in &sql {
        assert!(
            section.contains(&format!("`{reason}`")),
            "6.3 names {reason}"
        );
    }
}

// --- the catalog: every definer pinned and closed to PUBLIC; what each role may run ---------------------------------------

/// The three definers older than ADR-0091 that pin `pg_catalog, public, pg_temp` (G2280), by name: the test fails when one is fixed.
const G2280: [&str; 3] = [
    "business_ids_purge(integer)",
    "run_active_counts_track()",
    "schedule_purge_actions(integer,integer)",
];

#[tokio::test]
async fn every_security_definer_function_pins_its_search_path_and_none_is_executable_by_public() {
    let db = migrated_db().await;
    // every definer: a pinned path that begins with pg_catalog, ends with pg_temp, and holds between them only schemas that PUBLIC
    // cannot create in; and no definer executable by PUBLIC (ADR-0091 decision 9, D2267)
    let bad: Vec<(String, Option<Vec<String>>, bool)> = sqlx::query_as(
        "SELECT p.oid::regprocedure::text, p.proconfig::text[], has_function_privilege('public', p.oid, 'EXECUTE') \
           FROM pg_proc p WHERE p.pronamespace = 'public'::regnamespace AND p.prosecdef ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .expect("the catalog");
    let public_creates: bool =
        sqlx::query_scalar("SELECT has_schema_privilege('public', 'public', 'CREATE')")
            .fetch_one(&db.pool)
            .await
            .expect("schema privilege");
    let mut wrong = Vec::new();
    for (function, config, executable_by_public) in &bad {
        let path = config
            .iter()
            .flatten()
            .find_map(|setting| setting.strip_prefix("search_path="))
            .map(|path| path.split(',').map(str::trim).collect::<Vec<_>>());
        let pinned = match path.as_deref() {
            Some(["pg_catalog", "pg_temp"]) => true,
            // three definers older than this ADR pin `pg_catalog, public, pg_temp` (G2280), by name only: public after pg_catalog,
            // and nobody but the owner creates in it
            Some(["pg_catalog", "public", "pg_temp"]) => {
                !public_creates && G2280.contains(&function.as_str())
            }
            _ => false,
        };
        let adr = function.starts_with("schedule_trigger_handover_");
        if !pinned
            || *executable_by_public
            || (adr && path.as_deref() != Some(&["pg_catalog", "pg_temp"][..]))
        {
            wrong.push(format!(
                "{function} {config:?} public={executable_by_public}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "definer functions without the pinned path or open to PUBLIC: {wrong:?}"
    );
    // the tolerance of G2280 is closed: a function fixed leaves the list
    for function in G2280 {
        let (_, config, _) = bad
            .iter()
            .find(|(name, ..)| name == function)
            .unwrap_or_else(|| panic!("{function} is no longer a definer: take it out of G2280"));
        assert!(
            config
                .iter()
                .flatten()
                .any(|setting| setting == "search_path=pg_catalog, public, pg_temp"),
            "{function} pins another path now: take it out of G2280"
        );
    }
    // nobody but the owner may create in the schema the definers name (security review m7)
    let creators: Vec<String> = sqlx::query_scalar(
        "SELECT r.rolname::text FROM pg_roles r \
          WHERE r.rolname NOT LIKE 'pg\\_%' AND NOT r.rolsuper \
            AND r.oid <> (SELECT n.nspowner FROM pg_namespace n WHERE n.nspname = 'public') \
            AND has_schema_privilege(r.oid, 'public', 'CREATE') ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .expect("schema privileges");
    assert!(
        creators.is_empty(),
        "roles that may create in public: {creators:?}"
    );
    assert!(!public_creates, "nor PUBLIC");
    // and every function of this ADR, definer or not, pins it and is closed to PUBLIC
    let open: Vec<String> = sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text FROM pg_proc p \
          WHERE p.pronamespace = 'public'::regnamespace AND p.proname LIKE 'schedule_trigger_handover_%' \
            AND (has_function_privilege('public', p.oid, 'EXECUTE') \
                 OR NOT coalesce(p.proconfig @> ARRAY['search_path=pg_catalog, pg_temp'], false)) \
            AND p.proname <> 'schedule_trigger_handover_guard'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("the catalog");
    assert!(open.is_empty(), "{open:?}");
}

#[tokio::test]
async fn sagat_handover_can_run_its_five_functions_read_its_two_tables_and_one_column_and_nothing_else()
 {
    let db = migrated_db().await;
    let functions: Vec<String> = sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text FROM pg_proc p \
          WHERE p.pronamespace = 'public'::regnamespace \
            AND has_function_privilege($1, p.oid, 'EXECUTE') AND NOT has_function_privilege('public', p.oid, 'EXECUTE') \
          ORDER BY 1",
    )
    .bind(HANDOVER_ROLE)
    .fetch_all(&db.pool)
    .await
    .expect("functions");
    assert_eq!(
        functions,
        vec![
            "schedule_trigger_handover_audit()".to_owned(),
            "schedule_trigger_handover_auto(uuid)".to_owned(),
            "schedule_trigger_handover_note(uuid,text,integer,interval,integer,interval)"
                .to_owned(),
            "schedule_trigger_handover_retire()".to_owned(),
            "schedule_trigger_handover_round(uuid,integer,interval,integer,interval)".to_owned(),
        ],
        "the five functions of ADR-0091 decision 9 (OD-2026-11-15-1)"
    );
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname::text, p.privilege FROM pg_class c \
           CROSS JOIN unnest(ARRAY['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER']) AS p(privilege) \
          WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'v', 'm', 'p') \
            AND has_table_privilege($1, c.oid, p.privilege) ORDER BY 1, 2",
    )
    .bind(HANDOVER_ROLE)
    .fetch_all(&db.pool)
    .await
    .expect("tables");
    assert_eq!(
        tables,
        vec![
            ("schedule_trigger_handover".to_owned(), "SELECT".to_owned()),
            (
                "schedule_trigger_handover_attempts".to_owned(),
                "SELECT".to_owned()
            ),
        ]
    );
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname::text, a.attname::text FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid \
          WHERE c.relnamespace = 'public'::regnamespace AND a.attnum > 0 AND NOT a.attisdropped \
            AND c.relname NOT IN ('schedule_trigger_handover', 'schedule_trigger_handover_attempts') \
            AND has_column_privilege($1, c.oid, a.attnum, 'SELECT,INSERT,UPDATE,REFERENCES') ORDER BY 1, 2",
    )
    .bind(HANDOVER_ROLE)
    .fetch_all(&db.pool)
    .await
    .expect("columns");
    assert_eq!(
        columns,
        vec![(
            "platform_requirement_epoch".to_owned(),
            "changed_xact".to_owned()
        )],
        "the one column ADR-0082's deferred check reads at the commit of the job's activation (OD-2026-11-15-1)"
    );
    let definer: bool = sqlx::query_scalar(
        "SELECT prosecdef FROM pg_proc WHERE oid = 'platform_requirements_epoch_moved()'::regprocedure",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the check");
    assert!(
        !definer,
        "ADR-0082's deferred check stays as it was (D2282 reverted)"
    );
    let sequences: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c WHERE c.relkind = 'S' AND c.relnamespace = 'public'::regnamespace \
            AND (has_sequence_privilege($1, c.oid, 'USAGE') OR has_sequence_privilege($1, c.oid, 'SELECT') \
                 OR has_sequence_privilege($1, c.oid, 'UPDATE'))",
    )
    .bind(HANDOVER_ROLE)
    .fetch_one(&db.pool)
    .await
    .expect("sequences");
    assert_eq!(sequences, 0, "no sequence");
    let runtime: Vec<String> = sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text FROM pg_proc p \
          WHERE p.pronamespace = 'public'::regnamespace AND p.proname LIKE 'schedule_trigger_handover_%' \
            AND has_function_privilege($1, p.oid, 'EXECUTE') AND NOT has_function_privilege('public', p.oid, 'EXECUTE')",
    )
    .bind(RUNTIME_ROLE)
    .fetch_all(&db.pool)
    .await
    .expect("runtime");
    assert!(
        runtime.is_empty(),
        "the runtime role gained nothing executable: {runtime:?}"
    );
}

/// The stop note (OD-2026-11-15-1): a function of its own, a closed set, the run id, counting nothing.
#[tokio::test]
async fn the_note_function_writes_only_what_the_job_alone_sees_and_counts_nothing() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.round(run).await;
    fleet.age_run(run).await;
    let (_, _, before) = fleet.round(run).await;
    for reason in ["BUSY", "CHECK_TOO_SLOW", "INTERRUPTED", "DEADLINE"] {
        let (outcome,): (String,) = sqlx::query_as(
            "SELECT out_outcome FROM schedule_trigger_handover_note($1, $2, 3, interval '10 minutes', 1, interval '2 hours')",
        )
        .bind(run)
        .bind(reason)
        .fetch_one(&fleet.handover)
        .await
        .expect("note");
        assert_eq!(outcome, "NOTED");
        assert_eq!(fleet.last_reason(run).await.as_deref(), Some(reason));
    }
    let stable: i32 = sqlx::query_scalar(
        "SELECT stable_count FROM schedule_trigger_handover_attempts WHERE run_id = $1",
    )
    .bind(run)
    .fetch_one(&fleet.owner)
    .await
    .expect("count");
    assert_eq!(stable, before, "a note counts nothing and ends nothing");
    for refused in ["FIRST_HORIZON", "ENGINE_UNREGISTERED", "anything"] {
        let error = sqlx::query(
            "SELECT * FROM schedule_trigger_handover_note($1, $2, 3, interval '10 minutes', 1, interval '2 hours')",
        )
        .bind(run)
        .bind(refused)
        .execute(&fleet.handover)
        .await
        .expect_err(refused);
        assert_eq!(
            crate::support::db_error(error).0,
            "22023",
            "{refused}: what the database decides is not noted"
        );
    }
    let fresh = Uuid::new_v4();
    sqlx::query("SELECT * FROM schedule_trigger_handover_note($1, 'BUSY', 3, interval '10 minutes', 1, interval '2 hours')")
        .bind(fresh)
        .execute(&fleet.handover)
        .await
        .expect("a run whose first round failed gets its row from a note");
    assert_eq!(fleet.last_reason(fresh).await.as_deref(), Some("BUSY"));
}

/// Security review m3: the owner's floor of the live engines holds whatever a caller of the round asks.
#[tokio::test]
async fn the_owners_floor_of_live_engines_holds_against_a_weaker_min_live() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    fleet
        .sql("SELECT schedule_trigger_handover_set_floor(2)")
        .await;
    let run = Uuid::new_v4();
    assert_eq!(
        fleet.round(run).await.1.as_deref(),
        Some("TOO_FEW_ENGINES"),
        "a --min-live of 1 is raised to the floor"
    );
    fleet.engine().await;
    assert_ne!(fleet.round(run).await.1.as_deref(), Some("TOO_FEW_ENGINES"));
}

/// Correctness review: a streak with a gap of more than two heartbeats starts over.
#[tokio::test]
async fn a_streak_with_a_gap_of_more_than_two_heartbeats_starts_over() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.round(run).await;
    fleet.age_run(run).await;
    assert_eq!(fleet.round(run).await.2, 2);
    sqlx::query(
        "UPDATE schedule_trigger_handover_attempts SET last_stable_at = clock_timestamp() - interval '31 seconds' WHERE run_id = $1",
    )
    .bind(run)
    .execute(&fleet.owner)
    .await
    .expect("a gap");
    assert_eq!(fleet.round(run).await.2, 1, "not consecutive any more");
}

/// Security review m4: the runs are bounded, so the rows the engines read cannot grow without end.
#[tokio::test]
async fn a_seventeenth_live_run_is_refused_and_a_stopped_run_gives_way() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    for _ in 0..16 {
        fleet.round(Uuid::new_v4()).await;
    }
    let refused = sqlx::query(
        "SELECT * FROM schedule_trigger_handover_round($1, 3, interval '10 minutes', 1, interval '2 hours')",
    )
    .bind(Uuid::new_v4())
    .execute(&fleet.handover)
    .await
    .expect_err("a seventeenth");
    assert_eq!(crate::support::db_error(refused).0, "54000");
    fleet
        .sql(
            "UPDATE schedule_trigger_handover_attempts SET last_reason = 'INTERRUPTED' \
              WHERE run_id = (SELECT run_id FROM schedule_trigger_handover_attempts LIMIT 1)",
        )
        .await;
    fleet.round(Uuid::new_v4()).await;
}

/// Security nit: a planted record that differs only by its trace id is a conflict.
#[tokio::test]
async fn a_planted_audit_record_that_differs_only_by_its_trace_id_is_a_conflict() {
    let t = triggers().await;
    t.publish(cron_workflow("traced", "0 * * * *", "UTC")).await;
    t.reconcile().await;
    let fleet = Fleet::on(&t.w.db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    fleet.stable(run).await;
    assert_eq!(fleet.wrapper(run).await.0, "HANDED_OVER");
    sqlx::query(
        "INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, reason, trace_id, created_at) \
         SELECT s.audit_id, s.tenant_id, h.changed_by, 'schedule.trigger_handover', 'platform', 'schedule_trigger_handover', \
                format('the cron triggers were handed over to the schedule pass (mode %s, requirement epoch %s)', \
                       h.changed_mode, h.changed_epoch), \
                repeat('a', 32), h.changed_at \
           FROM schedule_trigger_handover_tenants s CROSS JOIN schedule_trigger_handover h",
    )
    .execute(&fleet.owner)
    .await
    .expect("planted");
    let conflict = sqlx::query("SELECT * FROM schedule_trigger_handover_audit()")
        .execute(&fleet.handover)
        .await
        .expect_err("a conflict");
    assert_eq!(crate::support::db_error(conflict).0, "23505");
}

#[tokio::test]
async fn the_job_switched_off_exits_zero_at_once_and_without_a_password_exits_two_naming_the_variable()
 {
    use sagat_control_plane::cli::{Command, parse};
    use sagat_control_plane::handover_auto::run_auto_command;
    let Command::ScheduleTriggerHandover(args) =
        parse(&["schedule-trigger-handover".to_owned(), "--auto".to_owned()]).expect("parse")
    else {
        panic!("the command")
    };
    // no database is reached in either case: the URL points nowhere
    let nowhere = DatabaseConfig {
        url: "postgres://sagat_handover@127.0.0.1:1/none".to_owned(),
        max_connections: 1,
        statement_timeout: Duration::from_secs(1),
    };
    let off = |name: &str| (name == "SAGAT_SCHEDULES_AUTO_HANDOVER").then(|| "false".to_owned());
    let said = run_auto_command(&nowhere, &args, &off, &Interrupt::default())
        .await
        .expect("exit 0");
    assert!(said.contains("switched off"), "{said}");
    let none = |_: &str| None;
    let refused = run_auto_command(&nowhere, &args, &none, &Interrupt::default())
        .await
        .expect_err("exit 2");
    assert_eq!(refused.exit_code(), 2);
    assert!(
        refused.to_string().contains("SAGAT_HANDOVER_PASSWORD"),
        "{refused}"
    );
    let bad_run = |name: &str| match name {
        "SAGAT_HANDOVER_PASSWORD" => Some("a-password-long-enough".to_owned()),
        "SAGAT_HANDOVER_RUN_ID" => Some("job-name-not-a-uid".to_owned()),
        _ => None,
    };
    let refused = run_auto_command(&nowhere, &args, &bad_run, &Interrupt::default())
        .await
        .expect_err("exit 2");
    assert_eq!(refused.exit_code(), 2, "{refused}");
}

/// Correctness review Major 1: the job keeps its own deadline, so it ends with exit 3 even when no round ever answers.
#[tokio::test]
async fn the_job_ends_at_its_deadline_with_exit_three_even_when_every_round_fails() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    let mut held = fleet.owner.begin().await.expect("begin");
    sqlx::query("LOCK TABLE scheduler_members IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .expect("every round times out");
    let mut short = request(run);
    short.ask.wait = Duration::from_secs(1);
    let pace = Clockwork::new(&fleet, run);
    let result = job(&fleet, &short, &pace).await;
    assert_eq!(exit_code(&result), 3, "{result:?}");
    assert_eq!(fleet.last_reason(run).await.as_deref(), Some("DEADLINE"));
    assert!(
        pace.asked()
            .iter()
            .all(|wait| *wait <= Duration::from_secs(2)),
        "every wait is clamped to the deadline: {:?}",
        pace.asked()
    );
    held.rollback().await.expect("release");
}

/// Correctness nit: a --wait that cannot hold a horizon and the stable rounds is refused at the command line.
#[test]
fn a_wait_shorter_than_the_horizon_and_the_stable_rounds_is_refused() {
    let args = |wait: &str| {
        ["schedule-trigger-handover", "--auto", "--wait", wait]
            .iter()
            .map(|a| (*a).to_owned())
            .collect::<Vec<_>>()
    };
    assert!(
        sagat_control_plane::cli::parse(&args("10m")).is_err(),
        "10m < 10m + 3 x 15s"
    );
    assert!(sagat_control_plane::cli::parse(&args("11m")).is_ok());
}

/// Correctness review Major 2: statement timeouts of the activation count on their own, so three of them are CHECK_TOO_SLOW even
/// though the rounds between them answer.
#[tokio::test]
async fn three_statement_timeouts_of_the_activation_are_check_too_slow() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    // the activation writes the tenant set: held, its statement runs past the second it is given
    let mut held = fleet.owner.begin().await.expect("begin");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *held)
        .await
        .expect("pid");
    sqlx::query("LOCK TABLE schedule_trigger_handover_tenants IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .expect("hold");
    let mut slow = request(run);
    slow.activation = sagat_infra_postgres::OperatorBounds {
        lock_timeout: Duration::from_secs(60),
        statement_timeout: Duration::from_secs(1),
        step: Duration::from_secs(60),
    };
    let mut pace = Clockwork::new(&fleet, run);
    pace.step = Some(Box::new(move |_, reason| {
        (reason == Some("CHECK_TOO_SLOW")).then(|| release(pid))
    }));
    let done = job(&fleet, &slow, &pace).await;
    assert!(
        matches!(done, Ok(AutoOutcome::HandedOver { .. })),
        "{done:?}"
    );
    let notes: Vec<Option<String>> = pace
        .reasons()
        .into_iter()
        .filter(|r| matches!(r.as_deref(), Some("BUSY" | "CHECK_TOO_SLOW")))
        .collect();
    assert_eq!(
        notes.iter().map(Option::as_deref).collect::<Vec<_>>(),
        vec![Some("BUSY"), Some("BUSY"), Some("CHECK_TOO_SLOW")],
        "{:?}",
        pace.reasons()
    );
    drop(held);
}

/// An interrupt that comes while an activation runs does not cut it: it commits, and the job exits 0.
#[tokio::test]
async fn an_interrupt_during_an_activation_lets_it_commit_whole() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    let mut held = fleet.owner.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM schedule_trigger_handover FOR UPDATE")
        .execute(&mut *held)
        .await
        .expect("hold the row");
    let mut wide = request(run);
    wide.activation = HELD;
    let interrupt = Interrupt::default();
    let task = {
        let pace = Clockwork::new(&fleet, run);
        let database = fleet.database();
        let interrupt = interrupt.clone();
        tokio::spawn(async move {
            auto_handover(
                &database,
                Some(HANDOVER_ROLE_PASSWORD),
                &wide,
                &pace,
                &interrupt,
            )
            .await
        })
    };
    waiting_on_locks(&fleet.owner, 1).await;
    interrupt.raise();
    held.commit().await.expect("release");
    let done = tokio::time::timeout(Duration::from_secs(120), task)
        .await
        .expect("ends")
        .expect("joined");
    assert!(
        matches!(done, Ok(AutoOutcome::HandedOver { .. })),
        "the activation begun committed: {done:?}"
    );
    assert_eq!(fleet.handover_row().await.0, "HANDED_OVER");
}

/// The job holds the epoch in its activation while the operator's command waits for it; the operator then finds it handed over.
#[tokio::test]
async fn the_job_holding_the_epoch_makes_the_operator_wait_and_find_it_handed_over() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    fleet.ready(1).await;
    let run = Uuid::new_v4();
    let mut held = fleet.owner.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM schedule_trigger_handover FOR UPDATE")
        .execute(&mut *held)
        .await
        .expect("hold the row");
    let mut wide = request(run);
    wide.activation = HELD;
    let job_task = {
        let pace = Clockwork::new(&fleet, run);
        let database = fleet.database();
        tokio::spawn(async move {
            auto_handover(
                &database,
                Some(HANDOVER_ROLE_PASSWORD),
                &wide,
                &pace,
                &Interrupt::default(),
            )
            .await
        })
    };
    waiting_on_locks(&fleet.owner, 1).await;
    let operator = {
        let url = db.url.clone();
        tokio::spawn(async move {
            schedule_trigger_handover(
                &DatabaseConfig {
                    url,
                    max_connections: 2,
                    statement_timeout: Duration::from_secs(60),
                },
                &HandoverRequest {
                    operator: Some("ana".to_owned()),
                    expect_live: Some(1),
                    dry_run: false,
                },
            )
            .await
        })
    };
    // the job waits on the row with the epoch held; the operator waits on the epoch
    waiting_on_locks(&fleet.owner, 2).await;
    held.commit().await.expect("release");
    let done = tokio::time::timeout(Duration::from_secs(120), job_task)
        .await
        .expect("ends")
        .expect("joined");
    assert!(
        matches!(done, Ok(AutoOutcome::HandedOver { .. })),
        "{done:?}"
    );
    let said = tokio::time::timeout(Duration::from_secs(120), operator)
        .await
        .expect("ends")
        .expect("joined");
    assert!(
        matches!(said, Ok(HandoverOutcome::AlreadyHandedOver { .. })),
        "{said:?}"
    );
    assert_eq!(
        fleet
            .count("SELECT count(*) FROM platform_requirement_history")
            .await,
        1
    );
    assert_eq!(fleet.handover_row().await.3.as_deref(), Some("AUTOMATIC"));
}

/// Security review m4: the engines see the reasons of every live run, not only the latest.
#[tokio::test]
async fn the_engines_see_the_reason_of_every_live_run() {
    let db = migrated_db().await;
    let fleet = Fleet::on(&db).await;
    let older = Uuid::new_v4();
    fleet.round(older).await;
    fleet.membership("engine-0.2", 15).await;
    fleet.ready(1).await;
    let newer = Uuid::new_v4();
    fleet.round(newer).await;
    let store = sagat_infra_postgres::PgScheduleStore::new(db.runtime_pool().await);
    let view = sagat_application::schedule_triggers::TriggerScheduleStore::handover_view(&store)
        .await
        .expect("view");
    assert_eq!(
        view.blocked,
        vec![
            HandoverBlock::EngineUnregistered,
            HandoverBlock::TooFewEngines
        ],
        "both live runs, read as the runtime role"
    );
    assert!(view.attempt_age_seconds.is_some());
    assert_eq!(
        view.audit_pending, None,
        "nothing to audit before the handover"
    );
}
