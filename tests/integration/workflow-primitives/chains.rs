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

// ------------------------------------------------------------------------------------ children

use sagat_application::execution::child_run_id;
use sagat_application::runs::RunOperation;
use sagat_domain::execution::StepStatus;

use crate::support::engine::{activity, child_node, document};

/// A continuation workflow that keeps going while `input.continue` is true and then ends.
fn rolling(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "again"},
            {"id": "again", "type": "condition", "expression": "input.continue",
             "ifTrue": "rollover", "ifFalse": "done"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 5, "timeoutMs": 3_600_000}
    })
}

struct Family {
    world: World,
    scheduler: Sched,
    parent: RunId,
    /// The child chain's first run, which is also its chain id.
    chain: RunId,
}

impl Family {
    /// The parent waits at `kid`, a child node naming the rolling workflow; `kid_input` is the
    /// child's input once it exists.
    async fn start(label: &str) -> Self {
        let world = world().await;
        let kid = format!("kid-{label}-{}", Uuid::now_v7());
        world.publish(rolling(&kid)).await;
        let (parent_workflow, _) = world
            .publish(document(
                &format!("mother-{label}-{}", Uuid::now_v7()),
                "kid",
                vec![child_node("kid", &kid, Some("after")), activity("after")],
            ))
            .await;
        let parent = world
            .start(
                parent_workflow,
                &json!({"continue": true, "next": {"continue": true, "next": {"continue": false}}}),
            )
            .await;
        let chain = child_run_id(parent, &NodeId::parse("kid").expect("kid node"));
        let scheduler = scheduler(&world, label).await;
        Self {
            world,
            scheduler,
            parent,
            chain,
        }
    }

    async fn chain_row(&self) -> (Uuid, i32, i64, bool, Option<String>) {
        sqlx::query_as(
            "SELECT current_run_id,segment_count,revision,cancel_requested,final_outcome \
             FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.world.db.pool)
        .await
        .expect("child chain row")
    }

    async fn link(&self) -> (bool, Option<String>, bool) {
        sqlx::query_as(
            "SELECT cancel_requested,final_outcome,settled FROM child_chain_links \
             WHERE tenant_id=$1 AND parent_run_id=$2 AND child_chain_id=$3",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.parent.as_uuid())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.world.db.pool)
        .await
        .expect("child chain link")
    }
}

#[tokio::test]
async fn a_continuation_workflow_starts_as_a_child_with_a_durable_link() {
    let family = Family::start("link").await;
    let report = family.scheduler.tick().await;
    assert_eq!(report.children_started, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    let chain = family.chain_row().await;
    assert_eq!(chain.0, family.chain.as_uuid(), "the first run is the head");
    assert_eq!((chain.1, chain.2, chain.3, chain.4), (1, 0, false, None));
    assert_eq!(family.link().await, (false, None, false));
    let (parent_instance, step): (Uuid, Uuid) = sqlx::query_as(
        "SELECT link.parent_instance_id,step.step_id FROM child_chain_links link \
           JOIN steps step ON step.tenant_id=link.tenant_id AND step.run_id=link.parent_run_id \
            AND step.node='kid' \
         WHERE link.tenant_id=$1 AND link.child_chain_id=$2",
    )
    .bind(family.world.admin.tenant_id().as_uuid())
    .bind(family.chain.as_uuid())
    .fetch_one(&family.world.db.pool)
    .await
    .expect("link identity");
    assert_eq!(
        parent_instance, step,
        "the link names the parent's child step"
    );
    let record = family
        .world
        .run_record(family.chain)
        .await
        .expect("child run");
    assert_eq!(record.info.depth, 1);
    assert_eq!(record.info.parent.expect("parent link").run, family.parent);
    assert_eq!(
        family.world.step_status(family.parent, "kid").await,
        Some(StepStatus::Waiting)
    );
}

#[tokio::test]
async fn the_parent_hears_only_the_final_outcome_of_a_child_chain_and_hears_it_once() {
    let family = Family::start("final").await;
    family.scheduler.tick().await;
    let session = family.world.session(&["segment.work", "act.after"]).await;
    let mut heads = vec![family.chain];

    // Two hand-offs: three segments, the last one ends the chain.
    for _ in 0..2 {
        let task = family.world.one_task_of(&session, "segment.work").await;
        family
            .world
            .complete(&session, &task, &json!({"processed": true}))
            .await
            .expect("segment activity");
        let report = family.scheduler.tick().await;
        assert_eq!(report.continuations_advanced, 1, "{report:?}");
        assert_eq!(
            report.children_finished, 0,
            "a continued segment is not an ending: {report:?}"
        );
        assert_eq!(report.failed, 0, "{report:?}");
        assert_eq!(
            family.world.step_status(family.parent, "kid").await,
            Some(StepStatus::Waiting),
            "the parent still waits for the chain"
        );
        heads.push(RunId::new(family.chain_row().await.0));
    }
    let last = family.world.one_task_of(&session, "segment.work").await;
    assert_eq!(last.run_id, heads[2]);
    family
        .world
        .complete(&session, &last, &json!({"processed": true}))
        .await
        .expect("final segment activity");
    assert_eq!(family.chain_row().await.4.as_deref(), Some("SUCCEEDED"));
    assert_eq!(family.link().await.1.as_deref(), Some("SUCCEEDED"));

    let report = family.scheduler.tick().await;
    assert_eq!(report.children_finished, 1, "{report:?}");
    assert_eq!(
        family.world.step_status(family.parent, "kid").await,
        Some(StepStatus::Succeeded)
    );
    assert_eq!(family.link().await, (false, Some("SUCCEEDED".into()), true));
    let again = family.scheduler.tick().await;
    assert_eq!(again.children_finished, 0, "told once: {again:?}");
    assert_eq!(again.failed, 0, "{again:?}");

    // The continued predecessors never reached the parent: one child.finished in its journal.
    let finished = family
        .world
        .journal(family.parent)
        .await
        .into_iter()
        .filter(|event| event == "child.finished")
        .count();
    assert_eq!(finished, 1);
    let after = family.world.one_task_of(&session, "act.after").await;
    assert_eq!(after.run_id, family.parent);
    family.world.assert_consistent(family.parent).await;
    for head in heads {
        family.world.assert_consistent(head).await;
    }
}

#[tokio::test]
async fn cancelling_the_parent_latches_the_child_chain_and_cancels_its_current_head() {
    let family = Family::start("cancel").await;
    family.scheduler.tick().await;
    let session = family.world.session(&["segment.work"]).await;
    let task = family.world.one_task_of(&session, "segment.work").await;
    family
        .world
        .complete(&session, &task, &json!({"processed": true}))
        .await
        .expect("first segment");
    family.scheduler.tick().await;
    let head = RunId::new(family.chain_row().await.0);
    assert_ne!(head, family.chain, "the chain has a successor head");

    family
        .world
        .operate_run(family.parent, RunOperation::Cancel)
        .await;
    assert!(
        family.link().await.0,
        "the parent's cancellation latches the link"
    );
    let report = family.scheduler.tick().await;
    assert_eq!(report.children_cancelled, 1, "{report:?}");
    assert_eq!(family.world.run_status(head).await, RunStatus::Cancelled);
    let chain = family.chain_row().await;
    assert_eq!(chain.4.as_deref(), Some("CANCELLED"));
    assert_eq!(
        chain.0,
        head.as_uuid(),
        "no successor was started after the latch"
    );
    assert_eq!(
        family.world.run_status(family.parent).await,
        RunStatus::Cancelled
    );

    let quiet = family.scheduler.tick().await;
    assert_eq!((quiet.failed, quiet.children_finished), (0, 0), "{quiet:?}");
    assert_eq!(family.chain_row().await, chain);
}

#[tokio::test]
async fn a_latched_child_chain_never_hands_off_even_before_its_head_is_cancelled() {
    let family = Family::start("latch").await;
    family.scheduler.tick().await;
    let session = family.world.session(&["segment.work"]).await;
    let task = family.world.one_task_of(&session, "segment.work").await;
    family
        .world
        .complete(&session, &task, &json!({"processed": true}))
        .await
        .expect("first segment");
    // The latch commits before the boundary is handed off: the scheduler must not pass it.
    sqlx::query(
        "UPDATE child_chain_links SET cancel_requested=true,updated_at=now() \
         WHERE tenant_id=$1 AND child_chain_id=$2",
    )
    .bind(family.world.admin.tenant_id().as_uuid())
    .bind(family.chain.as_uuid())
    .execute(&family.world.db.pool)
    .await
    .expect("latch");
    let report = family.scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 0, "{report:?}");
    assert_eq!(family.chain_row().await.1, 1, "no successor");
}

impl Family {
    /// Brings the tree of the parent to `members` runs with finished, settled filler children.
    async fn fill_tree_to(&self, members: i64) {
        let present: i64 =
            sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND root_run_id=$2")
                .bind(self.world.admin.tenant_id().as_uuid())
                .bind(self.parent.as_uuid())
                .fetch_one(&self.world.db.pool)
                .await
                .expect("tree size");
        assert!(present <= members, "the tree already has {present} runs");
        sqlx::query(
            "INSERT INTO runs (id,tenant_id,namespace_id,workflow_id,workflow_version_id,status, \
                               run_version,input_digest,started_by,created_at,updated_at,finished_at, \
                               parent_run_id,root_run_id,parent_node,cancel_policy,depth,parent_settled) \
             SELECT gen_random_uuid(),tenant_id,namespace_id,workflow_id,workflow_version_id, \
                    'SUCCEEDED',0,input_digest,started_by,now(),now(),now(), \
                    parent_run_id,root_run_id,parent_node,cancel_policy,depth,true \
             FROM runs, generate_series(1,$3::int) \
             WHERE tenant_id=$1 AND id=$2",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .bind(i32::try_from(members - present).expect("filler count"))
        .execute(&self.world.db.pool)
        .await
        .expect("filler runs");
    }

    /// Runs the first segment's activity so the boundary is pending.
    async fn finish_first_segment(&self) {
        let session = self.world.session(&["segment.work"]).await;
        let task = self.world.one_task_of(&session, "segment.work").await;
        self.world
            .complete(&session, &task, &json!({"processed": true}))
            .await
            .expect("first segment");
    }
}

#[tokio::test]
async fn the_last_run_the_tree_budget_allows_is_a_successor_and_the_next_one_is_refused() {
    use sagat_application::execution::MAX_TREE_RUNS;
    let members = i64::try_from(MAX_TREE_RUNS).expect("budget");

    // One run short of the budget: the successor fills it exactly.
    let fits = Family::start("tree-fits").await;
    fits.scheduler.tick().await;
    fits.fill_tree_to(members - 1).await;
    fits.finish_first_segment().await;
    let report = fits.scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(fits.chain_row().await.1, 2);
    let size: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND root_run_id=$2")
            .bind(fits.world.admin.tenant_id().as_uuid())
            .bind(fits.parent.as_uuid())
            .fetch_one(&fits.world.db.pool)
            .await
            .expect("tree size");
    assert_eq!(size, members, "the successor is a run of the parent's tree");

    // The tree is full: the boundary ends the chain with CHAIN_LIMIT and writes no successor.
    let full = Family::start("tree-full").await;
    full.scheduler.tick().await;
    full.fill_tree_to(members).await;
    full.finish_first_segment().await;
    // Finalizing the chain is the boundary's action, as it is at the segment limit.
    let report = full.scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    let chain = full.chain_row().await;
    assert_eq!(chain.1, 1, "no successor");
    assert_eq!(chain.4.as_deref(), Some("FAILED"));
    let (status, code): (String, Option<String>) =
        sqlx::query_as("SELECT status,stop_code FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(full.world.admin.tenant_id().as_uuid())
            .bind(full.chain.as_uuid())
            .fetch_one(&full.world.db.pool)
            .await
            .expect("head");
    assert_eq!(
        (status.as_str(), code.as_deref()),
        ("FAILED", Some("CHAIN_LIMIT"))
    );
    let size: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND root_run_id=$2")
            .bind(full.world.admin.tenant_id().as_uuid())
            .bind(full.parent.as_uuid())
            .fetch_one(&full.world.db.pool)
            .await
            .expect("tree size");
    assert_eq!(size, members, "the refused successor left no run behind");

    // The parent heard the chain's failure once, as a child failure.
    assert_eq!(report.children_finished, 1, "{report:?}");
    assert_eq!(full.scheduler.tick().await.children_finished, 0);
    assert_eq!(
        full.world.step_status(full.parent, "kid").await,
        Some(StepStatus::Failed)
    );
}

#[tokio::test]
async fn finished_chains_links_and_payloads_survive_every_housekeeping_pass() {
    // D298: retention of finished runs is a pending decision, so the counts the tree budget and the
    // segment limit read (runs of a tree, segments of a chain) are never reduced by anything the
    // platform runs on a schedule, and a future retention must keep chain, link and payload rows.
    let family = Family::start("retention").await;
    family.scheduler.tick().await;
    let session = family.world.session(&["segment.work", "act.after"]).await;
    for _ in 0..3 {
        let task = family.world.one_task_of(&session, "segment.work").await;
        family
            .world
            .complete(&session, &task, &json!({"processed": true}))
            .await
            .expect("segment");
        family.scheduler.tick().await;
    }
    assert_eq!(family.chain_row().await.4.as_deref(), Some("SUCCEEDED"));
    assert_eq!(family.link().await.2, true);

    let ticking = crate::limits::scheduler_with(
        &family.world,
        "chain-retention",
        sagat_scheduler::SchedulerSettings {
            idempotency_retention: StdDuration::ZERO,
            outbox_retention: StdDuration::ZERO,
            ..sagat_scheduler::SchedulerSettings::default()
        },
    )
    .await;
    let tables = [
        "runs",
        "run_events",
        "steps",
        "execution_chains",
        "chain_segments",
        "child_chain_links",
        "payloads",
        "workflow_operation_states",
    ];
    let census = || async {
        let mut counts = Vec::new();
        for table in tables {
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table} WHERE tenant_id = $1"
            )))
            .bind(family.world.admin.tenant_id().as_uuid())
            .fetch_one(&family.world.db.pool)
            .await
            .expect("table census");
            counts.push((table, count));
        }
        counts
    };
    let before = census().await;
    assert!(
        before
            .iter()
            .filter(|(table, _)| [
                "runs",
                "execution_chains",
                "chain_segments",
                "child_chain_links"
            ]
            .contains(table))
            .all(|(_, count)| *count >= 1),
        "{before:?}"
    );
    ticking.housekeeping().await;
    family
        .world
        .store
        .purge_projected(StdDuration::ZERO, 10_000)
        .await
        .expect("purge projected telemetry notes");
    assert_eq!(
        census().await,
        before,
        "maintenance must not delete chain data"
    );

    // The runtime role cannot delete any of it either, so no code path of the platform can.
    let role_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sagat_app')")
            .fetch_one(&family.world.db.pool)
            .await
            .expect("role lookup");
    if role_exists {
        for table in [
            "runs",
            "execution_chains",
            "chain_segments",
            "child_chain_links",
            "payloads",
        ] {
            let may_delete: bool =
                sqlx::query_scalar("SELECT has_table_privilege('sagat_app', $1, 'DELETE')")
                    .bind(table)
                    .fetch_one(&family.world.db.pool)
                    .await
                    .expect("privilege lookup");
            assert!(!may_delete, "the runtime role may delete from {table}");
        }
    }
}
