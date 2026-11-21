//! The platform the scenarios run on: the real engine passes, driven by the engine's own loops (`sagat_execution_engine::loops`),
//! over the services and the runtime-role pool of a world, and a recorder that keeps what the product measures about itself.
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::{StreamExt as _, stream};
use metrics::{
    Counter, Gauge, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder, SharedString, Unit,
};
use sagat_application::batch::BatchService;
use sagat_application::batch_executor::{BatchExecutor, ExecutorSettings};
use sagat_application::clock::SystemClock;
use sagat_application::identity::RequestContext;
use sagat_application::operations::MonitorStore as _;
use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedule_firing::ScheduleFiring;
use sagat_application::schedules::{ScheduleService, ScheduleWrite};
use sagat_domain::definition::WorkflowId;
use sagat_domain::schedules::{Ceilings, RemainingActions, ScheduleName};
use sagat_execution_engine::loops;
use sagat_infra_ir::schedule_calendar::SpecCalendar;
use sagat_infra_postgres::{
    DatabaseClock, PgBatchStore, PgIdentityStore, PgRunStore, PgScheduleStore,
};
use sagat_scheduler::{
    BatchPass, BatchSettings, Keeper, KeeperSettings, Placement, SchedulePass, ScheduleSettings,
};
use serde_json::json;
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::support::engine::{Runs, World, chain, world};

// --- what the product measures about itself ---------------------------------------------------------------------------

/// The samples of every histogram the process records, by metric name.
#[derive(Default)]
pub struct Capture {
    histograms: Mutex<BTreeMap<String, Vec<f64>>>,
}

impl Capture {
    /// Takes the samples of `name` recorded so far and forgets them.
    pub fn take(&self, name: &str) -> Vec<f64> {
        self.histograms
            .lock()
            .expect("capture")
            .remove(name)
            .unwrap_or_default()
    }
}

struct Sink {
    name: String,
    capture: Arc<Capture>,
}

impl HistogramFn for Sink {
    fn record(&self, value: f64) {
        self.capture
            .histograms
            .lock()
            .expect("capture")
            .entry(self.name.clone())
            .or_default()
            .push(value);
    }
}

struct Recording(Arc<Capture>);

impl Recorder for Recording {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

    fn register_counter(&self, _: &Key, _: &Metadata<'_>) -> Counter {
        Counter::noop()
    }

    fn register_gauge(&self, _: &Key, _: &Metadata<'_>) -> Gauge {
        Gauge::noop()
    }

    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(Arc::new(Sink {
            name: key.name().to_owned(),
            capture: Arc::clone(&self.0),
        }))
    }
}

/// The recorder of this process (installed on first use; every scenario of a process shares it, and takes what it needs).
pub fn capture() -> Arc<Capture> {
    static CAPTURE: OnceLock<Arc<Capture>> = OnceLock::new();
    Arc::clone(CAPTURE.get_or_init(|| {
        let capture = Arc::new(Capture::default());
        metrics::set_global_recorder(Recording(Arc::clone(&capture)))
            .expect("no other recorder is installed in this process");
        capture
    }))
}

// --- the platform ------------------------------------------------------------------------------------------------------

/// A world (a real PostgreSQL, the migrations, two tenants) with a published workflow every scenario starts runs of.
pub struct Bench {
    pub w: Arc<World>,
    pub workflow: WorkflowId,
}

pub async fn bench() -> Bench {
    let _ = capture();
    let w = Arc::new(world().await);
    let (workflow, _) = w.publish(chain("benchmarked")).await;
    // The runs these scenarios start never finish (no worker serves them), and a namespace admits at most 10,000 active runs by
    // default: a burst of 10,000 schedules, or 10,000 runs for a batch job, would meet that quota, not what is measured. The
    // quota is raised for the namespace and the tenant, as an operator would for a platform that runs this many.
    room_for(&w, "active_runs", 1_000_000).await;
    room_for(&w, "tenant_active_runs", 1_000_000).await;
    Bench { w, workflow }
}

/// Room for `n` of a resource in the namespace of the world: the ceiling the operator sets, then the limit.
pub async fn room_for(w: &World, resource: &str, n: i64) {
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) VALUES ($1, $3, $2, 'operator')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(n)
    .bind(resource)
    .execute(&w.db.pool)
    .await
    .expect("a higher ceiling");
    if resource.starts_with("tenant_") {
        sqlx::query(
            "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
             VALUES ($1, NULL, $3, $2, 'admin')",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(n)
        .bind(resource)
        .execute(&w.db.pool)
        .await
        .expect("room in the tenant");
    } else {
        sqlx::query(
            "INSERT INTO quota_limits (tenant_id, namespace_id, resource, max_value, updated_by) \
             VALUES ($1, $2, $4, $3, 'admin')",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(w.namespace.id().as_uuid())
        .bind(n)
        .bind(resource)
        .execute(&w.db.pool)
        .await
        .expect("room in the namespace");
    }
}

/// Starts `n` runs of the workflow of the bench, `concurrency` at a time. Returns how long it took.
pub async fn start_runs(b: &Bench, workflow: WorkflowId, n: usize, concurrency: usize) -> Duration {
    let started = Instant::now();
    stream::iter(0..n)
        .map(|i| {
            let w = Arc::clone(&b.w);
            async move { w.start(workflow, &json!({ "i": i })).await }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    started.elapsed()
}

/// Creates `n` schedules of the bench's workflow, each with one action to take (a burst is one time of each, not a load that goes on every minute), as the API would (the service, the runtime role): every create is a
/// transaction with its revision, its audit record and the quota count. Returns how long it took.
pub async fn create_schedules(b: &Bench, n: usize, spread_ms: u64) -> Duration {
    room_for(&b.w, "schedules", i64::try_from(n).expect("small") + 10).await;
    let service = ScheduleService::new(
        PgScheduleStore::new(b.w.execution_pool.clone()),
        SpecCalendar,
        SystemClock,
        Ceilings::default(),
    );
    let started = Instant::now();
    let results = stream::iter(0..n)
        .map(|i| {
            let service = &service;
            let w = &b.w;
            let workflow = b.workflow;
            // schedule i is due `offset_ms` after the start of every minute: the same instant for all, or spread
            let offset_ms = if spread_ms == 0 {
                0
            } else {
                (i as u64) * spread_ms / (n as u64)
            };
            async move {
                let write = ScheduleWrite {
                    name: ScheduleName::new(&format!("bench-{i}")).expect("name"),
                    spec: spec_from_json(
                        &json!({"intervals": [{"everyMs": 60_000, "offsetMs": offset_ms}]}),
                        &Ceilings::HARD,
                    )
                    .expect("spec"),
                    policies: policies_from_json(&json!({"overlap": "ALLOW_ALL"}), &Ceilings::HARD)
                        .expect("policies"),
                    target: target_from_json(&json!({"workflowId": workflow.to_string()}))
                        .expect("target"),
                    input: json!({"region": "eu"}),
                    paused: false,
                    note: None,
                    remaining_actions: Some(RemainingActions::new(1).expect("one action")),
                };
                service
                    .create(
                        &w.admin,
                        w.namespace.id(),
                        write,
                        &RequestContext::default(),
                    )
                    .await
            }
        })
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await;
    for result in results {
        result.expect("create the schedule");
    }
    let took = started.elapsed();
    sqlx::query("ANALYZE schedules")
        .execute(&b.w.db.pool)
        .await
        .expect("analyze");
    took
}

/// Gives every schedule the time it is due next: the first whole minute `lead` seconds or more from now by the clock of the
/// database, plus the offset its spec has within the minute (`create_schedules`). Returns that minute: the instant of the first.
pub async fn make_due(w: &World, lead: f64) -> OffsetDateTime {
    let first: OffsetDateTime = sqlx::query_scalar(
        "SELECT date_trunc('minute', clock_timestamp() + make_interval(secs => $1)) + interval '1 minute'",
    )
    .bind(lead)
    .fetch_one(&w.db.pool)
    .await
    .expect("the time of the database");
    sqlx::query(
        "UPDATE schedules s SET \
                next_action_at = $1::timestamptz + make_interval(secs => (r.spec->'intervals'->0->>'offsetMs')::float8 / 1000), \
                next_actual_at = $1::timestamptz + make_interval(secs => (r.spec->'intervals'->0->>'offsetMs')::float8 / 1000), \
                state_version = s.state_version + 1 \
           FROM schedule_revisions r \
          WHERE r.tenant_id = s.tenant_id AND r.schedule_id = s.id AND r.revision = s.head_revision",
    )
    .bind(first)
    .execute(&w.db.pool)
    .await
    .expect("make the schedules due");
    first
}

// --- the passes, in the engine's loops ---------------------------------------------------------------------------------

/// What a pass did, round by round.
#[derive(Default)]
pub struct RoundStats {
    pub rounds: AtomicU64,
    pub worked: AtomicU64,
    pub steps: AtomicU64,
    pub refused: AtomicU64,
    pub failed: AtomicU64,
    /// How long each round that did work took, in milliseconds.
    pub work_ms: Mutex<Vec<f64>>,
}

impl RoundStats {
    pub fn record(&self, steps: u32, refused: u32, failed: u32, took: Duration) {
        self.rounds.fetch_add(1, Ordering::Relaxed);
        self.steps.fetch_add(u64::from(steps), Ordering::Relaxed);
        self.refused
            .fetch_add(u64::from(refused), Ordering::Relaxed);
        self.failed.fetch_add(u64::from(failed), Ordering::Relaxed);
        if steps > 0 {
            self.worked.fetch_add(1, Ordering::Relaxed);
            self.work_ms
                .lock()
                .expect("stats")
                .push(took.as_secs_f64() * 1_000.0);
        }
    }

    pub fn json(&self) -> serde_json::Value {
        json!({
            "rounds": self.rounds.load(Ordering::Relaxed),
            "rounds_that_worked": self.worked.load(Ordering::Relaxed),
            "steps": self.steps.load(Ordering::Relaxed),
            "refused": self.refused.load(Ordering::Relaxed),
            "failed": self.failed.load(Ordering::Relaxed),
            "work_round_ms": crate::measure::summary_json(&self.work_ms.lock().expect("stats")),
        })
    }
}

/// A loop of an engine that runs until it is stopped.
pub struct Running {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    pub stats: Arc<RoundStats>,
}

impl Running {
    pub async fn stop(self) -> Arc<RoundStats> {
        let _ = self.stop.send(true);
        self.task.await.expect("the loop ends");
        self.stats
    }
}

type EngineSchedules =
    SchedulePass<PgScheduleStore, SpecCalendar, DatabaseClock<SystemClock>, PgIdentityStore, Runs>;
type EngineBatches = BatchPass<PgBatchStore, Runs, PgIdentityStore, DatabaseClock<SystemClock>>;

/// One engine replica as the schedule pass sees it: its own pool, its own services and its own clock over the same database.
pub struct ScheduleReplica {
    clock: DatabaseClock<SystemClock>,
    pass: Arc<EngineSchedules>,
    keeper: Keeper<PgRunStore>,
    pub placement: Placement,
}

impl ScheduleReplica {
    pub async fn new(w: &World, name: &str, settings: ScheduleSettings) -> Self {
        let replica = w.replica_with_pool(16).await;
        let clock = DatabaseClock::new(replica.execution_pool.clone(), SystemClock);
        clock.sync().await.expect("the database clock");
        let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
        let keeper = Keeper::new(
            replica.store.clone(),
            placement.clone(),
            KeeperSettings {
                ttl: Duration::from_secs(6 * 3_600),
            },
        );
        let firing = ScheduleFiring::new(
            PgScheduleStore::new(replica.execution_pool.clone()),
            SpecCalendar,
            clock.clone(),
            PgIdentityStore::new(replica.execution_pool.clone()),
            replica.runs,
            Ceilings::default(),
        );
        Self {
            clock,
            pass: Arc::new(SchedulePass::new(firing, placement.clone(), settings)),
            keeper,
            placement,
        }
    }

    pub async fn keep(&self) {
        self.keeper.round().await.expect("keeper round");
    }

    /// Runs the engine's own loop (`loops::run_schedules`) with its default poll.
    pub fn spawn(self: &Arc<Self>, poll: Duration) -> Running {
        let (stop, receiver) = watch::channel(false);
        let stats = Arc::new(RoundStats::default());
        let (replica, recorded) = (Arc::clone(self), Arc::clone(&stats));
        let task = tokio::spawn(async move {
            loops::run_schedules(
                || {
                    let (replica, recorded) = (Arc::clone(&replica), Arc::clone(&recorded));
                    async move {
                        let started = Instant::now();
                        let round = loops::schedule_round(&replica.clock, &replica.pass).await;
                        recorded.record(
                            round.steps,
                            round.refused,
                            round.failed,
                            started.elapsed(),
                        );
                        round
                    }
                },
                poll,
                receiver,
            )
            .await;
        });
        Running { stop, task, stats }
    }
}

/// Lets the replicas take the shards between them (each announces itself, the first gives back what is beyond its share, the
/// second takes it).
pub async fn share_the_shards(replicas: &[&ScheduleReplica]) {
    for _ in 0..3 {
        for replica in replicas {
            replica.keep().await;
        }
    }
    let owned: usize = replicas
        .iter()
        .map(|replica| replica.placement.owned().len())
        .sum();
    assert_eq!(owned, 64, "every shard has an owner");
}

/// One engine replica as the batch pass sees it.
pub struct BatchReplica {
    clock: DatabaseClock<SystemClock>,
    pass: Arc<EngineBatches>,
    keeper: Keeper<PgRunStore>,
}

impl BatchReplica {
    pub async fn new(w: &World, name: &str, settings: BatchSettings) -> Self {
        let replica = w.replica_with_pool(16).await;
        let clock = DatabaseClock::new(replica.execution_pool.clone(), SystemClock);
        clock.sync().await.expect("the database clock");
        let placement = Placement::new(format!("{name}-{}", Uuid::now_v7()));
        let keeper = Keeper::new(
            replica.store.clone(),
            placement.clone(),
            KeeperSettings {
                ttl: Duration::from_secs(6 * 3_600),
            },
        );
        let executor = BatchExecutor::new(
            PgBatchStore::new(replica.execution_pool.clone()),
            replica.runs,
            PgIdentityStore::new(replica.execution_pool.clone()),
            clock.clone(),
            ExecutorSettings::default(),
        );
        Self {
            clock,
            pass: Arc::new(BatchPass::new(executor, placement, settings)),
            keeper,
        }
    }

    pub async fn keep(&self) {
        self.keeper.round().await.expect("keeper round");
    }

    /// Runs the engine's own loop (`loops::run_batches`) with its default poll.
    pub fn spawn(self: &Arc<Self>, poll: Duration) -> Running {
        let (stop, receiver) = watch::channel(false);
        let stats = Arc::new(RoundStats::default());
        let (replica, recorded) = (Arc::clone(self), Arc::clone(&stats));
        let task = tokio::spawn(async move {
            loops::run_batches(
                || {
                    let (replica, recorded) = (Arc::clone(&replica), Arc::clone(&recorded));
                    async move {
                        let started = Instant::now();
                        let round = loops::batch_round(&replica.clock, &replica.pass).await;
                        recorded.record(
                            round.steps,
                            round.refused,
                            round.failed,
                            started.elapsed(),
                        );
                        round
                    }
                },
                poll,
                receiver,
            )
            .await;
        });
        Running { stop, task, stats }
    }
}

/// How long the batch loop rests when a round found nothing due: the engine's default (`SAGAT_BATCHES_POLL_MS`, 1000), or
/// `SAGAT_PERF_BATCH_POLL_MS` for the variant that measures what the poll costs.
pub fn batch_poll() -> Duration {
    Duration::from_millis(
        std::env::var("SAGAT_PERF_BATCH_POLL_MS")
            .map_or(1_000, |value| value.parse().expect("milliseconds")),
    )
}

/// The batch service of the control plane, over the pool of the world and the clock of the database.
pub async fn batch_service(w: &World) -> BatchService<PgBatchStore, DatabaseClock<SystemClock>> {
    let clock = DatabaseClock::new(w.execution_pool.clone(), SystemClock);
    clock.sync().await.expect("the database clock");
    BatchService::new(PgBatchStore::new(w.execution_pool.clone()), clock)
}

/// A sampler of `scheduler_lag_seconds{pass="schedules"}` and `{pass="batches"}`: what the monitor of an engine reads, every
/// `every`, as `(seconds since the start, schedules, batches)`.
pub struct Sampler {
    stop: watch::Sender<bool>,
    task: JoinHandle<Vec<(f64, f64, f64)>>,
}

impl Sampler {
    pub fn spawn(w: &World, every: Duration) -> Self {
        let store = w.store.clone();
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(async move {
            let began = Instant::now();
            let mut samples = Vec::new();
            while !*receiver.borrow() {
                if let Ok(sample) = store.sample().await {
                    samples.push((
                        began.elapsed().as_secs_f64(),
                        sample.overdue.schedules,
                        sample.overdue.batches,
                    ));
                }
                tokio::time::sleep(every).await;
            }
            samples
        });
        Self { stop, task }
    }

    pub async fn stop(self) -> Vec<(f64, f64, f64)> {
        let _ = self.stop.send(true);
        self.task.await.expect("the sampler ends")
    }
}

/// An observer of the runs the schedules start, on a connection of its own: every `every` it asks how many schedules have a
/// run that it can see, with the database's time of the answer, as `(unix seconds, schedules with a run)`. Its view is what
/// somebody outside the engine sees, and it includes the time a commit takes to be visible.
pub struct Observer {
    stop: watch::Sender<bool>,
    task: JoinHandle<Vec<(f64, i64)>>,
}

impl Observer {
    pub fn spawn(w: &World, every: Duration) -> Self {
        let pool = w.db.pool.clone();
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            while !*receiver.borrow() {
                let row: Result<(f64, i64), _> = sqlx::query_as(
                    "SELECT extract(epoch FROM clock_timestamp())::float8, count(DISTINCT schedule_id) \
                       FROM runs WHERE schedule_id IS NOT NULL",
                )
                .fetch_one(&pool)
                .await;
                if let Ok(row) = row {
                    seen.push(row);
                }
                tokio::time::sleep(every).await;
            }
            seen
        });
        Self { stop, task }
    }

    pub async fn stop(self) -> Vec<(f64, i64)> {
        let _ = self.stop.send(true);
        self.task.await.expect("the observer ends")
    }
}
