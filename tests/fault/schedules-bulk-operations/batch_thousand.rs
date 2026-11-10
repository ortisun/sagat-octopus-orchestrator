//! One real job of 1,000 runs through a kill and a cancellation (`T018-046`, `SC-018-009`, `FR-018-029..031`,
//! `FR-018-033`, `NFR-018-001`): the job cancels 1,000 real runs on two replicas of the engine (a keeper and a batch pass
//! each, the real run service for the effect) while their processes are killed between the effect on a run and the
//! record of the item (the connection of the transaction that records it is cut inside PostgreSQL), and once outright,
//! at a moment nothing chose; the shards of the dead process pass to the other, a new process takes its place, and the
//! job is cancelled by the API half way. The matrix of `batch.rs` cuts each write of each transaction of a job of three
//! items; this is the same claim at the size the success criterion names, with the whole engine in the way.
//!
//! What has to be true at the end, and is read from the database and not from what the processes say: each run was
//! cancelled at most once (the journal of each run holds one request to stop it, not two), the counters of the job
//! equal its items and its items equal what it selected, no item is left stuck, the unrun items are marked cancelled and
//! the runs of the items that ran stay as the job left them (it undoes nothing), a run whose creating transaction was
//! still open when the job was created is selected (the selection waits for the horizon of the creation, and the test
//! holds one open for as long as it takes to show it), and a monitor that looks at the row locks of `runs` all the while
//! never sees a transaction that holds two.
//!
//! There is no hint to lose: a job is found by the poll of the pass and no broker message announces it, so a broker that
//! loses a message can neither delay nor repeat a step of a job (`batch.rs`, and the benchmark that runs without a broker).
//!
//! Every wait here has a bound in time of minutes: the selection of a job waits for the oldest open transaction of the
//! whole server (the tests of a suite share one), and a runner that is slow is not a failure.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use sagat_application::batch::{BatchService, BatchWrite, DueJob, JobRecord};
use sagat_application::batch_executor::{
    BatchExecutor, ExecutorSettings, ItemActions, ItemEffect, ItemRequest, Stepped,
};
use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::identity::RequestContext;
use sagat_domain::batch::{Action, Filter, JobState, Justification, Rate, Selector};
use sagat_domain::execution::RunId;
use sagat_domain::sharding::Shard;
use sagat_infra_postgres::{DatabaseClock, PgBatchStore, PgIdentityStore, PgRunStore};
use sagat_scheduler::{BatchPass, BatchSettings, Keeper, KeeperSettings, Placement};
use serde_json::json;
use sqlx::{Connection as _, PgConnection};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::support::bounded::within;
use crate::support::engine::World;
use crate::support::lock_order::{RowLockMonitor, database_deadlocks, describe};
use crate::support::schedules::{Fixture, fixture};
use crate::tripwire::Tripwire;

type Clock = DatabaseClock<SystemClock>;

/// The run service of the world, as the executor's single-run use cases.
#[derive(Clone)]
struct Runs(Arc<World>);

impl ItemActions for Runs {
    async fn apply(&self, request: &ItemRequest<'_>) -> Result<ItemEffect, AppError> {
        ItemActions::apply(&self.0.runs, request).await
    }
}

type Pass = BatchPass<PgBatchStore, Runs, PgIdentityStore, Clock>;

/// The most one wait of this test is allowed to take. It is a time and never a number of rounds: a round of a loop that
/// waits for a transaction of another test of the server takes as long as that transaction does.
const WAIT: StdDuration = StdDuration::from_secs(600);

/// How many runs the job selects.
const RUNS: usize = 1_000;

/// A process of the engine: the keeper that holds its shards and the pass that works on them, which run in a loop of
/// their own once it is started and until it is killed.
struct Engine {
    name: String,
    keeper: Arc<Keeper<PgRunStore>>,
    pass: Arc<Pass>,
    task: Option<JoinHandle<()>>,
}

impl Engine {
    fn new(f: &Fixture, clock: &Clock, name: &str) -> Self {
        let name = format!("{name}-{}", Uuid::now_v7());
        let placement = Placement::new(name.clone());
        // as the engine runs: the stores over the pool of the least-privilege role when the suite asks for it
        let pool = f.w.execution_pool.clone();
        let keeper = Keeper::new(
            f.w.store.clone(),
            placement.clone(),
            KeeperSettings {
                ttl: StdDuration::from_secs(3_600),
            },
        );
        let executor = BatchExecutor::new(
            PgBatchStore::new(pool.clone()),
            Runs(Arc::clone(&f.w)),
            PgIdentityStore::new(pool),
            clock.clone(),
            ExecutorSettings::default(),
        );
        Self {
            name,
            keeper: Arc::new(keeper),
            pass: Arc::new(BatchPass::new(
                executor,
                placement,
                BatchSettings::default(),
            )),
            task: None,
        }
    }

    /// The process begins: it keeps its shards and takes steps, over and over.
    fn start(&mut self) {
        let (keeper, pass) = (Arc::clone(&self.keeper), Arc::clone(&self.pass));
        self.task = Some(tokio::spawn(async move {
            loop {
                // a failed round of the keeper is what a process that lost its database sees: it tries again
                let _ = keeper.round().await;
                let round = pass.round().await;
                if round.steps == 0 {
                    let rest = round
                        .next_due_in
                        .unwrap_or(StdDuration::from_millis(20))
                        .clamp(StdDuration::from_millis(2), StdDuration::from_millis(50));
                    tokio::time::sleep(rest).await;
                }
            }
        }));
    }

    /// The process dies where it is: whatever it was doing is dropped, nothing of it is finished or tidied, and what it
    /// held is nobody's once the leases it kept have lapsed (here they are made to lapse, as time would).
    async fn kill(&mut self, f: &Fixture) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        sqlx::query("UPDATE scheduler_shards SET expires_at = 'epoch' WHERE owner = $1")
            .bind(&self.name)
            .execute(&f.w.db.pool)
            .await
            .expect("the leases lapse");
        sqlx::query("DELETE FROM scheduler_members WHERE owner = $1")
            .bind(&self.name)
            .execute(&f.w.db.pool)
            .await
            .expect("the process is no member");
    }
}

fn due(job: &JobRecord) -> DueJob {
    DueJob {
        tenant_id: job.tenant_id,
        id: job.id,
    }
}

/// Waits until `condition` holds, looking every so often, and says what it waited for if the bound passes.
async fn until<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    within(what, WAIT, async {
        while !condition().await {
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await;
}

fn service(f: &Fixture, clock: &Clock) -> BatchService<PgBatchStore, Clock> {
    BatchService::new(PgBatchStore::new(f.w.execution_pool.clone()), clock.clone())
}

async fn job(f: &Fixture, clock: &Clock, id: Uuid) -> JobRecord {
    service(f, clock).get(&f.w.admin, id).await.expect("job")
}

/// `(state, how many)` of the items of the job.
async fn item_states(f: &Fixture, id: Uuid) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT state, count(*) FROM batch_items WHERE job_id = $1 GROUP BY state ORDER BY state",
    )
    .bind(id)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("items")
}

/// Items whose run the job has cancelled and whose outcome is not recorded: the effect without its record, which is
/// what a process leaves behind when it dies between the two.
async fn effects_without_a_record(f: &Fixture, id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM batch_items i JOIN runs r ON r.id = i.run_id \
         WHERE i.job_id = $1 AND i.state = 'PENDING' AND r.status = 'CANCELLED'",
    )
    .bind(id)
    .fetch_one(&f.w.db.pool)
    .await
    .expect("count")
}

/// The replica that holds the shard of the job, as the database says.
async fn owner_of(f: &Fixture, id: Uuid) -> String {
    sqlx::query_scalar("SELECT owner FROM scheduler_shards WHERE shard = $1")
        .bind(i16::from(Shard::of(id).index()))
        .fetch_one(&f.w.db.pool)
        .await
        .expect("the owner of the shard")
}

/// Starts `n` runs, a few at a time.
async fn start_runs(f: &Fixture, n: usize) -> Vec<RunId> {
    let mut started = Vec::with_capacity(n);
    for chunk in (0..n).collect::<Vec<_>>().chunks(8) {
        let calls = chunk
            .iter()
            .map(|_| async { f.w.start(f.workflow, &json!({})).await });
        started.extend(futures_util::future::join_all(calls).await);
    }
    started
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_of_a_thousand_runs_loses_its_process_three_times_between_an_effect_and_its_record_and_once_outright_and_is_cancelled()
 {
    let begun = Instant::now();
    let f = fixture().await;
    let clock = Clock::new(f.w.db.pool.clone(), SystemClock);
    clock.sync().await.expect("the database clock");

    // 999 runs, and the thousandth in a transaction that is still open when the job is created: it has written its run
    // and is held at its last write by a lock this test holds on the table of the telemetry marks
    let mut runs = start_runs(&f, RUNS - 1).await;
    // the switch that cuts a connection and the monitor of the row locks go in before the lock is taken: installing a
    // trigger on a table waits for every lock on it
    let wire = Tripwire::install(&f.w.db.pool).await;
    let deadlocks_before = database_deadlocks(&f.w.db.pool).await;
    let monitor = RowLockMonitor::start(&f.w.db.pool, "runs").await;
    // (a connection of its own: a test that fails while the lock is held drops it, the server sees it close and ends the
    // transaction, and the run held by it ends too, instead of waiting for a pool that is never closed)
    let mut hold = PgConnection::connect(&f.w.db.url)
        .await
        .expect("a connection for the lock");
    for statement in [
        "BEGIN",
        "LOCK TABLE telemetry_pending IN ACCESS EXCLUSIVE MODE",
    ] {
        sqlx::query(statement)
            .execute(&mut hold)
            .await
            .expect("the table is held");
    }
    let late = tokio::spawn({
        let (w, workflow) = (Arc::clone(&f.w), f.workflow);
        async move { w.start(workflow, &json!({})).await }
    });
    f.wait_for_a_backend_on_a_lock("the last run is held in its transaction, with its run written")
        .await;
    println!(
        "{} runs are started, the last in a transaction that is open ({:.1} s)",
        RUNS - 1,
        begun.elapsed().as_secs_f64()
    );

    // the job: every run of the namespace, cancelled, at the highest rate, under the default settings of the executor
    let created = service(&f, &clock)
        .create(
            &f.w.admin,
            f.w.namespace.id(),
            BatchWrite {
                selector: Selector::filter(Filter::default()).expect("selector"),
                action: Action::Cancel,
                step_id: None,
                instance_path: Vec::new(),
                signal: None,
                update: None,
                drop_business_id: false,
                repin: false,
                reason: Justification::new("cancel everything the namespace runs").expect("reason"),
                max_items: None,
                rate: Some(Rate::MAX),
            },
            "thousand-1",
            &RequestContext::default(),
        )
        .await
        .expect("create the job");

    // two replicas of the engine. Each announces itself, the first gives back what is beyond its share, the second takes it.
    let one = Engine::new(&f, &clock, "one");
    let two = Engine::new(&f, &clock, "two");
    for _ in 0..2 {
        one.keeper.round().await.expect("keeper");
        two.keeper.round().await.expect("keeper");
    }
    let owned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM scheduler_shards WHERE owner IN ($1, $2)")
            .bind(&one.name)
            .bind(&two.name)
            .fetch_one(&f.w.db.pool)
            .await
            .expect("owned shards");
    assert_eq!(owned, 64, "between them the two replicas hold every shard");

    // the selection waits for the transaction that was open when the job was created, and takes nothing meanwhile
    // (a round of a job that must wait returns at once: one that goes on instead meets the lock this test holds, and the
    // stage ends in a panic that names it)
    for engine in [&one, &two] {
        for _ in 0..5 {
            let stepped = within(
                "a round of a selection that waits takes no step",
                StdDuration::from_secs(120),
                engine.pass.round(),
            )
            .await;
            assert_eq!(
                (stepped.steps, stepped.failed),
                (0, 0),
                "a selection that waits takes no step: {stepped:?}"
            );
        }
    }
    let probe_executor = BatchExecutor::new(
        PgBatchStore::new(f.w.execution_pool.clone()),
        Runs(Arc::clone(&f.w)),
        PgIdentityStore::new(f.w.execution_pool.clone()),
        clock.clone(),
        ExecutorSettings::default(),
    );
    assert_eq!(
        probe_executor
            .step(&due(&created), None)
            .await
            .expect("a step"),
        Stepped::Idle,
        "a transaction of the time of creation could still commit a run"
    );
    let waiting = job(&f, &clock, created.id).await;
    assert_eq!(
        (waiting.state, waiting.counts.selected),
        (JobState::Selecting, 0),
        "nothing is selected while the transaction is open"
    );

    // it ends: the run is written, and the job has to take it
    sqlx::query("ROLLBACK")
        .execute(&mut hold)
        .await
        .expect("the lock is released");
    let held_run = within("the held run commits", WAIT, late)
        .await
        .expect("the start task");
    runs.push(held_run);
    let mut engines: Vec<Engine> = Vec::new();

    // which of them has the job; it takes the selection and then applies items. One round of it is watched: the writes of the
    // transaction that records an item are what is cut off below, at the first and at the last
    let owner = owner_of(&f, created.id).await;
    let leader = if owner == one.name { &one } else { &two };
    within("the job is selected", WAIT, async {
        loop {
            leader.pass.round().await;
            if job(&f, &clock, created.id).await.state == JobState::Running {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
    })
    .await;
    let selected = job(&f, &clock, created.id).await;
    assert_eq!(
        selected.counts.selected,
        u32::try_from(RUNS).expect("small"),
        "all the runs, the one that was held among them"
    );
    wire.watch().await;
    let round = leader.pass.round().await;
    wire.disarm().await;
    assert!(round.steps > 0, "the watched round took steps: {round:?}");
    let writes = wire.writes().await;
    // the transaction that records an item writes `batch_items` and the job and does not write the run
    let by_transaction: Vec<(i64, Vec<(usize, String)>)> = {
        let mut order: Vec<i64> = Vec::new();
        for (xid, _, _) in &writes {
            if !order.contains(xid) {
                order.push(*xid);
            }
        }
        order
            .into_iter()
            .map(|xid| {
                (
                    xid,
                    writes
                        .iter()
                        .filter(|(other, _, _)| *other == xid)
                        .map(|(_, position, table)| (*position, table.clone()))
                        .collect(),
                )
            })
            .collect()
    };
    let record = by_transaction
        .iter()
        .find(|(_, tables)| {
            tables.iter().any(|(_, table)| table == "batch_items")
                && !tables.iter().any(|(_, table)| table == "runs")
        })
        .expect("a transaction that records an item");
    let cuts: Vec<(usize, String)> = record
        .1
        .iter()
        .filter(|(_, table)| table == "batch_items" || table == "batch_jobs")
        .cloned()
        .collect();
    assert!(!cuts.is_empty());
    println!(
        "a record of an item is one transaction of {} writes ({:?}); the cuts are at {:?}",
        record.1.len(),
        record.1,
        cuts
    );

    // the leader goes on as a process of its own, and so does the other
    let (mut leader_engine, mut other_engine) = if owner == one.name {
        (one, two)
    } else {
        (two, one)
    };
    leader_engine.start();
    other_engine.start();
    engines.push(leader_engine);
    engines.push(other_engine);

    // --- the zombie, the cuts and the deaths ---------------------------------------------------------------------------
    let mut dead: Vec<Engine> = Vec::new();
    let mut deaths = 0;
    for cut in 0..3 {
        // enough has happened for a death to cost something
        let target = 100 + 150 * u32::try_from(cut).expect("small");
        until("the job gets to the point of the cut", || async {
            job(&f, &clock, created.id).await.counts.succeeded >= target
        })
        .await;
        let (position, table) = if cut == 1 {
            cuts.last().expect("a cut").clone()
        } else {
            cuts.first().expect("a cut").clone()
        };
        wire.arm_on(&table, position).await;
        until(
            "an item is cut off between its effect and its record",
            || async { effects_without_a_record(&f, created.id).await > 0 },
        )
        .await;
        // the process that was doing it dies here, with its connection cut
        let owner = owner_of(&f, created.id).await;
        let index = engines
            .iter()
            .position(|engine| engine.name == owner)
            .expect("the owner is one of the live processes");
        let mut victim = engines.remove(index);
        victim.kill(&f).await;
        wire.disarm().await;
        deaths += 1;
        // a new process takes the place of the dead one, from nothing but the database
        let mut born = Engine::new(&f, &clock, "born");
        born.start();
        engines.push(born);
        // what is left unrecorded is recorded by a process that is alive
        until(
            "a live process takes the shard over and records the item",
            || async {
                let holder = owner_of(&f, created.id).await;
                effects_without_a_record(&f, created.id).await == 0
                    && engines.iter().any(|engine| engine.name == holder)
            },
        )
        .await;
        if cut == 0 {
            // the first dead process, which still believes it holds its shards, is refused where it records
            let stale = victim.pass.round().await;
            assert_eq!(
                (stale.steps, stale.failed),
                (0, 0),
                "a process that was replaced takes no step: {stale:?}"
            );
        }
        dead.push(victim);
    }

    // one more outright, at a moment nothing chose
    until("the job gets to the last death", || async {
        job(&f, &clock, created.id).await.counts.succeeded >= 600
    })
    .await;
    let owner = owner_of(&f, created.id).await;
    let index = engines
        .iter()
        .position(|engine| engine.name == owner)
        .expect("the owner is one of the live processes");
    let mut victim = engines.remove(index);
    victim.kill(&f).await;
    deaths += 1;
    dead.push(victim);
    let mut born = Engine::new(&f, &clock, "born");
    born.start();
    engines.push(born);

    // --- the cancellation, half way --------------------------------------------------------------------------------------
    until("the job gets to the cancellation", || async {
        job(&f, &clock, created.id).await.counts.succeeded >= 700
    })
    .await;
    let cancelling = service(&f, &clock)
        .cancel(
            &f.w.admin,
            created.id,
            "stop this batch job now",
            &RequestContext::default(),
        )
        .await
        .expect("cancel the job");
    assert!(
        matches!(cancelling.state, JobState::Cancelling | JobState::Cancelled),
        "{:?}",
        cancelling.state
    );
    until("the cancelled job ends", || async {
        job(&f, &clock, created.id).await.state == JobState::Cancelled
    })
    .await;
    // the processes of the engine go on a moment longer and find nothing
    for engine in &mut engines {
        engine.kill(&f).await;
    }
    for engine in &dead {
        let stale = engine.pass.round().await;
        assert_eq!(
            stale.steps, 0,
            "a process that was killed does nothing when it is found again: {stale:?}"
        );
    }

    // --- what is true of the end -------------------------------------------------------------------------------------------
    let ended = job(&f, &clock, created.id).await;
    let counts = &ended.counts;
    let items: i64 = sqlx::query_scalar("SELECT count(*) FROM batch_items WHERE job_id = $1")
        .bind(created.id)
        .fetch_one(&f.w.db.pool)
        .await
        .expect("items");
    let states = item_states(&f, created.id).await;
    println!(
        "{deaths} deaths, one cancellation: counts {counts:?}, items {states:?}, {:.1} s",
        begun.elapsed().as_secs_f64()
    );
    assert_eq!(ended.state, JobState::Cancelled);
    assert_eq!(
        i64::from(counts.selected),
        items,
        "the job counts the items it has"
    );
    assert_eq!(
        items,
        i64::try_from(RUNS).expect("small"),
        "one item for each run"
    );
    assert_eq!(
        counts.succeeded + counts.skipped + counts.failed + counts.cancelled,
        counts.selected,
        "the counters add up to what was selected: {counts:?}"
    );
    assert_eq!(
        counts.deferred, 0,
        "nothing waits to be tried again: {counts:?}"
    );
    assert!(
        states
            .iter()
            .all(|(state, _)| matches!(state.as_str(), "SUCCEEDED" | "CANCELLED")),
        "no item is stuck, and each ran or was cancelled with the job: {states:?}"
    );
    assert!(
        counts.succeeded >= 700 && counts.cancelled > 0,
        "the job did work and was cancelled before it was done: {counts:?}"
    );
    assert_eq!(
        counts.failed + counts.skipped,
        0,
        "a repeated effect is answered as the first one was: {counts:?}"
    );

    // each run was cancelled at most once, whatever the processes say: one request to stop it in its journal, for a
    // run of an item that succeeded, and none for the run of an item that was cancelled with the job (which it left as it was)
    let doubled: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT run_id, count(*) FROM run_events WHERE event_type = 'run.stop_requested' \
         GROUP BY run_id HAVING count(*) > 1",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("stop requests");
    assert!(doubled.is_empty(), "a run was cancelled twice: {doubled:?}");
    let audited_twice: Vec<(String, i64)> = sqlx::query_as(
        "SELECT resource_id, count(*) FROM audit_records WHERE action = 'run.cancel_requested' \
         GROUP BY resource_id HAVING count(*) > 1",
    )
    .fetch_all(&f.w.db.pool)
    .await
    .expect("audit");
    assert!(
        audited_twice.is_empty(),
        "a run was audited as cancelled twice: {audited_twice:?}"
    );
    let wrong: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT i.state, r.status, \
                (SELECT count(*) FROM run_events e WHERE e.run_id = r.id AND e.event_type = 'run.stop_requested') \
         FROM batch_items i JOIN runs r ON r.id = i.run_id WHERE i.job_id = $1 \
           AND NOT ((i.state = 'SUCCEEDED' AND r.status = 'CANCELLED' \
                     AND (SELECT count(*) FROM run_events e WHERE e.run_id = r.id AND e.event_type = 'run.stop_requested') = 1) \
                 OR (i.state = 'CANCELLED' AND r.status = 'RUNNING' \
                     AND (SELECT count(*) FROM run_events e WHERE e.run_id = r.id AND e.event_type = 'run.stop_requested') = 0))",
    )
    .bind(created.id)
    .fetch_all(&f.w.db.pool)
    .await
    .expect("items against runs");
    assert!(
        wrong.is_empty(),
        "an item says what its run does not: {:?}",
        &wrong[..wrong.len().min(10)]
    );
    let cancelled_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE status = 'CANCELLED'")
            .fetch_one(&f.w.db.pool)
            .await
            .expect("count");
    assert_eq!(
        cancelled_runs,
        i64::from(counts.succeeded),
        "the job undid nothing and cancelled only what it counts"
    );

    // the run that was held when the job was created is one of its items
    let held: Option<String> =
        sqlx::query_scalar("SELECT state FROM batch_items WHERE job_id = $1 AND run_id = $2")
            .bind(created.id)
            .bind(held_run.as_uuid())
            .fetch_optional(&f.w.db.pool)
            .await
            .expect("the held run");
    assert!(
        held.is_some(),
        "the run whose transaction was open when the job was created is selected"
    );
    let distinct: BTreeSet<Uuid> =
        sqlx::query_scalar("SELECT run_id FROM batch_items WHERE job_id = $1")
            .bind(created.id)
            .fetch_all(&f.w.db.pool)
            .await
            .expect("items")
            .into_iter()
            .collect();
    let expected: BTreeSet<Uuid> = runs.iter().map(|run| run.as_uuid()).collect();
    assert_eq!(distinct, expected, "every run of the namespace, once");

    // the job ended once
    let endings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'batch.finished' AND resource_id = $1",
    )
    .bind(created.id.to_string())
    .fetch_one(&f.w.db.pool)
    .await
    .expect("audit");
    assert_eq!(endings, 1, "one record of the end of the job");

    // no transaction held two run locks and none deadlocked, with two processes, the API and the deaths at work
    let report = monitor.stop().await;
    println!(
        "{} ({:.1} s in all)",
        describe(&report),
        begun.elapsed().as_secs_f64()
    );
    assert!(
        report.most_held >= 1 && report.samples >= 20,
        "the monitor looked while the runs were locked, or it proves nothing: {}",
        describe(&report)
    );
    assert!(
        report.violations.is_empty(),
        "no transaction holds two run locks: {}",
        describe(&report)
    );
    // the statistics of the database report a deadlock a moment after it; give them that moment
    tokio::time::sleep(StdDuration::from_secs(2)).await;
    assert_eq!(
        database_deadlocks(&f.w.db.pool).await,
        deadlocks_before,
        "no deadlock"
    );
}
