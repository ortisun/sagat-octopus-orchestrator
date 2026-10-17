//! Continuation chains through the public runtime: the boundary waits for quiescence (D296), the
//! tree budget is shared with successors (D301) and nothing the chain owns is ever deleted (D298).
//!
//! Open work at a boundary cannot be produced by the published workflow shapes (an activity or a
//! signal-controlled wait precedes the continuation), so the tests add one consistent obligation to
//! the projection by hand and say so. Everything after that point is the real scheduler and store.
#![allow(clippy::expect_used)]

use std::time::Duration as StdDuration;

use sagat_application::chain_progress::{
    ChainHandoffCommand, ChainProgressError, ChainProgressStore, ContinuationSelection,
};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{ChainError, ChainScope, QuiescenceBlocker, RunId, RunStatus};
use sagat_domain::identity::Subject;
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{
    ContinuationAdvanceError, Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{Sched, World, world};

fn continuation_document(name: &str, max_segments: u16) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": max_segments, "timeoutMs": 3_600_000}
    })
}

/// A scheduler that holds every shard, the way one replica of the engine does.
pub(crate) async fn scheduler(world: &World, label: &str) -> Sched {
    let placement = Placement::new(format!("chains-{label}-{}", Uuid::now_v7()));
    Keeper::new(
        world.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(world.cache.clone())
    .with_placement(placement)
}

/// A chain whose first segment finished its activity and now sits at the continuation boundary.
struct Boundary {
    world: World,
    chain: RunId,
}

impl Boundary {
    async fn reach(label: &str) -> Self {
        let world = world().await;
        let name = format!("chain-{label}-{}", Uuid::now_v7());
        let (workflow, _) = world.publish(continuation_document(&name, 3)).await;
        let chain = world
            .start(workflow, &json!({"segment": 1, "next": {"segment": 2}}))
            .await;
        let session = world.session(&["segment.work"]).await;
        let task = world.one_task(&session).await;
        world
            .complete(&session, &task, &json!({"processed": true}))
            .await
            .expect("complete the activity");
        Self { world, chain }
    }

    /// Adds one open step to the run's projection, the shape the matching `StepState` has.
    async fn obstruct(&self, kind: QuiescenceBlocker) {
        let (status, waiting) = match kind {
            QuiescenceBlocker::SiblingWork => ("READY", None),
            QuiescenceBlocker::Timer => ("WAITING", Some("TIMER")),
            QuiescenceBlocker::Subscription => ("WAITING", Some("EVENT")),
            QuiescenceBlocker::Lease => ("LEASED", None),
            other => panic!("{other:?} is not seeded by this helper"),
        };
        sqlx::query("DELETE FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='ghost'")
            .bind(self.world.admin.tenant_id().as_uuid())
            .bind(self.chain.as_uuid())
            .execute(&self.world.db.pool)
            .await
            .expect("clear the previous obstruction");
        let leased = status == "LEASED";
        sqlx::query(
            "INSERT INTO steps (tenant_id,run_id,node,step_id,namespace_id,status,waiting_kind, \
                                activity,ready_at,due_at,event_type,correlation_key, \
                                lease_token_hash,lease_expires_at,leased_by,routed,updated_at, \
                                instance_path_nodes,instance_path_indices) \
             VALUES ($1,$2,'ghost',gen_random_uuid(),$3,$4,$5, \
                     CASE WHEN $4='READY' THEN 'ghost.work' END, now(), \
                     CASE WHEN $5 IS NOT NULL THEN now()+interval '1 day' END, \
                     CASE WHEN $5='EVENT' THEN 'order.paid' END, \
                     CASE WHEN $5='EVENT' THEN 'k' END, \
                     CASE WHEN $6 THEN decode('00','hex') END, \
                     CASE WHEN $6 THEN now()+interval '1 day' END, \
                     CASE WHEN $6 THEN 'w' END, false, now(), '{}', '{}')",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .bind(self.world.namespace.id().as_uuid())
        .bind(status)
        .bind(waiting)
        .bind(leased)
        .execute(&self.world.db.pool)
        .await
        .expect("seed the open step");
    }

    /// The open step finishes and is routed: the run is quiescent again.
    async fn settle(&self) {
        sqlx::query(
            "UPDATE steps SET status='SUCCEEDED',waiting_kind=NULL,due_at=NULL,lease_token_hash=NULL, \
                    lease_expires_at=NULL,leased_by=NULL,routed=true,updated_at=now() \
             WHERE tenant_id=$1 AND run_id=$2 AND node='ghost'",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .execute(&self.world.db.pool)
        .await
        .expect("settle the open step");
    }

    async fn chain_row(&self) -> (Uuid, i32, i64, Option<String>) {
        sqlx::query_as(
            "SELECT current_run_id,segment_count,revision,final_outcome FROM execution_chains \
             WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.world.db.pool)
        .await
        .expect("chain row")
    }

    async fn journal_len(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
            .bind(self.world.admin.tenant_id().as_uuid())
            .bind(self.chain.as_uuid())
            .fetch_one(&self.world.db.pool)
            .await
            .expect("journal length")
    }
}

impl Boundary {
    /// The write the scheduler would issue for this boundary, sent straight to the store, so the
    /// proof under the chain and run locks is exercised without the scheduler's own pre-check.
    async fn write_directly(
        &self,
    ) -> Result<sagat_application::chain_progress::ChainHandoffWrite, ChainProgressError> {
        let tenant = self.world.admin.tenant_id();
        let scope = ChainScope::new(tenant, self.world.namespace.id());
        let (revision, version): (i64, i64) = sqlx::query_as(
            "SELECT chain.revision,run.run_version FROM execution_chains chain \
               JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
             WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
        )
        .bind(tenant.as_uuid())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.world.db.pool)
        .await
        .expect("chain revisions");
        let owner = "chains-direct-write";
        let generation: i64 = sqlx::query_scalar(
            "UPDATE scheduler_shards SET generation=generation+1,owner=$2, \
                    expires_at=clock_timestamp()+interval '1 hour',acquired_at=clock_timestamp() \
             WHERE shard=run_shard($1) RETURNING generation",
        )
        .bind(self.chain.as_uuid())
        .bind(owner)
        .fetch_one(&self.world.db.pool)
        .await
        .expect("scheduler fence");
        let selection = ContinuationSelection::new(
            boundary_node(),
            serde_json::to_vec(&json!({"segment": 2})).expect("input"),
            None,
            None,
        )
        .expect("selection");
        let occurred_at = crate::support::now();
        let handoff = ChainHandoffCommand::new(
            scope,
            self.chain,
            self.chain,
            u64::try_from(revision).expect("revision"),
            u64::try_from(version).expect("version"),
            RunId::generate(),
            selection.transfer(),
            SchedulerAuthority::for_run(
                owner,
                self.chain,
                u64::try_from(generation).expect("generation"),
            ),
            Subject::engine(),
            None,
            occurred_at,
            AuditRecord::engine(
                tenant,
                AuditAction::RunContinued,
                ResourceType::Run,
                self.chain.to_string(),
                "continuation selected",
                None,
                occurred_at,
            ),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        self.world
            .store
            .continue_evaluated(&selection.into_command(handoff).expect("command"))
            .await
    }
}

fn boundary_node() -> NodeId {
    NodeId::parse("rollover").expect("boundary node")
}

#[tokio::test]
async fn a_boundary_with_open_work_waits_without_failing_and_hands_off_once_it_settles() {
    let boundary = Boundary::reach("wait").await;
    let scheduler = scheduler(&boundary.world, "wait").await;
    let tenant = boundary.world.admin.tenant_id();
    let namespace = boundary.world.namespace.id();
    let before = boundary.chain_row().await;
    let journal_before = boundary.journal_len().await;

    for blocker in [
        QuiescenceBlocker::SiblingWork,
        QuiescenceBlocker::Lease,
        QuiescenceBlocker::Timer,
        QuiescenceBlocker::Subscription,
    ] {
        boundary.obstruct(blocker).await;
        let direct = scheduler
            .advance_continuation(tenant, namespace, boundary.chain, &boundary_node())
            .await;
        assert!(
            matches!(&direct, Err(ContinuationAdvanceError::Waiting(waiting)) if *waiting == blocker),
            "{blocker:?}: {direct:?}"
        );
        let report = scheduler.tick().await;
        assert_eq!(report.continuations_advanced, 0, "{blocker:?}: {report:?}");
        assert_eq!(report.failed, 0, "{blocker:?}: {report:?}");
        assert_eq!(
            boundary.chain_row().await,
            before,
            "{blocker:?}: a waiting boundary changes no chain row"
        );
        assert_eq!(boundary.journal_len().await, journal_before, "{blocker:?}");
        assert_eq!(
            boundary.world.run_status(boundary.chain).await,
            RunStatus::Running
        );
    }

    boundary.settle().await;
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    let after = boundary.chain_row().await;
    assert_ne!(after.0, boundary.chain.as_uuid());
    assert_eq!(after.1, 2, "exactly one successor");
    assert_eq!(after.3, None, "the chain is still open, not failed");

    let again = scheduler.tick().await;
    assert_eq!(again.continuations_advanced, 0, "{again:?}");
    assert_eq!(boundary.chain_row().await, after, "one hand-off, not two");
}

#[tokio::test]
async fn the_original_deadline_ends_a_waiting_boundary_as_timed_out_with_no_successor() {
    let boundary = Boundary::reach("deadline").await;
    let scheduler = scheduler(&boundary.world, "deadline").await;
    boundary.obstruct(QuiescenceBlocker::Timer).await;
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 0, "{report:?}");

    boundary.world.age_run_deadline(boundary.chain).await;
    let report = scheduler.tick().await;
    assert_eq!(report.deadlines_enforced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(
        boundary.world.run_status(boundary.chain).await,
        RunStatus::TimedOut
    );
    let (head, segments, _, outcome) = boundary.chain_row().await;
    assert_eq!(head, boundary.chain.as_uuid(), "no successor was started");
    assert_eq!(segments, 1);
    assert_eq!(outcome.as_deref(), Some("TIMED_OUT"));

    // Even if the obligation settles afterwards, a finished chain never continues.
    boundary.settle().await;
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 0, "{report:?}");
    assert_eq!(boundary.chain_row().await.1, 1);
}

#[tokio::test]
async fn waiting_for_quiescence_never_records_a_durable_failure() {
    let boundary = Boundary::reach("no-failure").await;
    let scheduler = scheduler(&boundary.world, "no-failure").await;
    boundary.obstruct(QuiescenceBlocker::SiblingWork).await;
    for _ in 0..3 {
        scheduler.tick().await;
    }
    let recorded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
           AND (detail::text LIKE '%NOT_QUIESCENT%' OR event_type IN ('run.failed','run.stopped'))",
    )
    .bind(boundary.world.admin.tenant_id().as_uuid())
    .bind(boundary.chain.as_uuid())
    .fetch_one(&boundary.world.db.pool)
    .await
    .expect("failure events");
    assert_eq!(recorded, 0);
    let stop: Option<String> =
        sqlx::query_scalar("SELECT stop_code FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(boundary.world.admin.tenant_id().as_uuid())
            .bind(boundary.chain.as_uuid())
            .fetch_one(&boundary.world.db.pool)
            .await
            .expect("stop code");
    assert_eq!(stop, None);
}

#[tokio::test]
async fn the_write_proves_quiescence_again_under_the_locks_and_names_the_blocker() {
    let boundary = Boundary::reach("lock-proof").await;
    let before = boundary.chain_row().await;
    let journal_before = boundary.journal_len().await;
    for blocker in [
        QuiescenceBlocker::SiblingWork,
        QuiescenceBlocker::Lease,
        QuiescenceBlocker::Timer,
        QuiescenceBlocker::Subscription,
    ] {
        boundary.obstruct(blocker).await;
        assert_eq!(
            boundary.write_directly().await,
            Err(ChainProgressError::Chain(ChainError::NotQuiescent(blocker))),
            "{blocker:?}"
        );
        assert_eq!(
            boundary.chain_row().await,
            before,
            "{blocker:?}: no partial write"
        );
        assert_eq!(boundary.journal_len().await, journal_before, "{blocker:?}");
    }
    boundary.settle().await;
    let written = boundary.write_directly().await.expect("quiescent hand-off");
    assert!(!written.replayed());
    assert_eq!(written.successor_index(), 1);
}

#[tokio::test]
async fn a_blocked_boundary_does_not_hold_back_the_boundaries_behind_it() {
    let blocked = Boundary::reach("head-of-line").await;
    blocked.obstruct(QuiescenceBlocker::Timer).await;
    // A second chain of the same world, ready to continue.
    let name = format!("chain-ready-{}", Uuid::now_v7());
    let (workflow, _) = blocked.world.publish(continuation_document(&name, 3)).await;
    let ready = blocked
        .world
        .start(workflow, &json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    let session = blocked.world.session(&["segment.work"]).await;
    let tasks = blocked.world.poll(&session, 4).await;
    let task = tasks
        .iter()
        .find(|task| task.run_id == ready)
        .expect("the ready chain's activity");
    blocked
        .world
        .complete(&session, task, &json!({"processed": true}))
        .await
        .expect("complete the ready chain's activity");

    // One candidate per tick is enough to starve the second chain if the first were returned.
    let scheduler = Scheduler::new(
        blocked.world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            batch: 1,
            ..SchedulerSettings::default()
        },
    )
    .with_children(blocked.world.cache.clone());
    let placement = Placement::new(format!("chains-head-of-line-{}", Uuid::now_v7()));
    Keeper::new(
        blocked.world.store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire shards");
    let scheduler = scheduler.with_placement(placement);
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    let advanced: i32 = sqlx::query_scalar(
        "SELECT segment_count FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(blocked.world.admin.tenant_id().as_uuid())
    .bind(ready.as_uuid())
    .fetch_one(&blocked.world.db.pool)
    .await
    .expect("ready chain");
    assert_eq!(advanced, 2);
    assert_eq!(
        blocked.chain_row().await.1,
        1,
        "the blocked chain stayed put"
    );
}
