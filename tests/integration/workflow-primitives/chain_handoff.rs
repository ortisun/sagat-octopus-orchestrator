//! PostgreSQL acceptance boundary for planner-owned continuation handoff.
#![allow(clippy::expect_used)]

use std::{sync::Arc, time::Duration as StdDuration};

use sagat_application::chain_progress::{
    ChainHandoffCommand, ChainInitialization, ChainProgressError, ChainProgressStore,
    EvaluatedChainHandoffCommand,
};
use sagat_application::declared_operations::{
    DeclaredOperationEvaluator, DeclaredOperationStore, DeclaredQuery,
};
use sagat_application::definitions::{IrCodec, IrError, ParsedDefinition};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine, instance_step_id};
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::primitives::SchedulerAuthority;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ChainPolicy, ChainScope, ContinuationTransfer, ExecutionInstanceId, OperationAddress,
    OperationScope, RunId,
};
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_infra_ir::{JsonDeclaredOperationEvaluator, JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{
    PgChainProgressStore, PgDeclaredOperationStore, PgRunStore, initialize_chain_in_transaction,
};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use time::Duration;
use uuid::Uuid;

use crate::support::engine::{activity, document, world};

#[derive(Debug)]
struct ContinuationCodec;

impl IrCodec for ContinuationCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        IrCodec::check_draft(&JsonIrCodec, document)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        let parsed = JsonIrCodec.parse_continuation(document)?;
        Ok(ParsedDefinition {
            definition: parsed.definition().clone(),
            canonical: parsed.canonical().to_vec(),
            digest: parsed.digest(),
            warnings: parsed.warnings().to_vec(),
        })
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        IrCodec::diff(&JsonIrCodec, from, to)
    }
}

#[tokio::test]
async fn public_activity_completion_is_handed_to_a_successor_by_the_scheduler() {
    let world = world().await;
    let name = format!("public-chain-handoff-{}", Uuid::now_v7());
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "segment.work",
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            }
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    let (workflow, _) = world.publish(document).await;
    let chain = world
        .start(workflow, &json!({"segment": 1, "next": {"segment": 2}}))
        .await;
    let session = world.session(&["segment.work"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"processed": true}))
        .await
        .expect("complete activity");

    let placement = Placement::new(format!("public-chain-scheduler-{name}"));
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
    let scheduler = Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    let projection: (Uuid, i32) = sqlx::query_as(
        "SELECT current_run_id,segment_count FROM execution_chains \
         WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("chain projection");
    assert_ne!(projection.0, chain.as_uuid());
    assert_eq!(projection.1, 2);
    let predecessor: (String, Option<String>) = sqlx::query_as(
        "SELECT run.status,segment.completion_kind FROM runs run \
             JOIN chain_segments segment \
               ON segment.tenant_id=run.tenant_id AND segment.run_id=run.id \
             WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("predecessor");
    assert_eq!(predecessor.0, "SUCCEEDED");
    assert_eq!(predecessor.1.as_deref(), Some("CONTINUED"));
    let successor_input: Vec<u8> = sqlx::query_scalar(
        "SELECT payload.content FROM runs run JOIN payloads payload \
           ON payload.tenant_id=run.tenant_id AND payload.digest=run.input_digest \
         WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(projection.0)
    .fetch_one(&world.db.pool)
    .await
    .expect("successor input");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&successor_input)
            .expect("successor input JSON"),
        json!({"segment": 2})
    );
}

#[tokio::test]
async fn public_stateful_handoff_preserves_the_declared_snapshot() {
    let world = world().await;
    let name = format!("public-state-chain-{}", Uuid::now_v7());
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next", "state": "state"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000},
        "state": {
            "schema": {
                "type": "object", "additionalProperties": false,
                "properties": {"amount": {"type": "integer"}},
                "required": ["amount"]
            },
            "initial": "input.account",
            "queries": {
                "amount.read": {
                    "argsSchema": {"type": "object", "additionalProperties": false},
                    "resultSchema": {"type": "integer"},
                    "expression": "state.amount",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {}
        }
    });
    let (workflow, _) = world.publish(document).await;
    let chain = world
        .start(
            workflow,
            &json!({"account": {"amount": 19}, "next": {"account": {"amount": 0}}}),
        )
        .await;
    let session = world.session(&["segment.work"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"processed": true}))
        .await
        .expect("complete stateful segment");

    let placement = Placement::new(format!("public-state-chain-scheduler-{name}"));
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
    let scheduler = Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let report = scheduler.tick().await;
    assert_eq!(report.continuations_advanced, 1, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    let successor: Uuid = sqlx::query_scalar(
        "SELECT current_run_id FROM execution_chains WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("chain head");
    assert_ne!(successor, chain.as_uuid());
    let state: (i64, serde_json::Value) = sqlx::query_as(
        "SELECT operation.state_revision, convert_from(payload.content,'UTF8')::jsonb \
         FROM workflow_operation_states operation JOIN payloads payload \
           ON payload.tenant_id=operation.tenant_id AND payload.digest=operation.state_digest \
         WHERE operation.tenant_id=$1 AND operation.run_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(successor)
    .fetch_one(&world.db.pool)
    .await
    .expect("successor declared state");
    assert_eq!(state, (0, json!({"amount": 19})));
    let query = DeclaredQuery::for_chain(
        OperationScope::new(
            world.admin.tenant_id(),
            world.namespace.id(),
            OperationAddress::Chain(chain),
        ),
        None,
        ActivityName::parse("amount.read").expect("query name"),
        world.admin.subject().clone(),
    );
    let access = AuditRecord::by(
        &world.admin,
        AuditAction::RunQueried,
        ResourceType::Run,
        chain.to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
    );
    let snapshot = PgDeclaredOperationStore::new(world.db.pool.clone())
        .query_snapshot(&query, &access)
        .await
        .expect("query committed successor");
    assert_eq!(snapshot.run_id().as_uuid(), successor);
    let result = DeclaredOperationEvaluator::query(
        &JsonDeclaredOperationEvaluator,
        &snapshot,
        &ActivityName::parse("amount.read").expect("query name"),
        &DeclaredOperationArguments::parse(br#"{}"#).expect("query arguments"),
    )
    .expect("evaluate successor query");
    assert_eq!(result.response().run_id().as_uuid(), successor);
    assert_eq!(result.response().state_revision(), 0);
    assert_eq!(result.result().content(), b"19");
}

#[tokio::test]
async fn public_conditional_continuation_reaches_a_successful_final_outcome() {
    let world = world().await;
    let name = format!("public-chain-final-{}", Uuid::now_v7());
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "segment.work",
                "next": "again"
            },
            {
                "id": "again",
                "type": "condition",
                "expression": "input.continue",
                "ifTrue": "rollover",
                "ifFalse": "done"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            },
            {"id": "done", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": 3, "timeoutMs": 3_600_000}
    });
    let (workflow, _) = world.publish(document).await;
    let chain = world
        .start(
            workflow,
            &json!({"continue": true, "next": {"continue": false}}),
        )
        .await;
    let session = world.session(&["segment.work"]).await;
    let first = world.one_task(&session).await;
    world
        .complete(&session, &first, &json!({"processed": true}))
        .await
        .expect("complete continuing segment");

    let placement = Placement::new(format!("public-chain-final-scheduler-{name}"));
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
    let scheduler = Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let handoff = scheduler.tick().await;
    assert_eq!(handoff.continuations_advanced, 1, "{handoff:?}");
    assert_eq!(handoff.failed, 0, "{handoff:?}");

    let final_task = world.one_task(&session).await;
    world
        .complete(&session, &final_task, &json!({"processed": true}))
        .await
        .expect("complete final segment");
    let projection: (i32, Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT chain.segment_count,chain.final_outcome,run.status,segment.completion_kind \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         JOIN chain_segments segment ON segment.tenant_id=chain.tenant_id \
              AND segment.chain_id=chain.chain_id AND segment.run_id=chain.current_run_id \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("successful chain projection");
    assert_eq!(projection.0, 2);
    assert_eq!(projection.1.as_deref(), Some("SUCCEEDED"));
    assert_eq!(projection.2, "SUCCEEDED");
    assert_eq!(projection.3.as_deref(), Some("FINAL"));
    let after_final = scheduler.tick().await;
    assert_eq!(after_final.continuations_advanced, 0, "{after_final:?}");
    assert_eq!(after_final.failed, 0, "{after_final:?}");
}

#[tokio::test]
async fn segment_limit_finalizes_once_and_is_not_rediscovered() {
    let world = world().await;
    let name = format!("public-chain-limit-{}", Uuid::now_v7());
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "segment.work",
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            }
        ],
        "chainPolicy": {"maxSegments": 2, "timeoutMs": 3_600_000}
    });
    let (workflow, _) = world.publish(document).await;
    let chain = world
        .start(
            workflow,
            &json!({"segment": 1, "next": {"segment": 2, "next": {"segment": 3}}}),
        )
        .await;
    let session = world.session(&["segment.work"]).await;
    let placement = Placement::new(format!("public-chain-limit-scheduler-{name}"));
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
    let scheduler = Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);

    let first = world.one_task(&session).await;
    world
        .complete(&session, &first, &json!({"processed": true}))
        .await
        .expect("complete first segment");
    let first_tick = scheduler.tick().await;
    assert_eq!(first_tick.continuations_advanced, 1, "{first_tick:?}");
    assert_eq!(first_tick.failed, 0, "{first_tick:?}");

    let second = world.one_task(&session).await;
    world
        .complete(&session, &second, &json!({"processed": true}))
        .await
        .expect("complete final segment activity");
    let limit_tick = scheduler.tick().await;
    assert_eq!(limit_tick.continuations_advanced, 1, "{limit_tick:?}");
    assert_eq!(limit_tick.failed, 0, "{limit_tick:?}");
    let repeated_tick = scheduler.tick().await;
    assert_eq!(repeated_tick.continuations_advanced, 0, "{repeated_tick:?}");
    assert_eq!(repeated_tick.failed, 0, "{repeated_tick:?}");

    let projection: (Uuid, i32, Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT chain.current_run_id,chain.segment_count,chain.final_outcome,run.status, \
                segment.completion_kind \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         JOIN chain_segments segment ON segment.tenant_id=chain.tenant_id \
              AND segment.chain_id=chain.chain_id AND segment.run_id=chain.current_run_id \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("terminal chain projection");
    assert_ne!(projection.0, chain.as_uuid());
    assert_eq!(projection.1, 2);
    assert_eq!(projection.2.as_deref(), Some("FAILED"));
    assert_eq!(projection.3, "FAILED");
    assert_eq!(projection.4.as_deref(), Some("FINAL"));
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.finished' \
           AND resource_id=$2 AND reason='chain segment limit exhausted'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(projection.0.to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("chain limit audit");
    assert_eq!(audits, 1);
}

#[tokio::test]
async fn invalid_continuation_input_fails_the_chain_once() {
    let world = world().await;
    let name = format!("public-chain-expression-{}", Uuid::now_v7());
    let document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "rollover"},
            {"id": "rollover", "type": "continueAsNew", "input": "input.next.value"}
        ],
        "chainPolicy": {"maxSegments": 2, "timeoutMs": 3_600_000}
    });
    let (workflow, _) = world.publish(document).await;
    let chain = world.start(workflow, &json!({"next": {}})).await;
    let session = world.session(&["segment.work"]).await;
    let task = world.one_task(&session).await;
    world
        .complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete activity");

    let placement = Placement::new(format!("public-chain-expression-scheduler-{name}"));
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
    let scheduler = Scheduler::new(
        world.store.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let first = scheduler.tick().await;
    assert_eq!(first.continuations_advanced, 1, "{first:?}");
    assert_eq!(first.failed, 0, "{first:?}");
    let second = scheduler.tick().await;
    assert_eq!(second.continuations_advanced, 0, "{second:?}");
    assert_eq!(second.failed, 0, "{second:?}");

    let result: (String, Option<String>, i32) = sqlx::query_as(
        "SELECT run.status,chain.final_outcome,chain.segment_count \
         FROM execution_chains chain JOIN runs run \
           ON run.tenant_id=chain.tenant_id AND run.id=chain.current_run_id \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("chain outcome");
    assert_eq!(result, ("FAILED".to_owned(), Some("FAILED".to_owned()), 1));
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
           AND event_type='run.stop_requested' AND detail->>'code'='EXPRESSION_ERROR'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("durable failure");
    assert_eq!(failures, 1);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.finished' \
           AND resource_id=$2 AND reason='continuation expression failed'",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(chain.to_string())
    .fetch_one(&world.db.pool)
    .await
    .expect("failure audit");
    assert_eq!(audits, 1);
}

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

async fn scheduler(pool: &sqlx::PgPool, run: RunId) -> SchedulerAuthority {
    let owner = "chain-handoff-scheduler";
    let generation: i64 = sqlx::query_scalar(
        "UPDATE scheduler_shards SET generation=generation+1,owner=$2, \
                expires_at=clock_timestamp()+interval '1 hour',acquired_at=clock_timestamp() \
         WHERE shard=run_shard($1) RETURNING generation",
    )
    .bind(run.as_uuid())
    .bind(owner)
    .fetch_one(pool)
    .await
    .expect("scheduler fence");
    SchedulerAuthority::for_run(
        owner,
        run,
        u64::try_from(generation).expect("nonnegative generation"),
    )
}

#[tokio::test]
async fn prepared_continuation_commits_replays_and_rolls_back_atomically() {
    let world = world().await;
    let name = format!("chain-handoff-{}", Uuid::now_v7());
    let (workflow, _) = world
        .publish(document(&name, "work", vec![activity("work")]))
        .await;
    let document = serde_json::to_vec(&json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": "work",
        "nodes": [
            {
                "id": "work",
                "type": "activity",
                "activity": "segment.work",
                "next": "rollover"
            },
            {
                "id": "rollover",
                "type": "continueAsNew",
                "input": "input.next"
            }
        ],
        "chainPolicy": { "maxSegments": 3, "timeoutMs": 3_600_000 }
    }))
    .expect("continuation document");
    let parsed = JsonIrCodec
        .parse_continuation(&document)
        .expect("typed continuation");
    let version = WorkflowVersionId::new(Uuid::now_v7());
    sqlx::query(
        "INSERT INTO workflow_versions \
         (id,tenant_id,workflow_id,version_number,digest,document,published_by,published_at) \
         VALUES ($1,$2,$3,2,$4,$5,$6,$7)",
    )
    .bind(version.as_uuid())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(workflow.as_uuid())
    .bind(parsed.digest().to_hex())
    .bind(&document)
    .bind(world.admin.subject().as_str())
    .bind(crate::support::now())
    .execute(&world.db.pool)
    .await
    .expect("trusted continuation fixture");

    let deadline = crate::support::now() + Duration::hours(1);
    let chain_id = world
        .start_with(
            workflow,
            &json!({ "segment": 1, "next": { "segment": 2 } }),
            &format!("start-{name}"),
            Some(deadline),
        )
        .await
        .expect("initial segment");
    let tenant = world.admin.tenant_id();
    let scope = ChainScope::new(tenant, world.namespace.id());
    let mut tx = world.db.pool.begin().await.expect("handoff fixture tx");
    sqlx::query("UPDATE runs SET workflow_version_id=$3 WHERE tenant_id=$1 AND id=$2")
        .bind(tenant.as_uuid())
        .bind(chain_id.as_uuid())
        .bind(version.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("pin continuation version");
    sqlx::query("UPDATE steps SET step_id=$3 WHERE tenant_id=$1 AND run_id=$2 AND node='work'")
        .bind(tenant.as_uuid())
        .bind(chain_id.as_uuid())
        .bind(instance_step_id(
            version,
            chain_id,
            &ExecutionInstanceId::root(
                sagat_domain::workflow::NodeId::parse("work").expect("work node"),
            ),
        ))
        .execute(&mut *tx)
        .await
        .expect("pin entry step identity");
    initialize_chain_in_transaction(
        &mut tx,
        &ChainInitialization::new(scope, chain_id, ChainPolicy::new(3).expect("chain policy")),
    )
    .await
    .expect("initialize chain");

    let first_sequence: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant.as_uuid())
            .bind(chain_id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("run version");
    let selected_input = serde_json::to_vec(&json!({
        "segment": 2,
        "next": { "segment": 3 }
    }))
    .expect("selected input");
    let selected_digest = digest(&selected_input);
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(selected_digest.as_bytes().as_slice())
    .bind(&selected_input)
    .execute(&mut *tx)
    .await
    .expect("activity output payload");
    let now = crate::support::now();
    for (offset, event_type, detail, payload_ref) in [
        (
            1_i64,
            "step.leased",
            json!({ "attempt": 1, "generation": 1 }),
            None,
        ),
        (
            2,
            "step.started",
            json!({ "attempt": 1, "generation": 1 }),
            None,
        ),
        (
            3,
            "step.completed",
            json!({ "attempt": 1, "generation": 1 }),
            Some(selected_digest.as_bytes().as_slice()),
        ),
        (4, "step.routed", json!({ "activated": ["rollover"] }), None),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id,run_id,seq,event_type,node,attempt,detail,payload_digest,actor,occurred_at) \
             VALUES ($1,$2,$3,$4,'work',$5,$6,$7,$8,$9)",
        )
        .bind(tenant.as_uuid())
        .bind(chain_id.as_uuid())
        .bind(first_sequence + offset)
        .bind(event_type)
        .bind(if event_type == "step.routed" {
            None
        } else {
            Some(1_i32)
        })
        .bind(detail)
        .bind(payload_ref)
        .bind(world.admin.subject().as_str())
        .bind(now)
        .execute(&mut *tx)
        .await
        .expect("activity journal event");
    }
    sqlx::query(
        "UPDATE steps SET status='SUCCEEDED',attempt=1,generation=1,routed=true,output_digest=$3, \
                updated_at=$4 WHERE tenant_id=$1 AND run_id=$2 AND node='work'",
    )
    .bind(tenant.as_uuid())
    .bind(chain_id.as_uuid())
    .bind(selected_digest.as_bytes().as_slice())
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("complete activity");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,routed,updated_at, \
          instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,'rollover',$3,$4,'PENDING',false,$5,'{}','{}')",
    )
    .bind(tenant.as_uuid())
    .bind(chain_id.as_uuid())
    .bind(instance_step_id(
        version,
        chain_id,
        &ExecutionInstanceId::root(
            sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node"),
        ),
    ))
    .bind(world.namespace.id().as_uuid())
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("pending continuation");
    let run_version = first_sequence + 4;
    sqlx::query("UPDATE runs SET run_version=$3,updated_at=$4 WHERE tenant_id=$1 AND id=$2")
        .bind(tenant.as_uuid())
        .bind(chain_id.as_uuid())
        .bind(run_version)
        .bind(now)
        .execute(&mut *tx)
        .await
        .expect("advance fixture projection");
    tx.commit().await.expect("commit handoff fixture");

    let successor = RunId::generate();
    let authority = scheduler(&world.db.pool, chain_id).await;
    let audit = AuditRecord::by(
        &world.admin,
        AuditAction::RunContinued,
        ResourceType::Run,
        chain_id.to_string(),
        None,
        now,
    );
    let continuation_outbox = Uuid::now_v7();
    let successor_outbox = Uuid::now_v7();
    let command = ChainHandoffCommand::new(
        scope,
        chain_id,
        chain_id,
        0,
        u64::try_from(run_version).expect("positive run version"),
        successor,
        ContinuationTransfer::new(selected_digest, None),
        authority.clone(),
        world.admin.subject().clone(),
        None,
        now,
        audit.clone(),
        continuation_outbox,
        successor_outbox,
    );
    let cache = DefinitionCache::new(Arc::new(ContinuationCodec), 8);
    let store = PgChainProgressStore::new(world.db.pool.clone(), cache);
    let prepared = EvaluatedChainHandoffCommand::new(
        command,
        sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node"),
        selected_input,
        None,
        None,
    )
    .expect("prepared continuation");
    let applied = store
        .continue_evaluated(&prepared)
        .await
        .expect("planner-owned handoff commits atomically");
    assert_eq!(applied.successor(), successor);
    assert!(!applied.replayed());

    let projection = sqlx::query(
        "SELECT old.status AS old_status,boundary.status AS boundary_status,boundary.routed, \
                next.input_digest,next.workflow_version_id,chain.current_run_id,chain.segment_count \
         FROM runs old \
         JOIN steps boundary ON boundary.tenant_id=old.tenant_id AND boundary.run_id=old.id \
              AND boundary.node='rollover' \
         JOIN execution_chains chain ON chain.tenant_id=old.tenant_id AND chain.chain_id=old.id \
         JOIN runs next ON next.tenant_id=old.tenant_id AND next.id=chain.current_run_id \
         WHERE old.tenant_id=$1 AND old.id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(chain_id.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("committed handoff projection");
    assert_eq!(projection.get::<String, _>("old_status"), "SUCCEEDED");
    assert_eq!(projection.get::<String, _>("boundary_status"), "SUCCEEDED");
    assert!(projection.get::<bool, _>("routed"));
    assert_eq!(
        projection.get::<Vec<u8>, _>("input_digest"),
        selected_digest.as_bytes()
    );
    assert_eq!(
        projection.get::<Uuid, _>("workflow_version_id"),
        version.as_uuid()
    );
    assert_eq!(
        projection.get::<Uuid, _>("current_run_id"),
        successor.as_uuid()
    );
    assert_eq!(projection.get::<i32, _>("segment_count"), 2);
    store
        .verify_chain_journal(tenant, chain_id)
        .await
        .expect("predecessor journal matches projection");
    store
        .verify_chain_journal(tenant, successor)
        .await
        .expect("successor journal matches projection");

    let restarted = PgChainProgressStore::new(
        world.db.pool.clone(),
        DefinitionCache::new(Arc::new(ContinuationCodec), 8),
    );
    let replayed = restarted
        .continue_evaluated(&prepared)
        .await
        .expect("restart replays the committed handoff");
    assert!(replayed.replayed());
    assert_eq!(replayed.successor(), successor);

    let conflicting_input = serde_json::to_vec(&json!({ "segment": 99 })).expect("conflict input");
    let conflicting = EvaluatedChainHandoffCommand::new(
        ChainHandoffCommand::new(
            scope,
            chain_id,
            chain_id,
            0,
            u64::try_from(run_version).expect("positive run version"),
            successor,
            ContinuationTransfer::new(digest(&conflicting_input), None),
            authority.clone(),
            world.admin.subject().clone(),
            None,
            now,
            audit.clone(),
            continuation_outbox,
            successor_outbox,
        ),
        sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node"),
        conflicting_input,
        None,
        None,
    )
    .expect("internally consistent conflicting request");
    assert!(matches!(
        restarted.continue_evaluated(&conflicting).await,
        Err(ChainProgressError::Chain(
            sagat_domain::execution::ChainError::ContinuationConflict
        ))
    ));

    let third_input = serde_json::to_vec(&json!({ "segment": 3 })).expect("third input");
    let third_digest = digest(&third_input);
    let second_now = crate::support::now();
    let mut second_tx = world.db.pool.begin().await.expect("second segment fixture");
    sqlx::query(
        "INSERT INTO payloads (tenant_id,digest,content) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(third_digest.as_bytes().as_slice())
    .bind(&third_input)
    .execute(&mut *second_tx)
    .await
    .expect("second activity output payload");
    for (sequence, event_type, detail, payload_digest) in [
        (
            4_i64,
            "step.leased",
            json!({ "attempt": 1, "generation": 1 }),
            None,
        ),
        (
            5,
            "step.started",
            json!({ "attempt": 1, "generation": 1 }),
            None,
        ),
        (
            6,
            "step.completed",
            json!({ "attempt": 1, "generation": 1 }),
            Some(third_digest.as_bytes().as_slice()),
        ),
        (7, "step.routed", json!({ "activated": ["rollover"] }), None),
    ] {
        sqlx::query(
            "INSERT INTO run_events \
             (tenant_id,run_id,seq,event_type,node,attempt,detail,payload_digest,actor,occurred_at) \
             VALUES ($1,$2,$3,$4,'work',$5,$6,$7,$8,$9)",
        )
        .bind(tenant.as_uuid())
        .bind(successor.as_uuid())
        .bind(sequence)
        .bind(event_type)
        .bind(if event_type == "step.routed" {
            None
        } else {
            Some(1_i32)
        })
        .bind(detail)
        .bind(payload_digest)
        .bind(world.admin.subject().as_str())
        .bind(second_now)
        .execute(&mut *second_tx)
        .await
        .expect("second activity journal event");
    }
    sqlx::query(
        "UPDATE steps SET status='SUCCEEDED',attempt=1,generation=1,routed=true,output_digest=$3, \
                updated_at=$4 WHERE tenant_id=$1 AND run_id=$2 AND node='work'",
    )
    .bind(tenant.as_uuid())
    .bind(successor.as_uuid())
    .bind(third_digest.as_bytes().as_slice())
    .bind(second_now)
    .execute(&mut *second_tx)
    .await
    .expect("complete second activity");
    sqlx::query(
        "INSERT INTO steps \
         (tenant_id,run_id,node,step_id,namespace_id,status,routed,updated_at, \
          instance_path_nodes,instance_path_indices) \
         VALUES ($1,$2,'rollover',$3,$4,'PENDING',false,$5,'{}','{}')",
    )
    .bind(tenant.as_uuid())
    .bind(successor.as_uuid())
    .bind(instance_step_id(
        version,
        successor,
        &ExecutionInstanceId::root(
            sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node"),
        ),
    ))
    .bind(world.namespace.id().as_uuid())
    .bind(second_now)
    .execute(&mut *second_tx)
    .await
    .expect("pending second continuation");
    sqlx::query("UPDATE runs SET run_version=7,updated_at=$3 WHERE tenant_id=$1 AND id=$2")
        .bind(tenant.as_uuid())
        .bind(successor.as_uuid())
        .bind(second_now)
        .execute(&mut *second_tx)
        .await
        .expect("advance second projection");
    second_tx.commit().await.expect("commit second fixture");

    let failed_successor = RunId::generate();
    let failed_audit = AuditRecord::by(
        &world.admin,
        AuditAction::RunContinued,
        ResourceType::Run,
        successor.to_string(),
        None,
        second_now,
    );
    let rollback = EvaluatedChainHandoffCommand::new(
        ChainHandoffCommand::new(
            scope,
            chain_id,
            successor,
            1,
            7,
            failed_successor,
            ContinuationTransfer::new(third_digest, None),
            scheduler(&world.db.pool, successor).await,
            world.admin.subject().clone(),
            None,
            second_now,
            failed_audit,
            continuation_outbox,
            Uuid::now_v7(),
        ),
        sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node"),
        third_input,
        None,
        None,
    )
    .expect("rollback handoff");
    assert!(matches!(
        restarted.continue_evaluated(&rollback).await,
        Err(ChainProgressError::Storage(_))
    ));
    let rollback_projection = sqlx::query(
        "SELECT chain.current_run_id,chain.segment_count,run.status AS run_status,run.run_version, \
                step.status AS step_status, \
                step.routed, \
                EXISTS(SELECT 1 FROM runs created WHERE created.tenant_id=chain.tenant_id \
                       AND created.id=$3) AS successor_exists \
         FROM execution_chains chain \
         JOIN runs run ON run.tenant_id=chain.tenant_id AND run.id=$2 \
         JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
              AND step.node='rollover' \
         WHERE chain.tenant_id=$1 AND chain.chain_id=$4",
    )
    .bind(tenant.as_uuid())
    .bind(successor.as_uuid())
    .bind(failed_successor.as_uuid())
    .bind(chain_id.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("rolled back projection");
    assert_eq!(
        rollback_projection.get::<Uuid, _>("current_run_id"),
        successor.as_uuid()
    );
    assert_eq!(rollback_projection.get::<i32, _>("segment_count"), 2);
    assert_eq!(
        rollback_projection.get::<String, _>("run_status"),
        "RUNNING"
    );
    assert_eq!(rollback_projection.get::<i64, _>("run_version"), 7);
    assert_eq!(
        rollback_projection.get::<String, _>("step_status"),
        "PENDING"
    );
    assert!(!rollback_projection.get::<bool, _>("routed"));
    assert!(!rollback_projection.get::<bool, _>("successor_exists"));

    let _new_fence = scheduler(&world.db.pool, chain_id).await;
    assert!(matches!(
        restarted.continue_evaluated(&prepared).await,
        Err(ChainProgressError::SchedulerFenceLost)
    ));

    let foreign_tenant = sagat_domain::identity::TenantId::new(Uuid::now_v7());
    let foreign_scope = ChainScope::new(
        foreign_tenant,
        sagat_domain::identity::NamespaceId::new(Uuid::now_v7()),
    );
    let foreign_at = crate::support::now();
    let foreign_input = serde_json::to_vec(&json!({ "segment": 2 })).expect("foreign input");
    let foreign = EvaluatedChainHandoffCommand::new(
        ChainHandoffCommand::new(
            foreign_scope,
            chain_id,
            chain_id,
            0,
            u64::try_from(run_version).expect("positive run version"),
            successor,
            ContinuationTransfer::new(digest(&foreign_input), None),
            authority,
            sagat_domain::identity::Subject::engine(),
            None,
            foreign_at,
            AuditRecord::engine(
                foreign_tenant,
                AuditAction::RunContinued,
                ResourceType::Run,
                chain_id.to_string(),
                "continuation selected",
                None,
                foreign_at,
            ),
            Uuid::now_v7(),
            Uuid::now_v7(),
        ),
        sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node"),
        foreign_input,
        None,
        None,
    )
    .expect("foreign handoff command");
    assert!(matches!(
        restarted.continue_evaluated(&foreign).await,
        Err(ChainProgressError::Missing)
    ));

    let counts = sqlx::query(
        "SELECT \
           (SELECT count(*) FROM chain_segments WHERE tenant_id=$1 AND chain_id=$2) AS segments, \
           (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND id IN ($3,$4)) AS outboxes, \
           (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND id=$5) AS audits",
    )
    .bind(tenant.as_uuid())
    .bind(chain_id.as_uuid())
    .bind(continuation_outbox)
    .bind(successor_outbox)
    .bind(audit.id().as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("single committed handoff evidence");
    assert_eq!(counts.get::<i64, _>("segments"), 2);
    assert_eq!(counts.get::<i64, _>("outboxes"), 2);
    assert_eq!(counts.get::<i64, _>("audits"), 1);

    sqlx::query("UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 second'")
        .execute(&world.db.pool)
        .await
        .expect("expire fixture scheduler leases");
    let scheduler_store = PgRunStore::new(
        world.db.pool.clone(),
        DefinitionCache::new(Arc::new(ContinuationCodec), 8),
    );
    let placement = Placement::new(format!("continuation-planner-{name}"));
    Keeper::new(
        scheduler_store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire continuation shard");
    assert!(
        placement
            .snapshot()
            .lease_of(Shard::of(successor.as_uuid()), std::time::Instant::now())
            .is_some(),
        "the planner must hold the current head shard"
    );
    let planner = || {
        Scheduler::new(
            scheduler_store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings::default(),
        )
        .with_placement(placement.clone())
    };
    let rollover = sagat_domain::workflow::NodeId::parse("rollover").expect("rollover node");
    let left_planner = planner();
    let right_planner = planner();
    let (left, right) = tokio::join!(
        left_planner.advance_continuation(tenant, world.namespace.id(), chain_id, &rollover),
        right_planner.advance_continuation(tenant, world.namespace.id(), chain_id, &rollover),
    );
    let (committed, rejected) = match (left, right) {
        (Ok(write), Err(error)) | (Err(error), Ok(write)) => (write, error),
        outcomes => panic!("one planner must commit and one must lose the race: {outcomes:?}"),
    };
    assert_eq!(committed.predecessor(), successor);
    assert_eq!(committed.successor_index(), 2);
    assert!(!committed.replayed());
    assert!(matches!(
        rejected,
        sagat_scheduler::ContinuationAdvanceError::NotContinuation
            | sagat_scheduler::ContinuationAdvanceError::Progress(ChainProgressError::Chain(
                sagat_domain::execution::ChainError::ContinuationConflict
            ))
    ));
    let advanced: (Uuid, i32) = sqlx::query_as(
        "SELECT current_run_id,segment_count FROM execution_chains \
         WHERE tenant_id=$1 AND chain_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(chain_id.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("scheduler handoff projection");
    assert_eq!(advanced.0, committed.successor().as_uuid());
    assert_eq!(advanced.1, 3);
    restarted
        .verify_chain_journal(tenant, successor)
        .await
        .expect("scheduler predecessor journal matches projection");
    restarted
        .verify_chain_journal(tenant, committed.successor())
        .await
        .expect("scheduler successor journal matches projection");
}
