//! Reproducible capacity measurements for durable workflow waits and tenant collection slots.
//!
//! The measurements are deliberately ignored in ordinary test runs. They exercise production
//! services and PostgreSQL transactions, record the machine and configuration, and publish observed
//! values without turning them into deployment capacity promises.
//!
//! ```sh
//! SAGAT_PERF_FRAGMENTS=/path/to/fragments \
//!   cargo test -p sagat-tests --test perf_workflow_primitives --release -- --ignored --nocapture
//! ```
//!
//! Each workload is its own ignored test and writes one report fragment (see `common::emit`).
#![allow(clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../../support/mod.rs"]
mod support;

mod chains;
mod collections;
mod common;
mod loops;
mod tenants;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream};
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine, instance_step_id};
use sagat_application::primitives::{
    CollectionAdmission, CollectionEntry, CollectionLimits, CollectionScopeId, CollectionStore,
    CollectionTransition, PrimitiveStoreError, SchedulerAuthority,
};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{ExecutionInstanceId, InstancePath, RunId};
use sagat_domain::identity::Subject;
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgCollectionStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use support::engine::{World, activity, chain, ctx, document, timer_node, with, world};
use tokio::sync::Semaphore;
use uuid::Uuid;

const DEFAULT_APPROVALS: usize = 10_000;
const DEFAULT_TIMERS: usize = 1_000;
const WAIT_LOAD_CONCURRENCY: usize = 32;
const IDLE_TICKS: usize = 50;
const SLOT_RUNS: usize = 64;
const TENANT_SLOT_LIMIT: usize = 8;
const SLOT_CALLERS: usize = 5;
const CONTROLLED_LOCK_HOLD: Duration = Duration::from_millis(100);

#[derive(Debug)]
struct StructuredJsonIrCodec;

impl IrCodec for StructuredJsonIrCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        IrCodec::check_draft(&JsonIrCodec, document)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        JsonIrCodec.parse_structured(document)
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        IrCodec::diff(&JsonIrCodec, from, to)
    }
}

#[derive(Debug)]
struct WaitMeasurements {
    approvals: usize,
    timers: usize,
    approval_load: Duration,
    timer_load: Duration,
    approval_start_samples: Vec<Duration>,
    timer_start_samples: Vec<Duration>,
    approval_open: Duration,
    approval_open_ticks: u32,
    approval_schedule_ms: Vec<f64>,
    timer_schedule_ms: Vec<f64>,
    idle_ticks: Vec<Duration>,
    bytes_per_approval: i64,
    bytes_per_timer: i64,
    journal_bytes_per_approval: f64,
    journal_bytes_per_timer: f64,
    waiting_without_worker: i64,
    active_worker_sessions: i64,
    scheduler_skipped: u32,
    scheduler_fenced: u32,
    scheduler_failed: u32,
    process_rss_before: u64,
    process_rss_after: u64,
    resource_points: Vec<(String, common::Resources)>,
}

#[derive(Debug)]
struct SlotMeasurements {
    attempted: usize,
    admitted: usize,
    rejected: usize,
    accepted_latency: Vec<Duration>,
    rejected_latency: Vec<Duration>,
    observed_lock_waiters: i64,
    active_slots: i32,
    persisted_iterations: i64,
}

fn configured_count(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn percentile<T: Copy>(sorted: &[T], percentile: f64) -> T {
    let index = ((sorted.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn duration_ms(value: Duration) -> f64 {
    value.as_secs_f64() * 1_000.0
}

fn duration_row(name: &str, mut samples: Vec<Duration>) -> String {
    samples.sort_unstable();
    format!(
        "| {name} | {} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
        samples.len(),
        duration_ms(percentile(&samples, 0.50)),
        duration_ms(percentile(&samples, 0.95)),
        duration_ms(percentile(&samples, 0.99)),
        duration_ms(*samples.last().expect("duration samples")),
    )
}

fn float_row(name: &str, mut samples: Vec<f64>) -> String {
    samples.sort_by(f64::total_cmp);
    format!(
        "| {name} | {} | {:.1} ms | {:.1} ms | {:.1} ms | {:.1} ms |\n",
        samples.len(),
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        percentile(&samples, 0.99),
        samples.last().expect("floating samples"),
    )
}

fn command(name: &str, arguments: &[&str]) -> String {
    std::process::Command::new(name)
        .args(arguments)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn environment() -> String {
    let cpu = if cfg!(target_os = "macos") {
        command("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else {
        command(
            "sh",
            &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"],
        )
    };
    let cores = std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(0);
    format!(
        "- CPU: {cpu} ({cores} logical cores)\n- OS: {}\n- Load average at start: {}\n- Build profile: {}\n- Commit: {}\n- PostgreSQL: Docker container on the same host\n- SQLx pool: 8 connections\n- Services: in-process; no client network latency",
        command("uname", &["-srm"]),
        command("sh", &["-c", "uptime | sed 's/.*load averages*: //'"]),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        command("git", &["rev-parse", "--short", "HEAD"]),
    )
}

fn process_rss_bytes() -> u64 {
    command("ps", &["-o", "rss=", "-p", &std::process::id().to_string()])
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        .saturating_mul(1_024)
}

async fn relation_bytes(world: &World, relations: &[&str]) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum(pg_total_relation_size(c.oid)),0)::bigint \
         FROM pg_class c WHERE c.relname = ANY($1)",
    )
    .bind(relations)
    .fetch_one(&world.db.pool)
    .await
    .expect("relation size")
}

fn approval_workflow(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "review",
        "nodes": [
            {
                "id": "review",
                "type": "approval",
                "timeoutMs": 86_400_000,
                "policy": { "roles": ["operator"], "excludeInitiator": true },
                "onApprove": "approved",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            { "id": "approved", "type": "activity", "activity": "approval.approved" },
            { "id": "rejected", "type": "activity", "activity": "approval.rejected" },
            { "id": "expired", "type": "activity", "activity": "approval.expired" }
        ]
    })
}

async fn start_population(
    world: &World,
    workflow: WorkflowId,
    prefix: &str,
    count: usize,
) -> (Vec<RunId>, Vec<Duration>) {
    let results = stream::iter(0..count)
        .map(|index| async move {
            let started = Instant::now();
            let run = world
                .start_with(
                    workflow,
                    &json!({ "index": index }),
                    &format!("{prefix}-{index:06}"),
                    None,
                )
                .await
                .expect("start waiting run");
            (run, started.elapsed())
        })
        .buffer_unordered(WAIT_LOAD_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    results.into_iter().unzip()
}

async fn measure_waits(world: &World) -> WaitMeasurements {
    let approvals = configured_count("SAGAT_PRIMITIVES_APPROVALS", DEFAULT_APPROVALS);
    let timers = configured_count("SAGAT_PRIMITIVES_TIMERS", DEFAULT_TIMERS);
    let quota_room = u64::try_from(approvals + timers + SLOT_RUNS + 1_000).expect("quota room");
    for resource in [QuotaResource::ActiveRuns, QuotaResource::TenantActiveRuns] {
        world
            .quotas
            .grant_ceiling(world.admin.tenant_id(), resource, Some(quota_room))
            .await
            .expect("benchmark active-run ceiling");
    }
    world
        .quotas
        .set(
            &world.admin,
            Some(world.namespace.id()),
            QuotaResource::ActiveRuns,
            quota_room,
            &ctx(),
        )
        .await
        .expect("benchmark namespace active-run limit");
    world
        .quotas
        .set(
            &world.admin,
            None,
            QuotaResource::TenantActiveRuns,
            quota_room,
            &ctx(),
        )
        .await
        .expect("benchmark tenant active-run limit");
    let process_rss_before = process_rss_bytes();
    let mut resource_points = vec![("no waits".to_owned(), common::resources(world).await)];
    let approval_relations = [
        "runs",
        "steps",
        "run_events",
        "approval_requests",
        "approval_timers",
        "event_outbox",
        "audit_records",
    ];
    let timer_relations = [
        "runs",
        "steps",
        "run_events",
        "event_outbox",
        "audit_records",
    ];
    let approval_before = relation_bytes(world, &approval_relations).await;
    let (approval_workflow, _) = world
        .publish(approval_workflow(&format!(
            "perf-approval-{}",
            Uuid::now_v7()
        )))
        .await;
    let approval_started = Instant::now();
    let (_approval_runs, approval_start_samples) =
        start_population(world, approval_workflow, "perf-approval", approvals).await;
    let approval_load = approval_started.elapsed();
    resource_points.push((
        format!("{approvals} approvals started, none opened yet"),
        common::resources(world).await,
    ));

    let placement = Placement::new(format!("perf-approval-scheduler-{}", Uuid::now_v7()));
    Keeper::new(
        world.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    let scheduler = Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let approval_opened_at = Instant::now();
    let mut opened = 0usize;
    let mut approval_open_ticks = 0u32;
    let mut scheduler_skipped = 0u32;
    let mut scheduler_fenced = 0u32;
    let mut scheduler_failed = 0u32;
    while opened < approvals {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "approval scheduler failure: {report:?}");
        opened += usize::try_from(report.approvals_opened).expect("opened count");
        scheduler_skipped += report.skipped;
        scheduler_fenced += report.fenced;
        scheduler_failed += report.failed;
        approval_open_ticks += 1;
        assert!(
            approval_open_ticks < u32::try_from(approvals / 10 + 500).expect("tick bound"),
            "approval opening stalled at {opened}/{approvals}"
        );
    }
    let approval_open = approval_opened_at.elapsed();
    resource_points.push((
        format!("{approvals} approvals pending"),
        common::resources(world).await,
    ));
    let approval_after = relation_bytes(world, &approval_relations).await;

    let timer_before = relation_bytes(world, &timer_relations).await;
    let timer_name = format!("perf-timer-{}", Uuid::now_v7());
    let (timer_workflow, _) = world
        .publish(document(
            &timer_name,
            "pause",
            vec![
                with(timer_node("pause", 3_600_000), "next", json!("work")),
                activity("work"),
            ],
        ))
        .await;
    let timer_started = Instant::now();
    let (_timer_runs, timer_start_samples) =
        start_population(world, timer_workflow, "perf-timer", timers).await;
    let timer_load = timer_started.elapsed();
    resource_points.push((
        format!("{approvals} approvals pending + {timers} timers waiting"),
        common::resources(world).await,
    ));
    let timer_after = relation_bytes(world, &timer_relations).await;

    sqlx::query("ANALYZE steps, approval_requests, approval_timers")
        .execute(&world.db.pool)
        .await
        .expect("analyze wait projections");
    let mut idle_ticks = Vec::with_capacity(IDLE_TICKS);
    for _ in 0..IDLE_TICKS {
        let started = Instant::now();
        let report = scheduler.tick().await;
        idle_ticks.push(started.elapsed());
        scheduler_skipped += report.skipped;
        scheduler_fenced += report.fenced;
        scheduler_failed += report.failed;
        assert!(
            !report.did_work() && report.failed == 0,
            "idle wait tick: {report:?}"
        );
    }
    resource_points.push((
        format!("after {IDLE_TICKS} idle ticks over all waits"),
        common::resources(world).await,
    ));
    let (first, last) = (
        resource_points[0].1,
        resource_points[resource_points.len() - 1].1,
    );
    // Zero dedicated resources per wait: 11,000 pending waits may not add a thread, a PostgreSQL
    // connection beyond the pool, or a transaction held open.
    assert!(
        last.threads <= first.threads + 4,
        "threads grew with the waits: {} -> {}",
        first.threads,
        last.threads
    );
    assert!(
        last.pg_connections <= 8 + 2,
        "PostgreSQL connections exceed the pool plus the probe: {}",
        last.pg_connections
    );
    for (label, point) in &resource_points {
        assert_eq!(
            point.pg_idle_in_transaction, 0,
            "a transaction is held open at '{label}'"
        );
    }

    let approval_schedule_ms: Vec<f64> = sqlx::query_scalar(
        "SELECT extract(epoch FROM (request.created_at-run.created_at))*1000::float8 \
         FROM approval_requests request JOIN runs run \
           ON run.tenant_id=request.tenant_id AND run.id=request.run_id \
         WHERE request.tenant_id=$1 AND request.status='PENDING'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .fetch_all(&world.db.pool)
    .await
    .expect("approval schedule delay");
    assert_eq!(approval_schedule_ms.len(), approvals);
    let timer_schedule_ms: Vec<f64> = sqlx::query_scalar(
        "SELECT extract(epoch FROM (step.updated_at-run.created_at))*1000::float8 \
         FROM steps step JOIN runs run \
           ON run.tenant_id=step.tenant_id AND run.id=step.run_id \
         WHERE step.tenant_id=$1 AND step.status='WAITING' AND step.waiting_kind='TIMER'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .fetch_all(&world.db.pool)
    .await
    .expect("timer schedule delay");
    assert_eq!(timer_schedule_ms.len(), timers);
    let waiting_without_worker: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND status='WAITING' \
           AND waiting_kind IN ('APPROVAL','TIMER') AND leased_by IS NULL \
           AND lease_token_hash IS NULL AND lease_expires_at IS NULL",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("worker-free waits");
    assert_eq!(
        waiting_without_worker,
        i64::try_from(approvals + timers).expect("wait count")
    );
    let active_worker_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM worker_sessions WHERE tenant_id=$1 AND expires_at>clock_timestamp()",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("worker sessions");
    assert_eq!(active_worker_sessions, 0);
    let (approval_journal_bytes, approval_run_count): (i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(pg_column_size(event.*)),0)::bigint,count(DISTINCT run.id)::bigint \
         FROM runs run JOIN approval_requests request \
           ON request.tenant_id=run.tenant_id AND request.run_id=run.id \
         JOIN run_events event ON event.tenant_id=run.tenant_id AND event.run_id=run.id \
         WHERE run.tenant_id=$1",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("approval journal bytes");
    let (timer_journal_bytes, timer_run_count): (i64, i64) = sqlx::query_as(
        "SELECT coalesce(sum(pg_column_size(event.*)),0)::bigint,count(DISTINCT run.id)::bigint \
         FROM runs run JOIN steps step \
           ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
         JOIN run_events event ON event.tenant_id=run.tenant_id AND event.run_id=run.id \
         WHERE run.tenant_id=$1 AND step.waiting_kind='TIMER'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("timer journal bytes");

    WaitMeasurements {
        approvals,
        timers,
        approval_load,
        timer_load,
        approval_start_samples,
        timer_start_samples,
        approval_open,
        approval_open_ticks,
        approval_schedule_ms,
        timer_schedule_ms,
        idle_ticks,
        bytes_per_approval: (approval_after - approval_before)
            / i64::try_from(approvals).expect("approval count"),
        bytes_per_timer: (timer_after - timer_before) / i64::try_from(timers).expect("timer count"),
        journal_bytes_per_approval: approval_journal_bytes as f64 / approval_run_count as f64,
        journal_bytes_per_timer: timer_journal_bytes as f64 / timer_run_count as f64,
        waiting_without_worker,
        active_worker_sessions,
        scheduler_skipped,
        scheduler_fenced,
        scheduler_failed,
        process_rss_before,
        process_rss_after: process_rss_bytes(),
        resource_points,
    }
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

async fn retain_payload(world: &World, bytes: &[u8]) -> Digest {
    let value = digest(bytes);
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(value.as_bytes().as_slice())
    .bind(bytes)
    .execute(&world.db.pool)
    .await
    .expect("retain collection payload");
    value
}

fn collection_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "items",
        "bodies": {
            "line": {
                "entrypoint": "work",
                "nodes": [
                    { "id": "work", "type": "activity", "activity": "line.process", "next": "returned" },
                    { "id": "returned", "type": "return", "result": "steps.work.output" }
                ]
            }
        },
        "nodes": [
            {
                "id": "items",
                "type": "forEach",
                "items": "input.items",
                "body": "line",
                "result": "bodyResult",
                "maxItems": 1,
                "maxConcurrency": 1,
                "next": "done"
            },
            { "id": "done", "type": "end", "outputExpr": "steps.items.output" }
        ]
    })
}

async fn collection_definition(world: &World) -> (WorkflowId, WorkflowVersionId, DefinitionCache) {
    let name = format!("perf-collection-{}", Uuid::now_v7());
    let (workflow, _) = world.publish(chain(&name)).await;
    let document = serde_json::to_vec(&collection_document(&name)).expect("collection document");
    JsonIrCodec
        .parse_structured(&document)
        .expect("structured collection");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id,tenant_id,workflow_id,version_number,digest,document,published_by,published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,$7)",
    )
    .bind(version.as_uuid())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(digest(&document).to_hex())
    .bind(document)
    .bind(world.admin.subject().as_str())
    .bind(support::now())
    .execute(&world.db.pool)
    .await
    .expect("trusted collection version");
    (
        workflow,
        version,
        DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
    )
}

async fn scheduler_authority(world: &World, run: RunId) -> SchedulerAuthority {
    let owner = "primitive-capacity-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET \
           generation=CASE WHEN owner=$2 AND expires_at>clock_timestamp() \
                           THEN generation ELSE generation+1 END, \
           owner=$2,expires_at=clock_timestamp()+interval '1 hour', \
           acquired_at=clock_timestamp() \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(&world.db.pool)
    .await
    .expect("scheduler authority");
    SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("scheduler generation"),
    )
}

async fn collection_run(
    world: &World,
    workflow: WorkflowId,
    version: WorkflowVersionId,
    owner: &ExecutionInstanceId,
) -> RunId {
    let run = RunId::generate();
    let input = retain_payload(world, b"{}").await;
    let now = support::now();
    sqlx::query(
        "INSERT INTO runs \
         (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status,run_version, \
          input_digest,started_by,trace_id,created_at,updated_at,root_run_id) \
         VALUES ($1,$2,$3,$4,$5,'RUNNING',0,$6,'capacity-benchmark', \
                 '0af7651916cd43dd8448eb211c80319c',$7,$7,$1)",
    )
    .bind(run.as_uuid())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(version.as_uuid())
    .bind(input.as_bytes().as_slice())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("collection run");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,updated_at, \
          instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,$3,$4,$5,'PENDING',$6,'{}','{}')",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .bind(owner.node().as_str())
    .bind(instance_step_id(version, run, owner))
    .bind(world.namespace.id().as_uuid())
    .bind(now)
    .execute(&world.db.pool)
    .await
    .expect("collection owner");
    run
}

fn transition(
    world: &World,
    run: RunId,
    version: u64,
    owner: ExecutionInstanceId,
    authority: SchedulerAuthority,
) -> CollectionTransition {
    CollectionTransition::new(
        world.admin.tenant_id(),
        world.namespace.id(),
        run,
        version,
        owner,
        authority,
        Subject::engine(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        support::now(),
        None,
    )
}

async fn measure_slot_contention(world: &World) -> SlotMeasurements {
    let tenant = world.admin.tenant_id().as_uuid();
    let owner_node = NodeId::parse("items").expect("collection owner");
    let owner = ExecutionInstanceId::root(owner_node.clone());
    let (workflow, version, definitions) = collection_definition(world).await;
    sqlx::query(
        "INSERT INTO tenant_collection_policies \
         (tenant_id,max_active_iterations,updated_by) VALUES ($1,$2,'capacity-benchmark') \
         ON CONFLICT (tenant_id) DO UPDATE SET max_active_iterations=EXCLUDED.max_active_iterations, \
           updated_by=EXCLUDED.updated_by,updated_at=clock_timestamp()",
    )
    .bind(tenant)
    .bind(i32::try_from(TENANT_SLOT_LIMIT).expect("tenant slot limit"))
    .execute(&world.db.pool)
    .await
    .expect("tenant slot policy");
    let store = PgCollectionStore::with_definition_cache(world.db.pool.clone(), definitions);
    let snapshot = retain_payload(world, b"[1]").await;
    let item = retain_payload(world, b"1").await;
    let mut admissions = Vec::with_capacity(SLOT_RUNS);
    for _ in 0..SLOT_RUNS {
        let run = collection_run(world, workflow, version, &owner).await;
        let authority = scheduler_authority(world, run).await;
        let scope = CollectionScopeId::generate();
        let entered = store
            .enter_collection(
                &CollectionEntry::new(
                    transition(world, run, 0, owner.clone(), authority.clone()),
                    scope,
                    1,
                    snapshot,
                    CollectionLimits::new(1, 1, 100_000, 8).expect("collection limits"),
                )
                .expect("collection entry"),
            )
            .await
            .expect("enter collection");
        let entry = ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(owner_node.clone(), 0)
                .expect("collection path"),
            NodeId::parse("work").expect("body activity"),
        );
        admissions.push(
            CollectionAdmission::new(
                transition(world, run, entered.run_version, owner.clone(), authority),
                scope,
                0,
                entry,
                ActivityName::parse("line.process").expect("activity"),
                item,
                1,
                Uuid::now_v7(),
            )
            .expect("collection admission"),
        );
    }

    let mut blocker = world.db.pool.begin().await.expect("slot lock transaction");
    sqlx::query(
        "SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=$1 FOR UPDATE",
    )
    .bind(tenant)
    .fetch_one(&mut *blocker)
    .await
    .expect("lock tenant usage");
    let permits = Arc::new(Semaphore::new(SLOT_CALLERS));
    let handles = admissions
        .into_iter()
        .map(|admission| {
            let store = store.clone();
            let permits = Arc::clone(&permits);
            tokio::spawn(async move {
                let _permit = permits.acquire_owned().await.expect("caller permit");
                let started = Instant::now();
                let result = store.admit_collection_activity(&admission).await;
                (started.elapsed(), result)
            })
        })
        .collect::<Vec<_>>();
    tokio::time::sleep(CONTROLLED_LOCK_HOLD / 2).await;
    let observed_lock_waiters: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname=current_database() AND wait_event_type='Lock'",
    )
    .fetch_one(&world.db.pool)
    .await
    .expect("observe lock contention");
    assert!(
        observed_lock_waiters > 0,
        "the controlled tenant lock produced no waiter"
    );
    tokio::time::sleep(CONTROLLED_LOCK_HOLD / 2).await;
    blocker.commit().await.expect("release tenant usage lock");

    let mut accepted_latency = Vec::new();
    let mut rejected_latency = Vec::new();
    for handle in handles {
        let (elapsed, result) = handle.await.expect("admission task");
        match result {
            Ok(_) => accepted_latency.push(elapsed),
            Err(PrimitiveStoreError::AtCapacity) => rejected_latency.push(elapsed),
            Err(error) => panic!("unexpected admission failure: {error}"),
        }
    }
    assert_eq!(accepted_latency.len(), TENANT_SLOT_LIMIT);
    assert_eq!(rejected_latency.len(), SLOT_RUNS - TENANT_SLOT_LIMIT);
    let row = sqlx::query(
        "SELECT usage.active_iterations, \
           (SELECT count(*) FROM collection_iterations iteration \
             WHERE iteration.tenant_id=usage.tenant_id) AS persisted \
         FROM tenant_iteration_usage usage WHERE usage.tenant_id=$1",
    )
    .bind(tenant)
    .fetch_one(&world.db.pool)
    .await
    .expect("slot result");
    let active_slots = row.get::<i32, _>("active_iterations");
    let persisted_iterations = row.get::<i64, _>("persisted");
    assert_eq!(
        active_slots,
        i32::try_from(TENANT_SLOT_LIMIT).expect("slot limit")
    );
    assert_eq!(persisted_iterations, i64::from(active_slots));

    SlotMeasurements {
        attempted: SLOT_RUNS,
        admitted: accepted_latency.len(),
        rejected: rejected_latency.len(),
        accepted_latency,
        rejected_latency,
        observed_lock_waiters,
        active_slots,
        persisted_iterations,
    }
}

fn report(waits: WaitMeasurements, slots: SlotMeasurements) -> String {
    let resource_lines = waits
        .resource_points
        .iter()
        .map(|(label, point)| format!("  - {label}: {point}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut report = format!(
        "### Durable waits and the tenant slot lock\n\n{}\n\n#### Configuration\n\n- {} approval runs, loaded with {} concurrent callers\n- {} one-hour timer runs, loaded with {} concurrent callers\n- scheduler batch 200, scheduler concurrency 8, {} idle ticks\n- {} collection runs contending for {} tenant slots with {} concurrent callers\n- tenant usage row held for {} ms to make lock contention observable\n\n#### Latency observations\n\n| Operation | Samples | p50 | p95 | p99 | max |\n|---|---:|---:|---:|---:|---:|\n",
        environment(),
        waits.approvals,
        WAIT_LOAD_CONCURRENCY,
        waits.timers,
        WAIT_LOAD_CONCURRENCY,
        IDLE_TICKS,
        slots.attempted,
        TENANT_SLOT_LIMIT,
        SLOT_CALLERS,
        CONTROLLED_LOCK_HOLD.as_millis(),
    );
    report.push_str(&duration_row(
        "approval start transaction",
        waits.approval_start_samples,
    ));
    report.push_str(&duration_row(
        "timer start transaction",
        waits.timer_start_samples,
    ));
    report.push_str(&float_row(
        "approval scheduling delay (run created to request persisted)",
        waits.approval_schedule_ms,
    ));
    report.push_str(&float_row(
        "timer scheduling delay (run created to wait persisted)",
        waits.timer_schedule_ms,
    ));
    report.push_str(&duration_row(
        "scheduler tick with all waits idle",
        waits.idle_ticks,
    ));
    report.push_str(&duration_row(
        "collection admission accepted under tenant-slot contention",
        slots.accepted_latency,
    ));
    report.push_str(&duration_row(
        "collection admission rejected at tenant capacity",
        slots.rejected_latency,
    ));
    report.push_str(&format!(
        "\n#### Throughput and storage\n\n- Approval run creation: {:.1} s, {:.0} runs/s.\n- Approval opening: {:.1} s, {:.0} approvals/s across {} scheduler ticks.\n- Timer run creation and wait persistence: {:.1} s, {:.0} runs/s.\n- Observed relation growth: {} bytes per suspended approval and {} bytes per suspended timer. This is a delta over shared PostgreSQL relations and includes index allocation noise.\n- Journal row bytes: {:.0} bytes per approval run and {:.0} bytes per timer run, measured with `pg_column_size` and excluding index/toast overhead.\n- Test-process RSS: {} bytes before loading and {} bytes after measurement, a signed change of {} bytes. PostgreSQL container RSS was not isolated in this run.\n\n#### Correctness observations\n\n- Resources while the waits are pending (the engine, scheduler and worker service run in this process; PostgreSQL connections are every backend of the benchmark database):\n{resource_lines}\n- {}/{} collection admissions committed; {} were rejected with the typed tenant-capacity error. Durable tenant usage and persisted iterations both equal {}.\n- PostgreSQL exposed {} lock waiters while the tenant usage row was deliberately held. Admission latencies therefore include a known {} ms contention interval plus connection-pool and serialization delay.\n- {} approval/timer steps are durably waiting with no lease token, lease expiry or worker identity. Active worker sessions: {}.\n- Scheduler outcomes while opening and observing waits: {} skipped, {} fenced and {} failed. Explicit client transaction retries: 0; every start and admission was submitted once.\n\n#### Notes\n\n- The first default-population attempt was rejected after reaching the platform's default 10,000 active-run namespace quota. The benchmark now grants and configures explicit headroom before loading; this preserves production admission behavior while isolating the wait measurement.\n- Approval and timer runs use the supported publish/start/scheduler paths. Waiting consumes database rows and scheduler index scans; it does not allocate one process, connection, worker session or broker consumer per wait.\n- The shared development host, local container, eight-connection pool and in-process services are not representative of a production cluster. Repeat on target hardware before sizing.\n- The controlled slot test measures one tenant and one database. It proves exact enforcement under cross-run contention; it does not establish a cluster-wide throughput floor.\n- No timeout, queue-delay or capacity ceiling is inferred beyond the populations and concurrency recorded above.\n",
        waits.approval_load.as_secs_f64(),
        waits.approvals as f64 / waits.approval_load.as_secs_f64(),
        waits.approval_open.as_secs_f64(),
        waits.approvals as f64 / waits.approval_open.as_secs_f64(),
        waits.approval_open_ticks,
        waits.timer_load.as_secs_f64(),
        waits.timers as f64 / waits.timer_load.as_secs_f64(),
        waits.bytes_per_approval,
        waits.bytes_per_timer,
        waits.journal_bytes_per_approval,
        waits.journal_bytes_per_timer,
        waits.process_rss_before,
        waits.process_rss_after,
        i128::from(waits.process_rss_after) - i128::from(waits.process_rss_before),
        slots.admitted,
        slots.attempted,
        slots.rejected,
        slots.active_slots,
        slots.observed_lock_waiters,
        CONTROLLED_LOCK_HOLD.as_millis(),
        waits.waiting_without_worker,
        waits.active_worker_sessions,
        waits.scheduler_skipped,
        waits.scheduler_fenced,
        waits.scheduler_failed,
    ));
    assert_eq!(slots.persisted_iterations, i64::from(slots.active_slots));
    report
}

#[tokio::test]
#[ignore = "capacity benchmark: run explicitly with --ignored"]
async fn measure_durable_waits_and_cross_run_tenant_slot_contention() {
    let world = world().await;
    let waits = measure_waits(&world).await;
    let slots = measure_slot_contention(&world).await;
    let report = report(waits, slots);
    common::emit("waits", &format!("{}\n{}\n", common::machine(), report));
}
