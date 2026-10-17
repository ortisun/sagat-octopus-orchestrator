//! A continuation whose entrypoint is a `waitEvent` (G372): the successor is created already
//! waiting, in the hand-off transaction. The correlation key, the wait deadline and the event
//! subscription (the step row the dispatcher and the chain signal look at) commit with the
//! hand-off, so an event or signal that arrives right after it finds the wait, and nothing that
//! arrived before it is held back for it (the documented non-buffered rule).
#![allow(clippy::expect_used)]

use sagat_application::chain_progress::ChainCommandService;
use sagat_application::clock::SystemClock;
use sagat_application::events::PublishRequest;
use sagat_application::execution::Engine;
use sagat_domain::events::EventId;
use sagat_domain::execution::{CorrelationKey, RunId, RunStatus, StepStatus};
use sagat_domain::workflow::EventType;
use sagat_infra_ir::JsonEvaluator;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::chains::scheduler;
use crate::support::engine::{Sched, World, ctx, world};

const WAIT_MS: u64 = 600_000;

/// `release` (wait) -> `rollover`: the shape of `chain-root-wait`. Every segment waits again.
fn rolling(name: &str, max_segments: u16) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "release",
        "nodes": [
            {
                "id": "release", "type": "waitEvent", "eventType": "chain.release",
                "correlationExpr": "input.key", "timeoutMs": WAIT_MS, "next": "rollover"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
        ],
        "chainPolicy": {"maxSegments": max_segments, "timeoutMs": 3_600_000}
    })
}

/// `release` (wait) -> `again` -> `rollover` or `done`: the second segment ends the chain.
fn finishing(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "release",
        "nodes": [
            {
                "id": "release", "type": "waitEvent", "eventType": "chain.release",
                "correlationExpr": "input.key", "timeoutMs": WAIT_MS, "next": "again"
            },
            {
                "id": "again", "type": "condition", "expression": "input.continue",
                "ifTrue": "rollover", "ifFalse": "done"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    })
}

pub(crate) struct Waiting {
    pub(crate) world: World,
    pub(crate) scheduler: Sched,
    pub(crate) chain: RunId,
}

impl Waiting {
    pub(crate) async fn start(label: &str, document: Value, input: &Value) -> Self {
        let world = world().await;
        let scheduler = scheduler(&world, label).await;
        let (workflow, _) = world.publish(document).await;
        let chain = world.start(workflow, input).await;
        Self {
            world,
            scheduler,
            chain,
        }
    }

    pub(crate) async fn rolling(label: &str, max_segments: u16, input: &Value) -> Self {
        let name = format!("chain-wait-{label}-{}", Uuid::now_v7());
        Self::start(label, rolling(&name, max_segments), input).await
    }

    pub(crate) async fn finishing(label: &str, input: &Value) -> Self {
        let name = format!("chain-wait-{label}-{}", Uuid::now_v7());
        Self::start(label, finishing(&name), input).await
    }

    /// The chain's current head and how many segments it has.
    pub(crate) async fn head(&self) -> (RunId, i32, Option<String>) {
        let (head, segments, outcome): (Uuid, i32, Option<String>) = sqlx::query_as(
            "SELECT current_run_id,segment_count,final_outcome FROM execution_chains \
             WHERE tenant_id=$1 AND chain_id=$2",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(self.chain.as_uuid())
        .fetch_one(&self.world.db.pool)
        .await
        .expect("chain row");
        (RunId::new(head), segments, outcome)
    }

    pub(crate) async fn publish(&self, id: &str, key: &str) -> bool {
        self.world
            .events
            .publish(
                &self.world.admin,
                PublishRequest {
                    namespace: "payments".to_owned(),
                    event_id: Some(EventId::parse(id).expect("id")),
                    event_type: EventType::parse("chain.release").expect("type"),
                    subject: None,
                    correlation_key: Some(CorrelationKey::parse(key).expect("key")),
                    occurred_at: None,
                    data: serde_json::to_vec(&json!({"released": true})).expect("json"),
                    dedupe_ttl: None,
                },
                &ctx(),
            )
            .await
            .expect("published")
            .duplicate
    }

    /// Wakes the head's wait through the event inbox: one dispatch round.
    pub(crate) async fn deliver(&self) -> u32 {
        let round = self.world.dispatcher.tick().await;
        assert_eq!(round.dead + round.retried, 0, "{round:?}");
        round.steps_woken
    }

    /// The hand-off pass: one scheduler tick, which must not fail.
    pub(crate) async fn hand_off(&self) -> u32 {
        let report = self.scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        report.continuations_advanced
    }

    pub(crate) async fn count(&self, run: RunId, kind: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type=$3",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .bind(kind)
        .fetch_one(&self.world.db.pool)
        .await
        .expect("event count")
    }

    /// `(waiting_kind, event_type, correlation_key, due_at in the future, event_since is set)`.
    pub(crate) async fn subscription(
        &self,
        run: RunId,
    ) -> Option<(Option<String>, Option<String>, Option<String>, bool, bool)> {
        sqlx::query_as(
            "SELECT waiting_kind,event_type,correlation_key,due_at>now(),event_since IS NOT NULL \
             FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='release' \
               AND cardinality(instance_path_nodes)=0",
        )
        .bind(self.world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&self.world.db.pool)
        .await
        .expect("subscription row")
    }

    pub(crate) fn commands(
        &self,
    ) -> ChainCommandService<sagat_infra_postgres::PgRunStore, SystemClock, JsonEvaluator> {
        ChainCommandService::new(
            self.world.store.clone(),
            SystemClock,
            Engine::new(JsonEvaluator),
        )
    }

    pub(crate) async fn signal(
        &self,
        expected: Option<RunId>,
        key: &str,
    ) -> Result<usize, sagat_application::error::AppError> {
        self.commands()
            .signal(
                &self.world.admin,
                self.world.namespace.id(),
                self.chain,
                expected,
                "chain.release",
                br#"{"released":true}"#,
                key,
                &ctx(),
            )
            .await
            .map(|woken| woken.len())
    }
}

#[tokio::test]
async fn a_continuation_that_starts_with_a_wait_hands_off_to_a_successor_that_already_waits() {
    let chain = Waiting::rolling(
        "handoff",
        3,
        &json!({"key": "order-1", "next": {"key": "order-2", "next": {"key": "order-3"}}}),
    )
    .await;
    assert_eq!(
        chain.world.step_status(chain.chain, "release").await,
        Some(StepStatus::Waiting)
    );
    chain.publish("wait-handoff-1", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.hand_off().await, 1, "the segment is handed off");

    let (head, segments, outcome) = chain.head().await;
    assert_ne!(head, chain.chain);
    assert_eq!((segments, outcome), (2, None));
    assert_eq!(
        chain.world.step_status(head, "release").await,
        Some(StepStatus::Waiting),
        "the successor waits at once, not after a later scheduler pass"
    );
    assert_eq!(
        chain.subscription(head).await,
        Some((
            Some("EVENT".to_owned()),
            Some("chain.release".to_owned()),
            Some("order-2".to_owned()),
            true,
            true
        )),
        "the subscription was written by the hand-off, with the key of the successor's input"
    );
    assert_eq!(chain.world.run_status(head).await, RunStatus::Waiting);
    assert_eq!(
        chain.world.journal(head).await,
        [
            "run.started",
            "execution_chain.successor_linked",
            "wait.started"
        ]
    );
    let hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 \
           AND event_type='task.ready'",
    )
    .bind(chain.world.admin.tenant_id().as_uuid())
    .bind(head.as_uuid())
    .fetch_one(&chain.world.db.pool)
    .await
    .expect("task hints");
    assert_eq!(
        hints, 0,
        "a wait is not a task: no activity hint is written"
    );
    let deadlines: (bool,) = sqlx::query_as(
        "SELECT run.deadline_at=chain.original_deadline_at \
         FROM runs run JOIN execution_chains chain ON chain.tenant_id=run.tenant_id \
          AND chain.chain_id=$3 WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(chain.world.admin.tenant_id().as_uuid())
    .bind(head.as_uuid())
    .bind(chain.chain.as_uuid())
    .fetch_one(&chain.world.db.pool)
    .await
    .expect("deadlines");
    assert!(
        deadlines.0,
        "the successor keeps the chain's original deadline"
    );
    chain.world.assert_consistent(chain.chain).await;
    chain.world.assert_consistent(head).await;

    // The event the successor waits for, delivered once, continues the chain again.
    chain.publish("wait-handoff-2", "order-2").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.hand_off().await, 1);
    let (third, segments, _) = chain.head().await;
    assert_ne!(third, head);
    assert_eq!(segments, 3);
    assert_eq!(
        chain.subscription(third).await.expect("row").2.as_deref(),
        Some("order-3")
    );
    chain.world.assert_consistent(third).await;
}

#[tokio::test]
async fn a_chain_that_waits_in_every_segment_reaches_its_final_outcome() {
    let chain = Waiting::finishing(
        "final",
        &json!({"key": "k-1", "continue": true, "next": {"key": "k-2", "continue": false}}),
    )
    .await;
    chain.publish("wait-final-1", "k-1").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.hand_off().await, 1);
    let (second, segments, outcome) = chain.head().await;
    assert_eq!((segments, outcome), (2, None));
    assert_eq!(
        chain.world.step_status(second, "release").await,
        Some(StepStatus::Waiting)
    );

    // The second segment is woken by a chain signal, not an event: both paths see the wait.
    chain
        .signal(Some(second), "wait-final-signal")
        .await
        .expect("signal the successor's wait");
    let (head, segments, outcome) = chain.head().await;
    assert_eq!(head, second, "it ends instead of continuing");
    assert_eq!(segments, 2);
    assert_eq!(outcome.as_deref(), Some("SUCCEEDED"));
    assert_eq!(chain.world.run_status(second).await, RunStatus::Succeeded);
    assert_eq!(
        chain.hand_off().await,
        0,
        "a finished chain never continues"
    );
    chain.world.assert_consistent(second).await;
}

#[tokio::test]
async fn an_event_admitted_before_the_hand_off_does_not_wake_the_successor_and_one_after_does() {
    let chain = Waiting::rolling(
        "early-event",
        3,
        &json!({"key": "order-1", "next": {"key": "order-1"}}),
    )
    .await;
    chain.publish("early-first", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    // The boundary is pending and the head holds no wait. An event for the key the successor will
    // wait for is admitted now, and is still undispatched when the hand-off commits.
    chain.publish("early-second", "order-1").await;
    assert_eq!(chain.hand_off().await, 1);
    let (head, _, _) = chain.head().await;
    assert_eq!(
        chain.deliver().await,
        0,
        "it predates the successor's wait and the wait does not look back"
    );
    assert_eq!(
        chain.world.step_status(head, "release").await,
        Some(StepStatus::Waiting)
    );
    assert_eq!(chain.count(head, "wait.satisfied").await, 0);

    // One admitted after the hand-off finds the subscription.
    chain.publish("early-third", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.count(head, "wait.satisfied").await, 1);
    chain.world.assert_consistent(head).await;
}

#[tokio::test]
async fn a_duplicate_delivery_wakes_the_successor_once() {
    let chain = Waiting::rolling(
        "duplicate",
        3,
        &json!({"key": "order-1", "next": {"key": "order-2", "next": {"key": "order-3"}}}),
    )
    .await;
    chain.publish("dup-first", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.hand_off().await, 1);
    let (head, _, _) = chain.head().await;

    assert!(!chain.publish("dup-second", "order-2").await);
    assert!(
        chain.publish("dup-second", "order-2").await,
        "the second admission of one id is a duplicate and writes nothing"
    );
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(
        chain.deliver().await,
        0,
        "the inbox has nothing left to deliver"
    );
    assert_eq!(chain.count(head, "wait.satisfied").await, 1);

    // A signal replayed under its key delivers once as well.
    assert_eq!(chain.hand_off().await, 1);
    let (third, _, _) = chain.head().await;
    assert_eq!(
        chain
            .signal(Some(third), "dup-signal")
            .await
            .expect("first"),
        1
    );
    assert_eq!(
        chain
            .signal(Some(third), "dup-signal")
            .await
            .expect("replay"),
        1,
        "the replay answers the original delivery"
    );
    assert_eq!(chain.count(third, "wait.satisfied").await, 1);
    chain.world.assert_consistent(third).await;
}

#[tokio::test]
async fn the_original_deadline_ends_a_waiting_successor_and_its_chain_as_timed_out() {
    let chain = Waiting::rolling(
        "deadline",
        3,
        &json!({"key": "order-1", "next": {"key": "order-2"}}),
    )
    .await;
    chain.publish("deadline-first", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.hand_off().await, 1);
    let (head, segments, _) = chain.head().await;
    assert_eq!(segments, 2);
    assert_eq!(chain.world.run_status(head).await, RunStatus::Waiting);

    chain.world.age_run_deadline(head).await;
    let report = chain.scheduler.tick().await;
    assert_eq!(report.deadlines_enforced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(chain.world.run_status(head).await, RunStatus::TimedOut);
    let (still, segments, outcome) = chain.head().await;
    assert_eq!((still, segments), (head, 2));
    assert_eq!(outcome.as_deref(), Some("TIMED_OUT"));
    // The subscription is gone with the wait: an event now wakes nothing.
    chain.publish("deadline-late", "order-2").await;
    assert_eq!(chain.deliver().await, 0);
    chain.world.assert_consistent(head).await;
}

#[tokio::test]
async fn the_wait_of_the_successor_times_out_by_its_own_timeout_like_any_other_wait() {
    let chain = Waiting::rolling(
        "own-timeout",
        3,
        &json!({"key": "order-1", "next": {"key": "order-2"}}),
    )
    .await;
    chain.publish("timeout-first", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    assert_eq!(chain.hand_off().await, 1);
    let (head, _, _) = chain.head().await;

    chain.world.age_due(head, "release").await;
    let report = chain.scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(chain.world.run_status(head).await, RunStatus::Failed);
    assert_eq!(chain.count(head, "wait.timed_out").await, 1);
    assert_eq!(chain.head().await.2.as_deref(), Some("FAILED"));
    chain.world.assert_consistent(head).await;
}

#[tokio::test]
async fn a_successor_whose_correlation_cannot_be_computed_ends_the_chain_with_a_typed_failure() {
    // The next input names no key, so the successor's wait could never be matched.
    let chain = Waiting::rolling(
        "no-key",
        3,
        &json!({"key": "order-1", "next": {"other": 1}}),
    )
    .await;
    chain.publish("no-key-first", "order-1").await;
    assert_eq!(chain.deliver().await, 1);
    let report = chain.scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    let (head, segments, outcome) = chain.head().await;
    assert_eq!(
        (head, segments),
        (chain.chain, 1),
        "no successor is created"
    );
    assert_eq!(outcome.as_deref(), Some("FAILED"));
    assert_eq!(chain.world.run_status(chain.chain).await, RunStatus::Failed);
    let stop: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
           AND event_type='run.stop_requested'",
    )
    .bind(chain.world.admin.tenant_id().as_uuid())
    .bind(chain.chain.as_uuid())
    .fetch_one(&chain.world.db.pool)
    .await
    .expect("the stop that ended the run");
    assert_eq!(stop["code"], "EXPRESSION_ERROR", "{stop}");
    let successors: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(chain.world.admin.tenant_id().as_uuid())
    .bind(chain.chain.as_uuid())
    .fetch_one(&chain.world.db.pool)
    .await
    .expect("segments");
    assert_eq!(successors, 1);
}

// ------------------------------------------------------------------ the store proves the entry

use sagat_application::chain_progress::{
    ChainHandoffCommand, ChainHandoffWrite, ChainProgressError, ChainProgressStore,
    ContinuationSelection,
};
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::ChainScope;
use sagat_domain::identity::Subject;
use sagat_domain::workflow::NodeId;

/// The write the scheduler would issue for the chain's pending boundary, with a selection the
/// caller chose, sent straight to the store (no scheduler pre-check in front of it).
async fn write_directly(
    world: &World,
    chain: RunId,
    selection: ContinuationSelection,
) -> Result<ChainHandoffWrite, ChainProgressError> {
    let tenant = world.admin.tenant_id();
    let scope = ChainScope::new(tenant, world.namespace.id());
    let (revision, version): (i64, i64) = sqlx::query_as(
        "SELECT chain.revision,run.run_version FROM execution_chains chain \
           JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("chain revisions");
    let owner = "chain-wait-direct-write";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET generation=generation+1,owner=$2, \
                expires_at=clock_timestamp()+interval '1 hour',acquired_at=clock_timestamp() \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(chain.as_uuid())
    .bind(owner)
    .fetch_one(&world.db.pool)
    .await
    .expect("scheduler fence");
    let occurred_at = crate::support::now();
    let handoff = ChainHandoffCommand::new(
        scope,
        chain,
        chain,
        u64::try_from(revision).expect("revision"),
        u64::try_from(version).expect("version"),
        RunId::generate(),
        selection.transfer(),
        SchedulerAuthority::for_run(owner, chain, u64::try_from(generation).expect("generation")),
        Subject::engine(),
        None,
        occurred_at,
        AuditRecord::engine(
            tenant,
            AuditAction::RunContinued,
            ResourceType::Run,
            chain.to_string(),
            "continuation selected",
            None,
            occurred_at,
        ),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    world
        .store
        .continue_evaluated(&selection.into_command(handoff).expect("command"))
        .await
}

fn selection(input: &Value) -> ContinuationSelection {
    ContinuationSelection::new(
        NodeId::parse("rollover").expect("boundary"),
        serde_json::to_vec(input).expect("input"),
        None,
        None,
    )
    .expect("selection")
}

async fn segments(world: &World, chain: RunId) -> i32 {
    sqlx::query_scalar(
        "SELECT segment_count FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("segments")
}

#[tokio::test]
async fn the_store_refuses_a_selection_that_does_not_match_the_entry_the_definition_declares() {
    // A wait entry needs its key: without it the successor would wait for nothing.
    let waiting = Waiting::rolling(
        "refuse-wait",
        3,
        &json!({"key": "order-1", "next": {"key": "order-2"}}),
    )
    .await;
    waiting.publish("refuse-first", "order-1").await;
    assert_eq!(waiting.deliver().await, 1);
    let missing_key = write_directly(
        &waiting.world,
        waiting.chain,
        selection(&json!({"key": "order-2"})),
    )
    .await;
    assert_eq!(missing_key, Err(ChainProgressError::RuntimeNotIntegrated));
    assert_eq!(segments(&waiting.world, waiting.chain).await, 1);

    // A key for an activity entry has no wait to carry it.
    let world = world().await;
    let name = format!("chain-wait-refuse-activity-{}", Uuid::now_v7());
    let (workflow, _) = world
        .publish(json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "work",
            "nodes": [
                {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
                {"id": "rollover", "type": "continueAsNew", "input": "input.next"}
            ],
            "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
        }))
        .await;
    let chain = world
        .start(workflow, &json!({"next": {"key": "order-2"}}))
        .await;
    let session = world.session(&["segment.work"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"done": true}))
        .await
        .expect("complete the activity");
    let stray_key = write_directly(
        &world,
        chain,
        selection(&json!({"key": "order-2"}))
            .with_correlation_key(CorrelationKey::parse("order-2").expect("key")),
    )
    .await;
    assert_eq!(stray_key, Err(ChainProgressError::RuntimeNotIntegrated));
    assert_eq!(segments(&world, chain).await, 1);
    assert!(
        write_directly(&world, chain, selection(&json!({"key": "order-2"})))
            .await
            .is_ok(),
        "the matching selection still commits"
    );
}
