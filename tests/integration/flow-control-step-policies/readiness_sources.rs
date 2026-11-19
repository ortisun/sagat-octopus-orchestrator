//! The readiness sources of T022-096 that `readiness.rs` does not cover (FR-022-051; ADR-0089 decision 1, ADR-0094
//! pending): an approval's decision, an operator's retry and a declared Update. Each run here is checked by
//! `verify_run` (the journal alone agrees; invariant 11).
//!
//! An Update is not a readiness source on this tree: a declared Update replaces the run's business state
//! (`state.updated`) and releases no step, whatever waits. Its evidence is therefore that an applied Update leaves a
//! queued gated step exactly as it was, and the commit is consistent. An Update-with-Start admits its run through
//! `PgRunStore::create`, whose entry step is decided like any start (`readiness::the_entry_step_is_decided_by_the_admission_that_releases_it`).

use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalProgressStore,
};
use sagat_application::declared_operations::{DeclaredOperationStore, DeclaredUpdate};
use sagat_application::operation_evaluation::DeclaredOperationArguments;
use sagat_application::runs::ReviewAction;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    ApprovalDecision, ApprovalDecisionCommand, ApprovalReceiptId, ApprovalRequestId, DecisionKey,
    OperationAddress, OperationError, OperationScope, RunId, UpdateKey, UpdateReceiptStatus,
    UpdateRequest,
};
use sagat_domain::identity::{Principal, Role};
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_infra_ir::{JsonDeclaredOperationEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgDeclaredOperationStore;
use sagat_scheduler::SchedulerSettings;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::readiness::{granted, hints, queued, queued_after, serial};
use crate::support::engine::{World, activity, document, flaky};
use crate::support::flow_control::{gated_world, publish};
use crate::support::scale::{TTL, keeper, scheduler};

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

// ------------------------------------------------------------------------------------------------------- approval

/// `review` (an approval only an operator who did not start the run may decide) routing its approval to the gated `g`.
fn approval_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "review",
        "nodes": [
            {
                "id": "review",
                "type": "approval",
                "timeoutMs": 86_400_000,
                "policy": {"roles": ["operator"], "excludeInitiator": true},
                "onApprove": "g",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            serial(activity("g")),
            {"id": "rejected", "type": "end"},
            {"id": "expired", "type": "end"}
        ]
    })
}

async fn request_of(w: &World, run: RunId) -> ApprovalRequestId {
    ApprovalRequestId::new(
        sqlx::query_scalar(
            "SELECT request_id FROM approval_requests WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("an opened request"),
    )
}

async fn approve(w: &World, operator: &Principal, request: ApprovalRequestId, key: &str) {
    let at = crate::support::now();
    let audit = AuditRecord::by(
        operator,
        AuditAction::ApprovalDecided,
        ResourceType::ApprovalRequest,
        request.to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        at,
    );
    w.store
        .decide(&ApprovalDecisionAttempt::new(
            ApprovalAddress::new(w.admin.tenant_id(), w.namespace.id(), request),
            operator.subject().clone(),
            ApprovalReceiptId::generate(),
            ApprovalDecisionCommand::new(
                DecisionKey::parse(key).expect("decision key"),
                Digest::new([19; 32]),
                ApprovalDecision::Approve,
                None,
            ),
            audit,
            Uuid::now_v7(),
            at,
        ))
        .await
        .expect("approved");
}

/// Source: an approval's decision routes to a gated step, which the scheduler's route release decides in its commit:
/// granted while the key has room, queued behind the holder once it has none.
#[tokio::test]
async fn an_approved_request_releases_its_gated_route_into_its_gates() {
    let w = gated_world().await;
    let workflow = publish(&w, approval_document("approval-gated")).await;
    let operator = w.member("approver", Role::Operator).await;
    let alpha = keeper(&w, "alpha", TTL);
    alpha.round().await.expect("the shards");
    let sched = scheduler(
        &w,
        SchedulerSettings::default(),
        Some(alpha.placement().clone()),
    );
    let first = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let second = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let opened = sched.tick().await;
    assert_eq!(opened.approvals_opened, 2, "{opened:?}");

    approve(&w, &operator, request_of(&w, first).await, "approve-first").await;
    let routed = sched.tick().await;
    assert_eq!(routed.approval_routes_released, 1, "{routed:?}");
    assert_eq!(routed.failed, 0, "{routed:?}");
    granted(&w, first, "g", (true, false)).await;
    assert_eq!(
        hints(&w, first, "g").await,
        1,
        "the granted route is announced"
    );

    approve(
        &w,
        &operator,
        request_of(&w, second).await,
        "approve-second",
    )
    .await;
    let routed = sched.tick().await;
    assert_eq!(routed.approval_routes_released, 1, "{routed:?}");
    assert_eq!(routed.failed, 0, "{routed:?}");
    queued(&w, second, "g", (true, false)).await;
    assert_eq!(
        hints(&w, second, "g").await,
        0,
        "no task is announced for a waiting route"
    );
    let journal = w.journal(second).await;
    assert!(
        journal.iter().any(|e| e == "approval.decided"),
        "{journal:?}"
    );
}

// ------------------------------------------------------------------------------------------------- operator retry

/// Source: an operator's retry (`step.retry_requested`) of a step that waits out its retry delay: the next attempt is
/// decided in the operator's commit, queued while another run holds the key and granted when the key has room.
#[tokio::test]
async fn an_operators_retry_decides_the_next_attempt() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("operator-retry-gated", "g", vec![serial(flaky("g", 3))]),
    )
    .await;
    let session = w.session(&["act.g"]).await;
    let held = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    let free = w.start(workflow, &json!({"accountId": "acct-2"})).await;
    let tasks = w.poll(&session, 8).await;
    assert_eq!(tasks.len(), 2, "both first attempts were granted");
    for task in &tasks {
        w.fail(&session, task, true).await.expect("retry scheduled");
    }
    // another run takes acct-1's only permit while `held` waits out its delay
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, holder, "g", (true, false)).await;

    w.review(held, "g", ReviewAction::Retry, "the provider is back")
        .await
        .expect("retry requested");
    queued_after(&w, held, "g", (true, false), 1).await;
    assert_eq!(
        hints(&w, held, "g").await,
        1,
        "only the first attempt's hint"
    );

    w.review(free, "g", ReviewAction::Retry, "the provider is back")
        .await
        .expect("retry requested");
    let (status, _, attempt) = super::readiness::step(&w, free, "g").await;
    assert_eq!((status.as_str(), attempt), ("READY", 1));
    granted(&w, free, "g", (true, false)).await;
    let journal = w.journal(free).await;
    assert!(
        journal.iter().any(|e| e == "step.retry_requested"),
        "{journal:?}"
    );
    // the worker gets the granted retry (and the holder's attempt), never the queued one
    let mut leased: Vec<_> = w
        .poll(&session, 8)
        .await
        .iter()
        .map(|task| task.run_id)
        .collect();
    leased.sort();
    let mut expected = vec![free, holder];
    expected.sort();
    assert_eq!(leased, expected);
    w.assert_consistent(held).await;
}

// --------------------------------------------------------------------------------------------------------- update

/// A declared state with one Update that replaces it, declared for the version the run pins.
fn state_document() -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "gated-state"},
        "entrypoint": "done",
        "nodes": [{"id": "done", "type": "end"}],
        "state": {
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"balance": {"type": "integer"}},
                "required": ["balance"]
            },
            "initial": "input.account",
            "queries": {
                "balance.read": {
                    "argsSchema": {"type": "integer"},
                    "resultSchema": {"type": "integer"},
                    "expression": "state.balance + args",
                    "disclosure": "declared-business-result-v1"
                }
            },
            "updates": {
                "balance.replace": {
                    "argsSchema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "allowed": {"type": "boolean"},
                            "nextState": {"type": "object"}
                        },
                        "required": ["allowed", "nextState"]
                    },
                    "validate": "args.allowed",
                    "state": "args.nextState",
                    "result": "newState.balance",
                    "resultSchema": {"type": "integer"},
                    "disclosure": "declared-business-result-v1"
                }
            }
        }
    })
}

/// The declarations and the state row a published declared state gives a run (as `operations.rs` seeds them).
async fn seed_state(
    w: &World,
    version: WorkflowVersionId,
    run: RunId,
    declared: &sagat_infra_ir::ParsedDeclaredState,
) {
    let state_bytes = br#"{"balance":10}"#;
    let state_digest = digest(state_bytes);
    let state_schema = declared.state().schema().digest();
    let tenant = w.admin.tenant_id().as_uuid();
    sqlx::query(
        "INSERT INTO payloads (tenant_id, digest, content) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(state_digest.as_bytes().as_slice())
    .bind(state_bytes.as_slice())
    .execute(&w.db.pool)
    .await
    .expect("state payload");
    let queries = declared.state().queries().map(|operation| {
        (
            "QUERY",
            operation.name(),
            operation.digest(),
            operation.result_schema().digest(),
        )
    });
    let updates = declared.state().updates().map(|operation| {
        (
            "UPDATE",
            operation.name(),
            operation.digest(),
            operation.result_schema().digest(),
        )
    });
    for (kind, name, declaration, result) in queries.chain(updates) {
        sqlx::query(
            "INSERT INTO declared_operation_definitions \
             (tenant_id, namespace_id, workflow_version_id, operation_kind, operation_name, \
              declaration_digest, state_schema_digest, result_schema_digest, disclosure, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'declared-business-result-v1',$9)",
        )
        .bind(tenant)
        .bind(w.namespace.id().as_uuid())
        .bind(version.as_uuid())
        .bind(kind)
        .bind(name.as_str())
        .bind(declaration.as_bytes().as_slice())
        .bind(state_schema.as_bytes().as_slice())
        .bind(result.as_bytes().as_slice())
        .bind(crate::support::now())
        .execute(&w.db.pool)
        .await
        .expect("operation declaration");
    }
    let run_version: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id = $1 AND id = $2")
            .bind(tenant)
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("run version");
    sqlx::query(
        "INSERT INTO workflow_operation_states \
         (tenant_id, namespace_id, run_id, state_digest, state_schema_digest, state_size_bytes, \
          state_revision, last_changed_run_version, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,0,$7,$8)",
    )
    .bind(tenant)
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .bind(state_digest.as_bytes().as_slice())
    .bind(state_schema.as_bytes().as_slice())
    .bind(i32::try_from(state_bytes.len()).expect("state size"))
    .bind(run_version)
    .bind(crate::support::now())
    .execute(&w.db.pool)
    .await
    .expect("operation state");
}

/// "Update" among the readiness sources: an applied Update releases nothing, so a queued gated step is left exactly as
/// it was (its waiter row, no attempt, no hint) and the commit stays consistent; nothing bypasses the gate.
#[tokio::test]
async fn an_applied_update_leaves_a_queued_gated_step_as_it_was() {
    let w = gated_world().await;
    let workflow = publish(
        &w,
        document("update-gated", "g", vec![serial(activity("g"))]),
    )
    .await;
    let holder = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    granted(&w, holder, "g", (true, false)).await;
    let run = w.start(workflow, &json!({"accountId": "acct-1"})).await;
    queued(&w, run, "g", (true, false)).await;
    let version = WorkflowVersionId::new(
        sqlx::query_scalar("SELECT workflow_version_id FROM runs WHERE tenant_id = $1 AND id = $2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("version"),
    );
    let declared = JsonIrCodec
        .parse_declared_state(state_document().to_string().as_bytes())
        .expect("declared state");
    seed_state(&w, version, run, &declared).await;
    let before = w.journal(run).await;

    let arguments = br#"{"allowed":true,"nextState":{"balance":20}}"#;
    let name = ActivityName::parse("balance.replace").expect("update name");
    let command = DeclaredUpdate::new(
        UpdateRequest::new(
            OperationScope::new(
                w.admin.tenant_id(),
                w.namespace.id(),
                OperationAddress::Run(run),
            ),
            run,
            name.clone(),
            UpdateKey::parse("replace-while-queued").expect("update key"),
            digest(arguments),
            Some(0),
            w.admin.subject().clone(),
        ),
        DeclaredOperationArguments::parse(arguments).expect("arguments"),
        AuditRecord::by(
            &w.admin,
            AuditAction::RunUpdated,
            ResourceType::Run,
            run.to_string(),
            Some("0af7651916cd43dd8448eb211c80319c".into()),
            crate::support::now(),
        ),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let evaluated = command.arguments().clone();
    let resolution = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(
            &command,
            |snapshot, arguments| {
                JsonDeclaredOperationEvaluator
                    .validate_update_arguments(declared.state(), &name, snapshot, arguments)
                    .map_err(|_| OperationError::EvaluationFailed)
            },
            |snapshot| {
                JsonDeclaredOperationEvaluator
                    .update_snapshot(declared.state(), &name, snapshot, &evaluated)
                    .and_then(|plan| plan.store_decision(snapshot))
                    .map_err(|_| OperationError::EvaluationFailed)
            },
        )
        .await
        .expect("applied");
    assert_eq!(resolution.receipt().status(), UpdateReceiptStatus::Applied);

    let after = w.journal(run).await;
    assert_eq!(
        &after[..before.len()],
        &before[..],
        "the journal only grows"
    );
    assert_eq!(
        &after[before.len()..],
        ["state.updated"],
        "an Update releases nothing"
    );
    queued(&w, run, "g", (true, false)).await;
    assert_eq!(hints(&w, run, "g").await, 0);
}
