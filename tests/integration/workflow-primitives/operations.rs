//! Real PostgreSQL evidence for declared Query snapshots and durable Update receipts.

use sagat_application::declared_operations::{
    DeclaredOperationStore, DeclaredOperationStoreError, DeclaredQuery, DeclaredUpdate,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowVersionId;
use sagat_domain::execution::{
    OperationAddress, OperationScope, OperationValue, RunId, UpdateEvaluation, UpdateKey,
    UpdateReceiptStatus, UpdateRequest,
};
use sagat_domain::workflow::{ActivityName, Digest};
use sagat_infra_postgres::PgDeclaredOperationStore;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::engine::{chain, world};

fn digest(bytes: &[u8]) -> Digest {
    Digest::new(Sha256::digest(bytes).into())
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
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );

    let rejected = store
        .apply_update(&command, |_| panic!("revision rejects before evaluation"))
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
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let replay = store
        .apply_update(&replay_command, |_| {
            panic!("receipt replay does not evaluate")
        })
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
            .apply_update(&replay_command, |_| panic!(
                "revoked replay does not evaluate"
            ))
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
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let result =
        OperationValue::new(Digest::new([33; 32]), Digest::new([202; 32]), 8).expect("result");
    sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3)")
        .bind(tenant)
        .bind(result.digest().as_bytes().as_slice())
        .bind(b"accepted".as_slice())
        .execute(&w.db.pool)
        .await
        .expect("result payload");

    let resolution = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(&command, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: state.clone(),
                result,
            })
        })
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
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let replayed = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(&replay, |_| panic!("receipt replay must not evaluate"))
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
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    assert_eq!(
        PgDeclaredOperationStore::new(w.db.pool.clone())
            .apply_update(&rejected, |_| panic!("divergent state must not evaluate"))
            .await,
        Err(DeclaredOperationStoreError::DefinitionUnavailable)
    );
}

#[tokio::test]
async fn outbox_failure_rolls_back_state_journal_receipt_and_audit() {
    let w = world().await;
    let (workflow, version) = w.publish(chain("operation-rollback")).await;
    let run = w.start(workflow, &serde_json::json!({})).await;
    let state = seed_state(&w, version, run, "balance.debit", "UPDATE").await;
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
    let result =
        OperationValue::new(Digest::new([66; 32]), Digest::new([202; 32]), 2).expect("result");
    sqlx::query("INSERT INTO payloads (tenant_id, digest, content) VALUES ($1,$2,$3)")
        .bind(tenant)
        .bind(result.digest().as_bytes().as_slice())
        .bind(b"ok".as_slice())
        .execute(&w.db.pool)
        .await
        .expect("result payload");
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
        audit(&w, run, AuditAction::RunUpdated),
        outbox_id,
        crate::support::now(),
    );

    let error = PgDeclaredOperationStore::new(w.db.pool.clone())
        .apply_update(&command, |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: state,
                result,
            })
        })
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
        audit(&w, run, AuditAction::RunUpdated),
        Uuid::now_v7(),
        crate::support::now(),
    );
    let store = PgDeclaredOperationStore::new(w.db.pool.clone());
    let (first_result, second_result) = tokio::join!(
        store.apply_update(&first, move |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: state_a,
                result: result_a,
            })
        }),
        store.apply_update(&second, move |_| {
            Ok(UpdateEvaluation::Applied {
                new_state: state_b,
                result: result_b,
            })
        }),
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
