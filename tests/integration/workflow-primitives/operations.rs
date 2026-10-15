//! Real PostgreSQL evidence for declared Query snapshots and durable Update receipts.

use sagat_application::declared_operations::{
    DeclaredOperationStore, DeclaredOperationStoreError, DeclaredQuery, DeclaredUpdate,
    DeclaredUpdateDecision, EvaluatedOperationWrite, declared_update_request_hash,
};
use sagat_application::definitions::{Applied, DefinitionStore, Idempotency, PublishContext};
use sagat_application::execution::{RunInfo, RunTx};
use sagat_application::operation_evaluation::{
    DeclaredOperationArguments, OperationEvaluationError,
};
use sagat_application::quotas::RunLimits;
use sagat_application::runs::{NewRun, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{
    OperationAddress, OperationError, OperationScope, OperationValue, RunId, UpdateKey,
    UpdateReceiptStatus, UpdateRequest,
};
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_infra_ir::{JsonDeclaredOperationEvaluator, JsonIrCodec, ParsedDeclaredState};
use sagat_infra_postgres::PgDeclaredOperationStore;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::engine::{chain, world};

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
}

fn operation_arguments(bytes: &[u8]) -> DeclaredOperationArguments {
    DeclaredOperationArguments::parse(bytes).expect("canonical arguments")
}

#[test]
fn canonical_update_hash_frames_operation_arguments_revision_and_run() {
    let operation = ActivityName::parse("balance.replace").expect("operation");
    let other_operation = ActivityName::parse("balance.reserve").expect("operation");
    let run = "0198c5a0-0000-7000-8000-000000000001"
        .parse::<RunId>()
        .expect("run");
    let other_run = "0198c5a0-0000-7000-8000-000000000002"
        .parse::<RunId>()
        .expect("run");
    let arguments = operation_arguments(br#"{"allowed":true,"value":2}"#);
    let reordered = operation_arguments(br#"{"value":2,"allowed":true}"#);
    let identity = declared_update_request_hash(&operation, &arguments, Some(4), run);

    assert_eq!(
        identity,
        declared_update_request_hash(&operation, &reordered, Some(4), run)
    );
    assert_ne!(
        identity,
        declared_update_request_hash(&other_operation, &arguments, Some(4), run)
    );
    assert_ne!(
        identity,
        declared_update_request_hash(&operation, &arguments, Some(5), run)
    );
    assert_ne!(
        identity,
        declared_update_request_hash(&operation, &arguments, Some(4), other_run)
    );
}

fn evaluated_operations_document(query_expression: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "evaluated-operations"},
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
                    "expression": query_expression,
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

async fn publish_declared_internally(
    w: &crate::support::engine::World,
    document: Value,
) -> (WorkflowId, WorkflowVersionId, ParsedDeclaredState) {
    let workflow = w.create_draft(document.clone()).await;
    let record = w
        .defs
        .get_workflow(&w.admin, workflow, &crate::support::engine::ctx())
        .await
        .expect("draft");
    let declared = JsonIrCodec
        .parse_declared_state(document.to_string().as_bytes())
        .expect("declared state");
    let publication = declared
        .state()
        .publication()
        .expect("publication metadata");
    let version = WorkflowVersionId::generate();
    let now = crate::support::now();
    let audit = AuditRecord::by(
        &w.admin,
        AuditAction::WorkflowPublished,
        ResourceType::WorkflowVersion,
        version.to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".to_owned()),
        now,
    );
    let outcome = w
        .store
        .publish(
            &w.admin,
            workflow,
            record.workflow.draft_revision(),
            version,
            declared.parsed().digest,
            &declared.parsed().canonical,
            Some(&publication),
            None,
            &[],
            &[],
            &PublishContext::default(),
            &audit,
            &Idempotency {
                scope: format!("workflow.publish:{workflow}"),
                key: format!("declared-{}", Uuid::now_v7()),
                fingerprint: declared.parsed().digest.as_bytes().to_vec(),
            },
            now,
        )
        .await
        .expect("publication transaction")
        .expect("workflow");
    assert!(matches!(outcome, Applied::Done(_)));
    (workflow, version, declared)
}

async fn create_declared_run(
    w: &crate::support::engine::World,
    workflow: WorkflowId,
    version: WorkflowVersionId,
    declared: &ParsedDeclaredState,
    input: Vec<u8>,
) -> Result<RunId, sagat_application::error::StoreError> {
    let run = RunId::generate();
    let now = crate::support::now();
    let created = w
        .store
        .create(NewRun {
            limits: RunLimits::UNLIMITED,
            info: RunInfo {
                run_id: run,
                tenant_id: w.admin.tenant_id(),
                namespace_id: w.namespace.id(),
                workflow_id: workflow,
                version_id: version,
                started_by: w.admin.subject().as_str().to_owned(),
                correlation_id: None,
                trace_id: None,
                rerun_of: None,
                parent_span: None,
                deadline_at: None,
                created_at: now,
                parent: None,
                depth: 0,
                root: run,
            },
            definition: std::sync::Arc::new(declared.parsed().definition.clone()),
            input,
            input_blob: None,
            audit: AuditRecord::by(
                &w.admin,
                AuditAction::RunStarted,
                ResourceType::Run,
                run.to_string(),
                None,
                now,
            ),
            idempotency: Idempotency {
                scope: format!("run.start:{}", w.namespace.id()),
                key: format!("declared-run-{}", Uuid::now_v7()),
                fingerprint: run.as_uuid().as_bytes().to_vec(),
            },
        })
        .await?;
    let Applied::Done(tx) = created else {
        panic!("fresh run cannot replay");
    };
    tx.commit().await?;
    Ok(run)
}

#[tokio::test]
async fn publication_and_run_start_atomically_retain_declared_metadata_and_initial_state() {
    let w = world().await;
    let document = evaluated_operations_document("state.balance + args");
    let (workflow, version, declared) = publish_declared_internally(&w, document).await;

    let definitions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM declared_operation_definitions \
         WHERE tenant_id=$1 AND namespace_id=$2 AND workflow_version_id=$3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(version.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("published declarations");
    assert_eq!(definitions, 2);

    let run = create_declared_run(
        &w,
        workflow,
        version,
        &declared,
        br#"{"account":{"balance":17}}"#.to_vec(),
    )
    .await
    .expect("create run");
    let row: (Vec<u8>, Vec<u8>, i64, i64, Vec<u8>) = sqlx::query_as(
        "SELECT s.state_digest, s.state_schema_digest, s.state_revision, \
                s.last_changed_run_version, p.content \
         FROM workflow_operation_states s \
         JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.state_digest \
         WHERE s.tenant_id=$1 AND s.namespace_id=$2 AND s.run_id=$3",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("initial state");
    assert_eq!(row.0, digest(&row.4).as_bytes().as_slice());
    assert_eq!(
        row.1,
        declared.state().schema().digest().as_bytes().as_slice()
    );
    assert_eq!(row.2, 0);
    assert_eq!(row.3, 0);
    assert_eq!(row.4, br#"{"balance":17}"#);
}

#[tokio::test]
async fn invalid_initial_state_refuses_start_without_a_run_or_retained_payload() {
    let w = world().await;
    let (workflow, version, declared) =
        publish_declared_internally(&w, evaluated_operations_document("state.balance + args"))
            .await;
    let before_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND workflow_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(workflow.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("run count");

    assert!(
        create_declared_run(
            &w,
            workflow,
            version,
            &declared,
            br#"{"account":{"balance":"canary-secret"}}"#.to_vec(),
        )
        .await
        .is_err()
    );
    let after_runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runs WHERE tenant_id=$1 AND workflow_id=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(workflow.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("run count");
    assert_eq!(after_runs, before_runs);
}

async fn seed_evaluated_state(
    w: &crate::support::engine::World,
    version: WorkflowVersionId,
    run: RunId,
    declared: &ParsedDeclaredState,
) -> OperationValue {
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
    for (kind, operation, declaration_digest, result_schema) in declared
        .state()
        .queries()
        .map(|operation| {
            (
                "QUERY",
                operation.name(),
                operation.digest(),
                operation.result_schema().digest(),
            )
        })
        .chain(declared.state().updates().map(|operation| {
            (
                "UPDATE",
                operation.name(),
                operation.digest(),
                operation.result_schema().digest(),
            )
        }))
    {
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
        .bind(operation.as_str())
        .bind(declaration_digest.as_bytes().as_slice())
        .bind(state_schema.as_bytes().as_slice())
        .bind(result_schema.as_bytes().as_slice())
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
    OperationValue::new(
        state_digest,
        state_schema,
        u32::try_from(state_bytes.len()).expect("state size"),
    )
    .expect("state reference")
}

async fn seed_state(
    w: &crate::support::engine::World,
    version: WorkflowVersionId,
    run: RunId,
    operation: &str,
    kind: &str,
) -> OperationValue {
    let state_bytes = br#"{"balance":10}"#;
    let state_digest = digest(state_bytes);
    let state_schema = Digest::new([201; 32]);
    let result_schema = Digest::new([202; 32]);
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
    .bind(operation)
    .bind(Digest::new([200; 32]).as_bytes().as_slice())
    .bind(state_schema.as_bytes().as_slice())
    .bind(result_schema.as_bytes().as_slice())
    .bind(crate::support::now())
    .execute(&w.db.pool)
    .await
    .expect("operation declaration");
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
         VALUES ($1,$2,$3,$4,$5,$6,0,$7,$8) ON CONFLICT DO NOTHING",
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
    OperationValue::new(
        state_digest,
        state_schema,
        u32::try_from(state_bytes.len()).expect("state size"),
    )
    .expect("state reference")
}

fn scope(w: &crate::support::engine::World, run: RunId) -> OperationScope {
    OperationScope::new(
        w.admin.tenant_id(),
        w.namespace.id(),
        OperationAddress::Run(run),
    )
}

fn audit(w: &crate::support::engine::World, run: RunId, action: AuditAction) -> AuditRecord {
    AuditRecord::by(
        &w.admin,
        action,
        ResourceType::Run,
        run.to_string(),
        Some("0af7651916cd43dd8448eb211c80319c".into()),
        crate::support::now(),
    )
}

async fn counts(pool: &sqlx::PgPool, tenant: Uuid, run: RunId) -> (i64, i64, i64) {
    let journal =
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2")
            .bind(tenant)
            .bind(run.as_uuid())
            .fetch_one(pool)
            .await
            .expect("journal count");
    let steps = sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2")
        .bind(tenant)
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("step count");
    let outbox = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("outbox count");
    (journal, steps, outbox)
}

#[tokio::test]
async fn query_returns_one_committed_snapshot_and_only_adds_access_audit() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("operation-query")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let state = seed_state(&w, version, run, "balance.read", "QUERY").await;
    let before = counts(&w.db.pool, w.admin.tenant_id().as_uuid(), run).await;
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let query = DeclaredQuery::new(
        scope(&w, run),
        run,
        ActivityName::parse("balance.read").expect("operation"),
        w.admin.subject().clone(),
    );

    let snapshot = store
        .query_snapshot(&query, &audit(&w, run, AuditAction::RunQueried))
        .await
        .expect("query snapshot");

    assert_eq!(snapshot.run_id(), run);
    assert_eq!(snapshot.state_revision(), 0);
    assert_eq!(snapshot.state(), &state);
    assert_eq!(
        counts(&w.db.pool, w.admin.tenant_id().as_uuid(), run).await,
        before
    );
    let access: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.queried' AND resource_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("query audit");
    assert_eq!(access, 1);
}

#[tokio::test]
async fn missing_declaration_fails_closed_without_an_audit_or_value() {
    let w = world().await;
    let (workflow, _) = w.publish(chain("operation-missing")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let query = DeclaredQuery::new(
        scope(&w, run),
        run,
        ActivityName::parse("missing.read").expect("operation"),
        w.admin.subject().clone(),
    );

    assert_eq!(
        store
            .query_snapshot(&query, &audit(&w, run, AuditAction::RunQueried))
            .await,
        Err(DeclaredOperationStoreError::DefinitionUnavailable)
    );
    let access: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND action='run.queried' AND resource_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("query audit");
    assert_eq!(access, 0);
}

#[tokio::test]
async fn revision_rejection_is_durable_replays_once_and_checks_current_authorization() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("operation-rejection")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    seed_state(&w, version, run, "balance.credit", "UPDATE").await;
    let tenant = w.admin.tenant_id().as_uuid();
    let original_run_version: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("run version");
    let request = UpdateRequest::new(
        scope(&w, run),
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("credit-rejected").expect("key"),
        Digest::new([7; 32]),
        Some(9),
        w.admin.subject().clone(),
    );
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let command = DeclaredUpdate::new(
        request.clone(),
        operation_arguments(b"{}"),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );

    let rejected = store
        .apply_update(
            &command,
            |_, _| Ok(()),
            |_| -> Result<DeclaredUpdateDecision, OperationError> {
                panic!("revision rejects before evaluation")
            },
        )
        .await
        .expect("durable rejection");
    assert_eq!(rejected.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(
        i64::try_from(rejected.receipt().run_version()).expect("run version"),
        original_run_version
    );

    sqlx::query("UPDATE runs SET status='FAILED', finished_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
        .bind(tenant)
        .bind(run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("terminal run");
    let replay_command = DeclaredUpdate::new(
        request.clone(),
        operation_arguments(b"{}"),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let replay = store
        .apply_update(
            &replay_command,
            |_, _| -> Result<(), OperationError> {
                panic!("receipt replay does not validate arguments")
            },
            |_| -> Result<DeclaredUpdateDecision, OperationError> {
                panic!("receipt replay does not evaluate")
            },
        )
        .await
        .expect("receipt replay");
    assert!(replay.replayed());
    assert_eq!(replay.receipt(), rejected.receipt());
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operation_update_receipts WHERE tenant_id=$1 AND resolved_run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("receipt count");
    assert_eq!(receipts, 1);

    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(tenant)
        .bind(w.admin.subject().as_str())
        .execute(&w.db.pool)
        .await
        .expect("revoke update permission");
    assert_eq!(
        store
            .apply_update(
                &replay_command,
                |_, _| -> Result<(), OperationError> {
                    panic!("revoked replay does not validate arguments")
                },
                |_| -> Result<DeclaredUpdateDecision, OperationError> {
                    panic!("revoked replay does not evaluate")
                },
            )
            .await,
        Err(DeclaredOperationStoreError::Forbidden)
    );
}

#[tokio::test]
async fn applied_update_commits_state_receipt_journal_audit_and_outbox_together() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("operation-applied-gate")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let state = seed_state(&w, version, run, "balance.credit", "UPDATE").await;
    let tenant = w.admin.tenant_id().as_uuid();
    let before = counts(&w.db.pool, tenant, run).await;
    let request = UpdateRequest::new(
        scope(&w, run),
        run,
        ActivityName::parse("balance.credit").expect("operation"),
        UpdateKey::parse("credit-applied").expect("key"),
        Digest::new([8; 32]),
        Some(0),
        w.admin.subject().clone(),
    );
    let command = DeclaredUpdate::new(
        request,
        operation_arguments(b"{}"),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let state_write =
        EvaluatedOperationWrite::restore(state.clone(), br#"{"balance":10}"#.to_vec())
            .expect("evaluated state");
    let result_bytes = b"accepted";
    let result = EvaluatedOperationWrite::restore(
        OperationValue::new(
            digest(result_bytes),
            Digest::new([202; 32]),
            u32::try_from(result_bytes.len()).expect("result size"),
        )
        .expect("result"),
        result_bytes.to_vec(),
    )
    .expect("evaluated result");

    let resolution = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(
            &command,
            |_, _| Ok(()),
            |_| {
                Ok(DeclaredUpdateDecision::Applied {
                    new_state: state_write,
                    result,
                })
            },
        )
        .await
        .expect("applied update");
    assert_eq!(resolution.receipt().status(), UpdateReceiptStatus::Applied);
    let after = counts(&w.db.pool, tenant, run).await;
    assert_eq!(after.0, before.0 + 1);
    assert_eq!(after.1, before.1);
    assert_eq!(after.2, before.2 + 1);
    let projection: (i64, i64, Vec<u8>) = sqlx::query_as(
        "SELECT state_revision, last_changed_run_version, state_digest \
         FROM workflow_operation_states WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("state projection");
    assert_eq!(projection.0, 1);
    assert_eq!(projection.1, resolution.receipt().run_version() as i64);
    assert_eq!(projection.2, state.digest().as_bytes());
    let event: (String, serde_json::Value, Vec<u8>) = sqlx::query_as(
        "SELECT event_type, detail, payload_digest FROM run_events \
         WHERE tenant_id=$1 AND run_id=$2 AND seq=$3",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(projection.1)
    .fetch_one(&w.db.pool)
    .await
    .expect("state journal event");
    assert_eq!(event.0, "state.updated");
    assert_eq!(event.1["operation"], "balance.credit");
    assert_eq!(event.1["before_revision"], 0);
    assert_eq!(event.1["after_revision"], 1);
    assert_eq!(event.2, state.digest().as_bytes());
    let outbox_payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM event_outbox WHERE id=$1 AND tenant_id=$2")
            .bind(command.outbox_id())
            .bind(tenant)
            .fetch_one(&w.db.pool)
            .await
            .expect("state outbox");
    assert_eq!(outbox_payload["operation"], "balance.credit");
    assert!(outbox_payload.get("state").is_none());
    assert!(outbox_payload.get("stateDigest").is_none());
    let audit_reason: String = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE tenant_id=$1 AND action='run.updated' \
         AND resource_id=$2",
    )
    .bind(tenant)
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("update audit");
    assert_eq!(audit_reason, "operation=balance.credit;outcome=applied");
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operation_update_receipts WHERE tenant_id=$1 AND resolved_run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("receipt count");
    assert_eq!(receipts, 1);

    let replay = DeclaredUpdate::new(
        command.request().clone(),
        command.arguments().clone(),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let replayed = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(
            &replay,
            |_, _| -> Result<(), OperationError> {
                panic!("receipt replay does not validate arguments")
            },
            |_| -> Result<DeclaredUpdateDecision, OperationError> {
                panic!("receipt replay must not evaluate")
            },
        )
        .await
        .expect("applied receipt replay");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), resolution.receipt());
    assert_eq!(counts(&w.db.pool, tenant, run).await, after);

    let divergent_digest = Digest::new([91; 32]);
    sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3)")
        .bind(tenant)
        .bind(divergent_digest.as_bytes().as_slice())
        .bind(b"tamper".as_slice())
        .execute(&w.db.pool)
        .await
        .expect("divergent payload");
    sqlx::query(
        "UPDATE workflow_operation_states SET state_digest=$3, state_size_bytes=6 \
         WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(divergent_digest.as_bytes().as_slice())
    .execute(&w.db.pool)
    .await
    .expect("diverge projection");
    let rejected = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            ActivityName::parse("balance.credit").expect("operation"),
            UpdateKey::parse("divergent-projection").expect("key"),
            Digest::new([92; 32]),
            Some(1),
            w.admin.subject().clone(),
        ),
        operation_arguments(b"{}"),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_eq!(
        PgDeclaredOperationStore::new(w.db.pool.clone())
            .apply_update(
                &rejected,
                |_, _| Ok(()),
                |_| -> Result<DeclaredUpdateDecision, OperationError> {
                    panic!("divergent state must not evaluate")
                },
            )
            .await,
        Err(DeclaredOperationStoreError::DefinitionUnavailable)
    );
}

#[tokio::test]
async fn outbox_failure_rolls_back_state_journal_receipt_and_audit() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("operation-rollback")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    seed_state(&w, version, run, "balance.debit", "UPDATE").await;
    let tenant = w.admin.tenant_id().as_uuid();
    let outbox_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO event_outbox (id, tenant_id, aggregate_id, event_type, payload) \
         VALUES ($1,$2,$3,'collision',$4)",
    )
    .bind(outbox_id)
    .bind(tenant)
    .bind(run.as_uuid())
    .bind(serde_json::json!({}))
    .execute(&w.db.pool)
    .await
    .expect("outbox collision seed");
    let state_bytes = br#"{"balance":9}"#;
    let result_bytes = b"ok";
    let new_state = EvaluatedOperationWrite::restore(
        OperationValue::new(
            digest(state_bytes),
            Digest::new([201; 32]),
            u32::try_from(state_bytes.len()).expect("state size"),
        )
        .expect("state value"),
        state_bytes.to_vec(),
    )
    .expect("evaluated state");
    let result = EvaluatedOperationWrite::restore(
        OperationValue::new(
            digest(result_bytes),
            Digest::new([202; 32]),
            u32::try_from(result_bytes.len()).expect("result size"),
        )
        .expect("result value"),
        result_bytes.to_vec(),
    )
    .expect("evaluated result");
    let uncommitted_digests = [new_state.value().digest(), result.value().digest()];
    let before = counts(&w.db.pool, tenant, run).await;
    let run_version: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("run version");
    let command = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            ActivityName::parse("balance.debit").expect("operation"),
            UpdateKey::parse("debit-rollback").expect("key"),
            Digest::new([67; 32]),
            Some(0),
            w.admin.subject().clone(),
        ),
        operation_arguments(b"{}"),
        audit(&w, run, AuditAction::RunUpdated),
        outbox_id,
        crate::support::now(),
    );

    let error = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(
            &command,
            |_, _| Ok(()),
            |_| Ok(DeclaredUpdateDecision::Applied { new_state, result }),
        )
        .await
        .expect_err("duplicate outbox rolls back");
    assert!(matches!(error, DeclaredOperationStoreError::Storage(_)));
    assert_eq!(counts(&w.db.pool, tenant, run).await, before);
    let durable: (i64, i64, i64) = sqlx::query_as(
        "SELECT s.state_revision, r.run_version, \
                (SELECT count(*) FROM operation_update_receipts u \
                 WHERE u.tenant_id=$1 AND u.resolved_run_id=$2) \
         FROM workflow_operation_states s JOIN runs r \
           ON r.tenant_id=s.tenant_id AND r.id=s.run_id \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("durable state");
    assert_eq!(durable, (0, run_version, 0));
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_id=$2 \
         AND action='run.updated'",
    )
    .bind(tenant)
    .bind(run.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("audit count");
    assert_eq!(audits, 0);
    for digest in uncommitted_digests {
        let retained: i64 =
            sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id=$1 AND digest=$2")
                .bind(tenant)
                .bind(digest.as_bytes().as_slice())
                .fetch_one(&w.db.pool)
                .await
                .expect("payload count");
        assert_eq!(retained, 0);
    }
}

#[tokio::test]
async fn concurrent_expected_revision_updates_apply_once_and_reject_once() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("operation-concurrency")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    seed_state(&w, version, run, "balance.reserve", "UPDATE").await;
    let tenant = w.admin.tenant_id().as_uuid();
    let before = counts(&w.db.pool, tenant, run).await;
    let state_schema = Digest::new([201; 32]);
    let result_schema = Digest::new([202; 32]);
    let state_a_bytes = br#"{"reserved":11}"#;
    let state_b_bytes = br#"{"reserved":12}"#;
    let result_a_bytes = br#"{"accepted":11}"#;
    let result_b_bytes = br#"{"accepted":12}"#;
    let state_a = OperationValue::new(
        digest(state_a_bytes),
        state_schema,
        u32::try_from(state_a_bytes.len()).expect("state size"),
    )
    .expect("state A");
    let state_b = OperationValue::new(
        digest(state_b_bytes),
        state_schema,
        u32::try_from(state_b_bytes.len()).expect("state size"),
    )
    .expect("state B");
    let result_a = OperationValue::new(
        digest(result_a_bytes),
        result_schema,
        u32::try_from(result_a_bytes.len()).expect("result size"),
    )
    .expect("result A");
    let result_b = OperationValue::new(
        digest(result_b_bytes),
        result_schema,
        u32::try_from(result_b_bytes.len()).expect("result size"),
    )
    .expect("result B");
    for (value, bytes) in [
        (&state_a, state_a_bytes.as_slice()),
        (&state_b, state_b_bytes.as_slice()),
        (&result_a, result_a_bytes.as_slice()),
        (&result_b, result_b_bytes.as_slice()),
    ] {
        sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3)")
            .bind(tenant)
            .bind(value.digest().as_bytes().as_slice())
            .bind(bytes)
            .execute(&w.db.pool)
            .await
            .expect("operation payload");
    }
    let state_a =
        EvaluatedOperationWrite::restore(state_a, state_a_bytes.to_vec()).expect("state A bytes");
    let state_b =
        EvaluatedOperationWrite::restore(state_b, state_b_bytes.to_vec()).expect("state B bytes");
    let result_a = EvaluatedOperationWrite::restore(result_a, result_a_bytes.to_vec())
        .expect("result A bytes");
    let result_b = EvaluatedOperationWrite::restore(result_b, result_b_bytes.to_vec())
        .expect("result B bytes");
    let operation = ActivityName::parse("balance.reserve").expect("operation");
    let first = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            operation.clone(),
            UpdateKey::parse("reserve-a").expect("key"),
            Digest::new([71; 32]),
            Some(0),
            w.admin.subject().clone(),
        ),
        operation_arguments(br#"{"reserved":11}"#),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let second = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            operation,
            UpdateKey::parse("reserve-b").expect("key"),
            Digest::new([72; 32]),
            Some(0),
            w.admin.subject().clone(),
        ),
        operation_arguments(br#"{"reserved":12}"#),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let (first_result, second_result) = tokio::join!(
        store.apply_update(
            &first,
            |_, _| Ok(()),
            move |_| {
                Ok(DeclaredUpdateDecision::Applied {
                    new_state: state_a,
                    result: result_a,
                })
            }
        ),
        store.apply_update(
            &second,
            |_, _| Ok(()),
            move |_| {
                Ok(DeclaredUpdateDecision::Applied {
                    new_state: state_b,
                    result: result_b,
                })
            }
        ),
    );
    let statuses = [
        first_result.expect("first update").receipt().status(),
        second_result.expect("second update").receipt().status(),
    ];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == UpdateReceiptStatus::Applied)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == UpdateReceiptStatus::Rejected)
            .count(),
        1
    );
    let after = counts(&w.db.pool, tenant, run).await;
    assert_eq!(after.0, before.0 + 1);
    assert_eq!(after.1, before.1);
    assert_eq!(after.2, before.2 + 1);
    let durable: (i64, i64, i64) = sqlx::query_as(
        "SELECT state_revision, \
                (SELECT count(*) FROM operation_update_receipts r \
                 WHERE r.tenant_id=$1 AND r.resolved_run_id=$2), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 \
                 AND e.event_type='state.updated') \
         FROM workflow_operation_states WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant)
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("serialized outcomes");
    assert_eq!(durable, (1, 2, 1));
}

#[tokio::test]
async fn evaluator_applies_and_retains_schema_checked_state_and_result_in_one_transaction() {
    let w = world().await;
    let document = evaluated_operations_document("input.offset + state.balance + args");
    let declared = JsonIrCodec
        .parse_declared_state(document.to_string().as_bytes())
        .expect("declared state");
    let (workflow, version) = w.publish(chain("operation-evaluator-store")).await;
    let run = w.start(workflow, &json!({"offset": 1})).await;
    seed_evaluated_state(&w, version, run, &declared).await;
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let query_name = ActivityName::parse("balance.read").expect("query name");
    let query = DeclaredQuery::new(
        scope(&w, run),
        run,
        query_name.clone(),
        w.admin.subject().clone(),
    );
    let snapshot = store
        .query_snapshot(&query, &audit(&w, run, AuditAction::RunQueried))
        .await
        .expect("query snapshot");
    let mut substituted_document = document.clone();
    substituted_document["state"]["queries"]["balance.read"]["expression"] = json!("state.balance");
    let substituted = JsonIrCodec
        .parse_declared_state(substituted_document.to_string().as_bytes())
        .expect("substituted declaration");
    assert_eq!(
        JsonDeclaredOperationEvaluator.query_snapshot(
            substituted.state(),
            &query_name,
            &snapshot,
            b"3",
        ),
        Err(OperationEvaluationError::DeclarationMismatch)
    );
    let evaluated = JsonDeclaredOperationEvaluator
        .query_snapshot(declared.state(), &query_name, &snapshot, b"3")
        .expect("evaluated query");
    assert_eq!(evaluated.result().content(), b"14");
    let response = evaluated
        .response(snapshot.snapshot().clone())
        .expect("bound query response");
    assert_eq!(response.result(), evaluated.result().value());

    let arguments = br#"{"allowed":true,"nextState":{"balance":20}}"#;
    let update_name = ActivityName::parse("balance.replace").expect("update name");
    let command = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            update_name.clone(),
            UpdateKey::parse("replace-evaluated").expect("update key"),
            digest(arguments),
            Some(0),
            w.admin.subject().clone(),
        ),
        operation_arguments(arguments),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let evaluated_arguments = command.arguments().clone();
    let resolution = store
        .apply_update(
            &command,
            |snapshot, arguments| {
                JsonDeclaredOperationEvaluator
                    .validate_update_arguments(declared.state(), &update_name, snapshot, arguments)
                    .map_err(|_| OperationError::EvaluationFailed)
            },
            |snapshot| {
                JsonDeclaredOperationEvaluator
                    .update_snapshot(
                        declared.state(),
                        &update_name,
                        snapshot,
                        &evaluated_arguments,
                    )
                    .and_then(|plan| plan.store_decision(snapshot))
                    .map_err(|_| OperationError::EvaluationFailed)
            },
        )
        .await
        .expect("evaluated update");
    assert_eq!(resolution.receipt().status(), UpdateReceiptStatus::Applied);
    let result = resolution.receipt().result().expect("update result");
    let retained_result: Vec<u8> =
        sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id=$1 AND digest=$2")
            .bind(w.admin.tenant_id().as_uuid())
            .bind(result.digest().as_bytes().as_slice())
            .fetch_one(&w.db.pool)
            .await
            .expect("retained result");
    assert_eq!(retained_result, b"20");
    let retained_state: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM workflow_operation_states s \
         JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.state_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("retained state");
    assert_eq!(retained_state, br#"{"balance":20}"#);

    let reordered_replay = DeclaredUpdate::new(
        command.request().clone(),
        operation_arguments(br#"{"nextState":{"balance":20},"allowed":true}"#),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_eq!(
        reordered_replay.request().request_hash(),
        command.request().request_hash()
    );
    let replayed = store
        .apply_update(
            &reordered_replay,
            |_, _| -> Result<(), OperationError> {
                panic!("receipt replay does not validate arguments")
            },
            |_| -> Result<DeclaredUpdateDecision, OperationError> {
                panic!("receipt replay does not evaluate")
            },
        )
        .await
        .expect("canonical replay");
    assert!(replayed.replayed());
    assert_eq!(replayed.receipt(), resolution.receipt());

    let changed_arguments = DeclaredUpdate::new(
        command.request().clone(),
        operation_arguments(br#"{"allowed":true,"nextState":{"balance":21}}"#),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_ne!(
        changed_arguments.request().request_hash(),
        command.request().request_hash()
    );
    assert_eq!(
        store
            .apply_update(
                &changed_arguments,
                |_, _| -> Result<(), OperationError> {
                    panic!("same-key collision does not validate arguments")
                },
                |_| -> Result<DeclaredUpdateDecision, OperationError> {
                    panic!("same-key collision does not evaluate")
                },
            )
            .await,
        Err(DeclaredOperationStoreError::Operation(
            OperationError::UpdateKeyReused
        ))
    );

    let invalid_arguments = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            update_name.clone(),
            UpdateKey::parse("invalid-before-revision").expect("update key"),
            Digest::new([0; 32]),
            Some(99),
            w.admin.subject().clone(),
        ),
        operation_arguments(br#"{"allowed":true,"nextState":"invalid"}"#),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_eq!(
        store
            .apply_update(
                &invalid_arguments,
                |snapshot, arguments| {
                    JsonDeclaredOperationEvaluator
                        .validate_update_arguments(
                            declared.state(),
                            &update_name,
                            snapshot,
                            arguments,
                        )
                        .map_err(|_| OperationError::EvaluationFailed)
                },
                |_| -> Result<DeclaredUpdateDecision, OperationError> {
                    panic!("invalid arguments fail before revision and evaluation")
                },
            )
            .await,
        Err(DeclaredOperationStoreError::Operation(
            OperationError::EvaluationFailed
        ))
    );
    let invalid_receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operation_update_receipts \
         WHERE tenant_id=$1 AND resolved_run_id=$2 AND update_key='invalid-before-revision'",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("invalid receipt count");
    assert_eq!(invalid_receipts, 0);

    let rejected_arguments =
        operation_arguments(br#"{"allowed":false,"nextState":{"balance":999}}"#);
    let rejected_command = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            update_name.clone(),
            UpdateKey::parse("validator-rejected").expect("update key"),
            Digest::new([0; 32]),
            Some(1),
            w.admin.subject().clone(),
        ),
        rejected_arguments.clone(),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let rejected = store
        .apply_update(
            &rejected_command,
            |snapshot, arguments| {
                JsonDeclaredOperationEvaluator
                    .validate_update_arguments(declared.state(), &update_name, snapshot, arguments)
                    .map_err(|_| OperationError::EvaluationFailed)
            },
            |snapshot| {
                JsonDeclaredOperationEvaluator
                    .update_snapshot(
                        declared.state(),
                        &update_name,
                        snapshot,
                        &rejected_arguments,
                    )
                    .and_then(|plan| plan.store_decision(snapshot))
                    .map_err(|_| OperationError::EvaluationFailed)
            },
        )
        .await
        .expect("validator rejection");
    assert_eq!(rejected.receipt().status(), UpdateReceiptStatus::Rejected);
    assert_eq!(rejected.receipt().before_state_revision(), 1);
    assert_eq!(rejected.receipt().after_state_revision(), 1);

    let updated_snapshot = store
        .query_snapshot(&query, &audit(&w, run, AuditAction::RunQueried))
        .await
        .expect("updated query snapshot");
    let updated = JsonDeclaredOperationEvaluator
        .query_snapshot(declared.state(), &query_name, &updated_snapshot, b"3")
        .expect("updated query");
    assert_eq!(updated.result().content(), b"24");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_completion_serializes_after_the_snapshot_used_by_an_update() {
    let w = world().await;
    let document = evaluated_operations_document("completedResults.a.amount");
    let declared = JsonIrCodec
        .parse_declared_state(document.to_string().as_bytes())
        .expect("declared state");
    let (workflow, version) = w.publish(chain("operation-activity-race")).await;
    let run = w.start(workflow, &json!({"offset": 1})).await;
    let session = w.session(&["act.a", "act.b"]).await;
    let activity = w.one_task(&session).await;
    seed_evaluated_state(&w, version, run, &declared).await;

    let arguments = br#"{"allowed":true,"nextState":{"balance":20}}"#.to_vec();
    let update_name = ActivityName::parse("balance.replace").expect("update name");
    let command = DeclaredUpdate::new(
        UpdateRequest::new(
            scope(&w, run),
            run,
            update_name.clone(),
            UpdateKey::parse("replace-before-completion").expect("update key"),
            digest(&arguments),
            Some(0),
            w.admin.subject().clone(),
        ),
        operation_arguments(&arguments),
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let state_definition = declared.state().clone();
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let update_task = tokio::spawn(async move {
        let evaluated_arguments = command.arguments().clone();
        let validation_state = state_definition.clone();
        let validation_name = update_name.clone();
        store
            .apply_update(
                &command,
                move |snapshot, arguments| {
                    JsonDeclaredOperationEvaluator
                        .validate_update_arguments(
                            &validation_state,
                            &validation_name,
                            snapshot,
                            arguments,
                        )
                        .map_err(|_| OperationError::EvaluationFailed)
                },
                move |snapshot| {
                    entered_tx
                        .send((snapshot.run_version(), snapshot.completed_results().count()))
                        .map_err(|_| OperationError::EvaluationFailed)?;
                    release_rx
                        .recv()
                        .map_err(|_| OperationError::EvaluationFailed)?;
                    JsonDeclaredOperationEvaluator
                        .update_snapshot(
                            &state_definition,
                            &update_name,
                            snapshot,
                            &evaluated_arguments,
                        )
                        .and_then(|plan| plan.store_decision(snapshot))
                        .map_err(|_| OperationError::EvaluationFailed)
                },
            )
            .await
    });
    let (observed_run_version, completed_count) =
        entered_rx.await.expect("update reached locked snapshot");
    assert_eq!(completed_count, 0);
    let release = async move {
        tokio::task::yield_now().await;
        release_tx.send(()).expect("release evaluator");
    };
    let activity_output = json!({"amount": 7});
    let (completion, ()) = tokio::join!(w.complete(&session, &activity, &activity_output), release);
    completion.expect("activity completion");
    let resolution = update_task
        .await
        .expect("update task")
        .expect("serialized update");
    assert_eq!(resolution.receipt().status(), UpdateReceiptStatus::Applied);
    assert_eq!(resolution.receipt().run_version(), observed_run_version + 1);

    let query_name = ActivityName::parse("balance.read").expect("query name");
    let query = DeclaredQuery::new(
        scope(&w, run),
        run,
        query_name.clone(),
        w.admin.subject().clone(),
    );
    let snapshot = PgDeclaredOperationStore::new(w.db.pool.clone())
        .query_snapshot(&query, &audit(&w, run, AuditAction::RunQueried))
        .await
        .expect("post-completion snapshot");
    assert!(snapshot.run_version() > resolution.receipt().run_version());
    assert_eq!(snapshot.completed_results().count(), 1);
    let evaluated = JsonDeclaredOperationEvaluator
        .query_snapshot(declared.state(), &query_name, &snapshot, b"0")
        .expect("query completed result");
    assert_eq!(evaluated.result().content(), b"7");
}
