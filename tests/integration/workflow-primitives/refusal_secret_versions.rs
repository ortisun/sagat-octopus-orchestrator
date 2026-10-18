//! A scoped successful-output refusal must revalidate the exact provider-backed secret versions
//! scanned by the bounded path when it restores the full run transaction.

use std::{sync::Arc, time::Duration};

use reqwest::Method;
use sagat_application::{
    entropy::Token, execution::DefinitionCache, secret_provider::SecretProvider,
};
use sagat_domain::execution::RunId;
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::PgRunStore;
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use serde_json::{Value, json};
use testcontainers_modules::testcontainers::{
    GenericImage, ImageExt, core::WaitFor, runners::AsyncRunner,
};
use uuid::Uuid;

use crate::support::{
    api::Options,
    blob_collections::{scheduler, scoped_worker},
    scope_flow::{activity, collection_document, retry},
    worker::{Platform, machine_code, platform_with},
};

const SECRET_NAME: &str = "scoped-key";
const ACTIVITY: &str = "line.reserve";

fn refused_member_output() -> Value {
    let output = Value::String("x".repeat(249_998));
    assert!(
        output.to_string().len() == 250_000,
        "the refusal candidate must have the intended encoded byte length"
    );
    output
}

fn refusal_document(name: &str) -> Value {
    let reserve = activity(
        "reserve",
        ACTIVITY,
        "returned",
        json!({
            "secrets": [SECRET_NAME],
            "compensateWith": "line.release",
            "compensationRetry": retry()
        }),
    );
    let mut document = collection_document(
        name,
        vec![
            reserve,
            json!({"id": "returned", "type": "return", "result": "1"}),
        ],
        1,
    );
    document["nodes"][0]["maxItems"] = json!(17);
    document
}

async fn drive(scheduler: &crate::support::blob_collections::Sched) {
    for _ in 0..80 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "a scheduler action failed");
        if !report.did_work() {
            return;
        }
    }
    panic!("the scheduler did not become idle within its bounded drive");
}

async fn put_secret(
    platform: &Platform,
    namespace: &str,
    value: &str,
    expected_version: u32,
    request_id: Uuid,
) -> Value {
    let (status, metadata) = platform
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/secrets/{SECRET_NAME}"),
            Some(&json!({
                "value": value,
                "requiredProvider": "openbao",
                "expectedVersion": expected_version,
                "requestId": request_id.to_string()
            })),
        )
        .await;
    assert_eq!(status, 200, "managed secret write failed");
    assert_eq!(metadata["provider"], "openbao");
    assert_eq!(metadata["version"], expected_version + 1);
    assert!(
        !metadata.to_string().contains(value),
        "managed secret metadata must not contain its submitted value"
    );
    metadata
}

async fn secret_metadata(platform: &Platform, namespace: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'authority',secret.authority, \
            'currentVersion',secret.current_version, \
            'versions',jsonb_agg(jsonb_build_object( \
                'id',version.id,'version',version.version, \
                'providerOperation',version.provider_operation, \
                'providerActive',operation.state='active', \
                'hasLocalCiphertext',version.ciphertext IS NOT NULL) \
              ORDER BY version.version)) \
           FROM secrets secret \
           JOIN secret_versions version \
             ON version.tenant_id=secret.tenant_id AND version.secret_id=secret.id \
           JOIN secret_provider_operations operation \
             ON operation.tenant_id=version.tenant_id \
            AND operation.id=version.provider_operation \
          WHERE secret.tenant_id=$1 AND secret.namespace_id=$2 AND secret.name=$3 \
          GROUP BY secret.authority,secret.current_version",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .bind(SECRET_NAME)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("provider-backed secret metadata")
}

async fn secret_catalog_snapshot(platform: &Platform, namespace: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'secret',to_jsonb(secret), \
            'versions',(SELECT jsonb_agg(to_jsonb(version_row) ORDER BY version_row.version) \
              FROM secret_versions version_row \
              WHERE version_row.tenant_id=secret.tenant_id AND version_row.secret_id=secret.id), \
            'operations',(SELECT jsonb_agg(to_jsonb(operation) ORDER BY operation.created_at,operation.id) \
              FROM secret_provider_operations operation \
              WHERE operation.tenant_id=secret.tenant_id AND operation.secret_id=secret.id)) \
           FROM secrets secret \
          WHERE secret.tenant_id=$1 AND secret.namespace_id=$2 AND secret.name=$3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .bind(SECRET_NAME)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("complete provider-backed secret catalog")
}

fn without_verified_rotation(catalog: &Value, before: &Value, operation: Uuid) -> Value {
    let mut normalized = catalog.clone();
    normalized["secret"]["current_version"] = before["secret"]["current_version"].clone();
    normalized["secret"]["updated_at"] = before["secret"]["updated_at"].clone();
    let operation = operation.to_string();
    normalized["versions"]
        .as_array_mut()
        .expect("catalog versions")
        .retain(|version| version["provider_operation"].as_str() != Some(operation.as_str()));
    normalized["operations"]
        .as_array_mut()
        .expect("catalog operations")
        .retain(|intent| intent["id"].as_str() != Some(operation.as_str()));
    normalized
}

async fn verified_rotation_audits(
    platform: &Platform,
    namespace: Uuid,
    operation: Uuid,
) -> Vec<Uuid> {
    let resource = format!("{namespace}/{SECRET_NAME}");
    let reason = format!("openbao operation {operation}");
    let rows: Vec<(Uuid, String, String, String, String)> = sqlx::query_as(
        "SELECT id,action,resource_type,resource_id,actor FROM audit_records \
          WHERE tenant_id=$1 AND reason=$2 ORDER BY action,id",
    )
    .bind(platform.tenant.as_uuid())
    .bind(reason)
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("native rotation audits");
    assert!(
        rows.len() == 2,
        "the rotation must append exactly two native secret audits"
    );
    assert!(
        rows.iter()
            .any(|row| row.1 == "secret.provisioning_started")
            && rows.iter().any(|row| row.1 == "secret.stored"),
        "the rotation must record its prepare and activation audits"
    );
    assert!(
        rows.iter()
            .all(|row| { row.2 == "secret" && row.3 == resource && row.4 == "alice" }),
        "the rotation audits must identify only the managed secret operation"
    );
    rows.into_iter().map(|row| row.0).collect()
}

async fn retained_usage(platform: &Platform) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT content_bytes,content_reserved_bytes,log_bytes \
           FROM tenant_retained_usage WHERE tenant_id=$1",
    )
    .bind(platform.tenant.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("tenant retained usage")
}

async fn secret_catalog_counts(platform: &Platform) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM secrets secret WHERE secret.tenant_id=$1), \
           (SELECT count(*) FROM secret_versions version WHERE version.tenant_id=$1), \
           (SELECT count(*) FROM secret_provider_operations operation WHERE operation.tenant_id=$1)",
    )
    .bind(platform.tenant.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("tenant secret catalog counts")
}

async fn assert_exact_rotation_catalog(
    platform: &Platform,
    namespace: Uuid,
    request: Uuid,
    operation: Uuid,
    value_bytes: usize,
    counts_before: (i64, i64, i64),
    catalogs: (&Value, &Value),
) {
    let (before, after) = catalogs;
    assert!(
        after["secret"]["current_version"] == 2,
        "the managed secret must advance to version two"
    );
    assert!(
        after["secret"]["updated_at"] != before["secret"]["updated_at"],
        "the managed secret catalog timestamp must advance"
    );
    assert!(
        after["versions"]
            .as_array()
            .expect("catalog versions")
            .len()
            == before["versions"]
                .as_array()
                .expect("catalog versions")
                .len()
                + 1,
        "the rotation must append exactly one version row"
    );
    assert!(
        after["operations"]
            .as_array()
            .expect("catalog operations")
            .len()
            == before["operations"]
                .as_array()
                .expect("catalog operations")
                .len()
                + 1,
        "the rotation must append exactly one provider operation"
    );
    let preexisting_catalog = without_verified_rotation(after, before, operation);
    assert!(
        preexisting_catalog.eq(before),
        "version one and every preexisting secret catalog row must remain unchanged"
    );
    let value_bytes = i64::try_from(value_bytes).expect("bounded secret bytes");
    let exact: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM secret_provider_operations candidate \
             WHERE candidate.tenant_id=$1 AND candidate.namespace_id=$2 \
               AND candidate.id=$3 AND candidate.secret_id=secret.id \
               AND candidate.name=$4 AND candidate.actor='alice' \
               AND candidate.request_id=$5 AND candidate.expected_version=1 \
               AND candidate.value_bytes=$6 AND candidate.state='active' \
               AND candidate.activated_at IS NOT NULL \
               AND octet_length(candidate.binding)=32 \
               AND candidate.source_version IS NULL AND candidate.pools IS NULL), \
           (SELECT count(*) FROM secret_versions version \
             WHERE version.tenant_id=$1 AND version.secret_id=secret.id \
               AND version.id=$3 AND version.version=2 \
               AND version.provider_operation=$3 AND version.created_by='alice' \
               AND version.key_id IS NULL AND version.nonce IS NULL \
               AND version.ciphertext IS NULL), \
           (SELECT count(*) FROM secrets candidate \
             WHERE candidate.tenant_id=$1 AND candidate.namespace_id=$2 \
               AND candidate.id=secret.id AND candidate.name=$4 \
               AND candidate.authority='openbao' AND candidate.current_version=2) \
          FROM secrets secret \
         WHERE secret.tenant_id=$1 AND secret.namespace_id=$2 AND secret.name=$4",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .bind(operation)
    .bind(SECRET_NAME)
    .bind(request)
    .bind(value_bytes)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact native rotation catalog delta");
    assert!(
        exact.0 == 1 && exact.1 == 1 && exact.2 == 1,
        "version two must have one exact active provider operation and catalog row"
    );
    let counts_after = secret_catalog_counts(platform).await;
    assert!(
        counts_after.0 == counts_before.0
            && counts_after.1 == counts_before.1 + 1
            && counts_after.2 == counts_before.2 + 1,
        "the rotation must preserve the secret count and append exactly one version and provider operation"
    );
}

async fn refusal_snapshot(
    platform: &Platform,
    run: Uuid,
    omitted_audits: &[Uuid],
    retained_content_adjustment: i64,
) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'run',to_jsonb(run), \
            'scopes',(SELECT coalesce(jsonb_agg(to_jsonb(scope) ORDER BY scope.scope_id),'[]'::jsonb) \
              FROM iteration_scopes scope \
              WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id), \
            'iterations',(SELECT coalesce(jsonb_agg(to_jsonb(member) \
              ORDER BY member.scope_id,member.iteration_index),'[]'::jsonb) \
              FROM collection_iterations member \
              WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id), \
            'steps',(SELECT coalesce(jsonb_agg(to_jsonb(step) \
              ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node),'[]'::jsonb) \
              FROM steps step WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id), \
            'journal',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
                'row',to_jsonb(event),'payload',encode(payload.content,'hex')) \
              ORDER BY event.seq),'[]'::jsonb) \
              FROM run_events event LEFT JOIN payloads payload \
                ON payload.tenant_id=event.tenant_id AND payload.digest=event.payload_digest \
              WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id), \
            'usage',(SELECT to_jsonb(usage) FROM run_instance_usage usage \
              WHERE usage.tenant_id=run.tenant_id AND usage.run_id=run.id), \
            'telemetry',(SELECT to_jsonb(head) FROM telemetry_pending head \
              WHERE head.tenant_id=run.tenant_id AND head.run_id=run.id), \
            'tenantPayloads',(SELECT coalesce(jsonb_agg(to_jsonb(payload) \
              ORDER BY encode(payload.digest,'hex')),'[]'::jsonb) \
              FROM payloads payload WHERE payload.tenant_id=run.tenant_id), \
            'tenantRetainedUsage',(SELECT to_jsonb(retained) || jsonb_build_object( \
                'content_bytes',retained.content_bytes-$4) \
              FROM tenant_retained_usage retained WHERE retained.tenant_id=run.tenant_id), \
            'audits',(SELECT coalesce(jsonb_agg(to_jsonb(audit) ORDER BY audit.id),'[]'::jsonb) \
              FROM audit_records audit WHERE audit.tenant_id=run.tenant_id \
                AND audit.id<>ALL($3::uuid[])), \
            'receipts',(SELECT coalesce(jsonb_agg(to_jsonb(receipt) \
              ORDER BY receipt.scope,receipt.idempotency_key),'[]'::jsonb) \
              FROM command_dedupe receipt WHERE receipt.tenant_id=run.tenant_id), \
            'outbox',(SELECT coalesce(jsonb_agg(to_jsonb(message) ORDER BY message.id),'[]'::jsonb) \
              FROM event_outbox message WHERE message.tenant_id=run.tenant_id), \
            'sagas',(SELECT coalesce(jsonb_agg(to_jsonb(saga) \
              ORDER BY saga.owner_path_nodes,saga.owner_path_indices,saga.owner_node),'[]'::jsonb) \
              FROM scope_sagas saga WHERE saga.tenant_id=run.tenant_id AND saga.run_id=run.id), \
            'sagaEntries',(SELECT coalesce(jsonb_agg(to_jsonb(entry) \
              ORDER BY entry.owner_path_nodes,entry.owner_path_indices,entry.owner_node, \
                       entry.iteration_index,entry.node),'[]'::jsonb) \
              FROM scope_saga_entries entry \
              WHERE entry.tenant_id=run.tenant_id AND entry.run_id=run.id)) \
           FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .bind(omitted_audits)
    .bind(retained_content_adjustment)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("complete report transaction snapshot")
}

async fn waiting_full_checkpoint_restore(platform: &Platform, gate_pid: i32) -> i32 {
    crate::support::eventually(
        "the exact full-run checkpoint restoration query",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT activity.pid FROM pg_stat_activity activity \
                  WHERE activity.datname=current_database() \
                    AND activity.usename='sagat_app' \
                    AND activity.state='active' \
                    AND activity.wait_event_type='Lock' \
                    AND $1=ANY(pg_blocking_pids(activity.pid)) \
                    AND position('SELECT scope_id, iteration_index, return_instance_id, result_digest, journal_sequence' in activity.query)>0 \
                    AND position('FROM iteration_checkpoints WHERE tenant_id = $1 AND run_id = $2' in activity.query)>0 \
                    AND EXISTS(SELECT 1 FROM pg_locks lock \
                      WHERE lock.pid=activity.pid \
                        AND lock.relation='public.iteration_checkpoints'::regclass \
                        AND lock.mode='AccessShareLock' AND NOT lock.granted) \
                  LIMIT 1",
            )
            .bind(gate_pid)
            .fetch_optional(&platform.api.db.pool)
            .await
            .expect("observe full checkpoint restore wait")
        },
    )
    .await
}

async fn still_waiting_full_checkpoint_restore(
    platform: &Platform,
    gate_pid: i32,
    writer_pid: i32,
) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity \
          WHERE activity.pid=$1 AND activity.datname=current_database() \
            AND activity.usename='sagat_app' AND activity.state='active' \
            AND activity.wait_event_type='Lock' \
            AND $2=ANY(pg_blocking_pids(activity.pid)) \
            AND position('SELECT scope_id, iteration_index, return_instance_id, result_digest, journal_sequence' in activity.query)>0 \
            AND position('FROM iteration_checkpoints WHERE tenant_id = $1 AND run_id = $2' in activity.query)>0 \
            AND EXISTS(SELECT 1 FROM pg_locks lock \
              WHERE lock.pid=activity.pid \
                AND lock.relation='public.iteration_checkpoints'::regclass \
                AND lock.mode='AccessShareLock' AND NOT lock.granted))",
    )
    .bind(writer_pid)
    .bind(gate_pid)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("the same full checkpoint restore remains blocked")
}

async fn assert_no_refusal_effect(platform: &Platform, run: Uuid, key: &str, scope: &str) {
    let state: (String, Option<String>, String, bool, i64, i64) = sqlx::query_as(
        "SELECT run.status,run.stop_reason,step.status,step.output_digest IS NULL, \
                (SELECT count(*) FROM command_dedupe receipt \
                  WHERE receipt.tenant_id=run.tenant_id AND receipt.idempotency_key=$3 AND receipt.scope=$4), \
                (SELECT count(*) FROM audit_records audit \
                  WHERE audit.tenant_id=run.tenant_id \
                    AND audit.resource_type='run' AND audit.resource_id=run.id::text \
                    AND audit.action='run.data_limit_exceeded') \
           FROM runs run JOIN steps step \
             ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2 \
            AND step.instance_path_nodes=ARRAY['lines']::text[] \
            AND step.instance_path_indices=ARRAY[16]::integer[] \
            AND step.node='reserve'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .bind(key)
    .bind(scope)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("unchanged seventeenth member");
    assert_eq!(state.0, "RUNNING");
    assert!(state.1.is_none());
    assert_eq!(state.2, "LEASED");
    assert!(state.3);
    assert_eq!(state.4, 0);
    assert_eq!(state.5, 0);
}

async fn assert_one_truthful_refusal(platform: &Platform, run: Uuid, key: &str, scope: &str) {
    let state: (
        String,
        Option<String>,
        Option<String>,
        String,
        bool,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT run.status,run.stop_reason,run.stop_code,step.status, \
                    step.output_digest IS NULL,usage.logical_output_bytes, \
                    (SELECT count(*) FROM command_dedupe receipt \
                      WHERE receipt.tenant_id=run.tenant_id AND receipt.idempotency_key=$3 AND receipt.scope=$4), \
                    (SELECT count(*) FROM audit_records audit \
                      WHERE audit.tenant_id=run.tenant_id \
                        AND audit.resource_type='run' AND audit.resource_id=run.id::text \
                        AND audit.action='run.data_limit_exceeded' \
                        AND audit.reason='successful scoped output refused by run data limit'), \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.event_type='step.completed' AND event.node='reserve' \
                        AND event.instance_path_nodes=ARRAY['lines']::text[] \
                        AND event.instance_path_indices=ARRAY[16]::integer[]) \
               FROM runs run JOIN steps step \
                 ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
               JOIN run_instance_usage usage \
                 ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
              WHERE run.tenant_id=$1 AND run.id=$2 \
                AND step.instance_path_nodes=ARRAY['lines']::text[] \
                AND step.instance_path_indices=ARRAY[16]::integer[] \
                AND step.node='reserve'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .bind(key)
    .bind(scope)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("one truthful refusal");
    assert_eq!(state.0, "CANCELLING");
    assert_eq!(state.1.as_deref(), Some("FAILED"));
    assert_eq!(state.2.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(state.3, "SUCCEEDED");
    assert!(state.4);
    assert_eq!(state.5, 4_000_000);
    assert_eq!(state.6, 1);
    assert_eq!(state.7, 1);
    assert_eq!(state.8, 1);
}

#[derive(sqlx::FromRow)]
struct RefusalLeaseFence {
    status: String,
    lease_token_hash: Vec<u8>,
    attempt: i32,
    generation: i64,
    leased_by: String,
    lease_expires_at: time::OffsetDateTime,
    updated_at: time::OffsetDateTime,
    cancel_requested: bool,
    output_absent: bool,
    reported_lease_absent: bool,
    database_time: time::OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct BindingAuditWitness {
    id: Uuid,
    tenant_id: Uuid,
    actor: String,
    action: String,
    resource_type: String,
    resource_id: String,
    reason: Option<String>,
    trace_id: Option<String>,
    created_at: time::OffsetDateTime,
}

async fn refusal_lease_fence(platform: &Platform, run: Uuid, step: Uuid) -> RefusalLeaseFence {
    sqlx::query_as(
        "SELECT status,lease_token_hash,attempt,generation,leased_by,lease_expires_at,updated_at, \
                cancel_requested,output_digest IS NULL AS output_absent, \
                reported_lease_token_hash IS NULL AS reported_lease_absent, \
                clock_timestamp() AS database_time \
           FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3 \
             AND instance_path_nodes=ARRAY['lines']::text[] \
             AND instance_path_indices=ARRAY[16]::integer[] AND node='reserve'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .bind(step)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact seventeenth lease fence")
}

async fn exact_step_row(platform: &Platform, run: Uuid, step: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(step) FROM steps step \
          WHERE step.tenant_id=$1 AND step.run_id=$2 AND step.step_id=$3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .bind(step)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact seventeenth step row")
}

async fn checkpoint_snapshot(platform: &Platform, run: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(checkpoint) \
                  ORDER BY checkpoint.scope_id,checkpoint.iteration_index),'[]'::jsonb) \
           FROM iteration_checkpoints checkpoint \
          WHERE checkpoint.tenant_id=$1 AND checkpoint.run_id=$2",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact iteration checkpoint snapshot")
}

async fn binding_rows(platform: &Platform, namespace: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT pattern,updated_by FROM worker_bindings \
          WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker' ORDER BY pattern",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("exact scope-worker binding rows")
}

async fn binding_physical_bytes(platform: &Platform, namespace: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce(sum((octet_length(pattern)+octet_length(subject))::bigint),0)::bigint \
           FROM worker_bindings \
          WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact scope-worker binding bytes")
}

async fn binding_pool_snapshot(platform: &Platform, namespace: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(pool_row) ORDER BY pool_row.pool),'[]'::jsonb) \
           FROM worker_binding_pools pool_row \
          WHERE pool_row.tenant_id=$1 AND pool_row.namespace_id=$2 \
            AND pool_row.subject='scope-worker'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact scope-worker pool rows")
}

async fn worker_membership_snapshot(platform: &Platform) -> Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(member) FROM memberships member \
          WHERE member.tenant_id=$1 AND member.subject='scope-worker'",
    )
    .bind(platform.tenant.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact scope-worker membership")
}

async fn worker_session_witness(
    platform: &Platform,
    namespace: Uuid,
) -> Vec<(Uuid, Vec<String>, Vec<String>)> {
    sqlx::query_as(
        "SELECT id,activities,pools FROM worker_sessions \
          WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker' ORDER BY id",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("exact scope-worker sessions")
}

async fn worker_authority_snapshot(platform: &Platform) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'bindings',(SELECT coalesce(jsonb_agg(to_jsonb(binding_row) \
              ORDER BY binding_row.namespace_id,binding_row.subject,binding_row.pattern),'[]'::jsonb) \
              FROM worker_bindings binding_row WHERE binding_row.tenant_id=$1), \
            'bindingPools',(SELECT coalesce(jsonb_agg(to_jsonb(pool_row) \
              ORDER BY pool_row.namespace_id,pool_row.subject,pool_row.pool),'[]'::jsonb) \
              FROM worker_binding_pools pool_row WHERE pool_row.tenant_id=$1), \
            'sessions',(SELECT coalesce(jsonb_agg(to_jsonb(session_row) \
              ORDER BY session_row.namespace_id,session_row.subject,session_row.id),'[]'::jsonb) \
              FROM worker_sessions session_row WHERE session_row.tenant_id=$1), \
            'memberships',(SELECT coalesce(jsonb_agg(to_jsonb(member_row) \
              ORDER BY member_row.subject),'[]'::jsonb) \
              FROM memberships member_row WHERE member_row.tenant_id=$1))",
    )
    .bind(platform.tenant.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("complete tenant worker authority snapshot")
}

async fn exact_binding_updated_at(platform: &Platform, namespace: Uuid) -> time::OffsetDateTime {
    sqlx::query_scalar(
        "SELECT updated_at FROM worker_bindings \
          WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker' AND pattern=$3",
    )
    .bind(platform.tenant.as_uuid())
    .bind(namespace)
    .bind(ACTIVITY)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("exact retained binding update time")
}

fn authority_narrowing_is_exact(
    before: &Value,
    after: &Value,
    namespace: Uuid,
    removed_session: Uuid,
) -> bool {
    let namespace = namespace.to_string();
    let removed_session = removed_session.to_string();
    let Some(before_bindings) = before["bindings"].as_array() else {
        return false;
    };
    let Some(after_bindings) = after["bindings"].as_array() else {
        return false;
    };
    let binding_is_target = |row: &&Value| {
        row["namespace_id"].as_str() == Some(namespace.as_str())
            && row["subject"].as_str() == Some("scope-worker")
    };
    let before_target_bindings: Vec<&Value> = before_bindings
        .iter()
        .filter(|row| binding_is_target(row))
        .collect();
    let after_target_bindings: Vec<&Value> = after_bindings
        .iter()
        .filter(|row| binding_is_target(row))
        .collect();
    let before_other_bindings: Vec<&Value> = before_bindings
        .iter()
        .filter(|row| !binding_is_target(row))
        .collect();
    let after_other_bindings: Vec<&Value> = after_bindings
        .iter()
        .filter(|row| !binding_is_target(row))
        .collect();
    if before_target_bindings.len() != 2
        || after_target_bindings.len() != 1
        || before_other_bindings != after_other_bindings
    {
        return false;
    }

    let Some(before_sessions) = before["sessions"].as_array() else {
        return false;
    };
    let Some(after_sessions) = after["sessions"].as_array() else {
        return false;
    };
    let session_is_target = |row: &&Value| row["id"].as_str() == Some(removed_session.as_str());
    let before_target_sessions: Vec<&Value> = before_sessions
        .iter()
        .filter(|row| session_is_target(row))
        .collect();
    let after_target_sessions: Vec<&Value> = after_sessions
        .iter()
        .filter(|row| session_is_target(row))
        .collect();
    let before_other_sessions: Vec<&Value> = before_sessions
        .iter()
        .filter(|row| !session_is_target(row))
        .collect();
    let after_other_sessions: Vec<&Value> = after_sessions
        .iter()
        .filter(|row| !session_is_target(row))
        .collect();
    before_target_sessions.len() == 1
        && after_target_sessions.is_empty()
        && before_other_sessions == after_other_sessions
        && before["bindingPools"] == after["bindingPools"]
        && before["memberships"] == after["memberships"]
}

fn restore_native_expiry_fields(snapshot: &mut Value, step: Uuid, original: &Value) {
    let step = step.to_string();
    let target = snapshot["steps"]
        .as_array_mut()
        .expect("snapshot step rows")
        .iter_mut()
        .find(|row| row["step_id"].as_str() == Some(step.as_str()))
        .expect("seventeenth snapshot step");
    target["lease_expires_at"] = original["lease_expires_at"].clone();
    target["updated_at"] = original["updated_at"].clone();
}

fn only_native_expiry_fields_changed(after: &Value, before: &Value) -> bool {
    let mut normalized = after.clone();
    normalized["lease_expires_at"] = before["lease_expires_at"].clone();
    normalized["updated_at"] = before["updated_at"].clone();
    &normalized == before
}

#[tokio::test(flavor = "current_thread")]
async fn a_scoped_refusal_rechecks_openbao_versions_after_its_path_check() {
    let provider_token = Uuid::now_v7().to_string();
    let openbao = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", provider_token.clone())
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .expect("real OpenBao");
    let port = openbao
        .get_host_port_ipv4(8200)
        .await
        .expect("OpenBao port");
    let credential_directory = tempfile::tempdir().expect("private provider credential directory");
    let token_file = credential_directory.path().join("token");
    std::fs::write(&token_file, provider_token).expect("private provider credential");
    let provider: Arc<dyn SecretProvider> = Arc::new(
        OpenBao::new(OpenBaoConfig {
            address: format!("http://127.0.0.1:{port}"),
            mount: "secret".into(),
            token_file,
            timeout: Duration::from_secs(5),
            development_loopback: true,
            ca_pem: None,
        })
        .expect("official OpenBao adapter"),
    );
    let platform = platform_with(Options {
        runtime_role: true,
        statement_timeout: Duration::from_secs(30),
        secret_provider: Some(provider),
        ..Options::default()
    })
    .await;
    let namespace = platform.namespace_id().await;
    let namespace_id = Uuid::parse_str(&namespace).expect("namespace id");
    let secret_v1 = format!("version-one-{}", Uuid::now_v7());
    let secret_v2 = format!("version-two-{}", Uuid::now_v7());
    let v1_request = Uuid::now_v7();
    let v2_request = Uuid::now_v7();
    let (_log_guard, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);
    let v1_response = put_secret(&platform, &namespace, &secret_v1, 0, v1_request).await;
    assert_eq!(v1_response["version"], 1);
    let v1_metadata = secret_metadata(&platform, namespace_id).await;
    assert_eq!(v1_metadata["authority"], "openbao");
    assert_eq!(v1_metadata["currentVersion"], 1);
    assert_eq!(
        v1_metadata["versions"].as_array().expect("versions").len(),
        1
    );
    assert_eq!(v1_metadata["versions"][0]["providerActive"], true);
    assert_eq!(v1_metadata["versions"][0]["hasLocalCiphertext"], false);

    let (quota_status, _) = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 4})),
        )
        .await;
    assert_eq!(quota_status, 200);
    let name = format!("refusal-secret-version-{}", Uuid::now_v7());
    let workflow = platform.publish(&name, &refusal_document(&name)).await;
    let run: Uuid = platform
        .start(
            &workflow,
            &json!({"lines": (0_u32..17).collect::<Vec<_>>() }),
        )
        .await
        .parse()
        .expect("run id");
    let scheduler = scheduler(&platform, None).await;
    let mut worker = scoped_worker(&platform, "scope-worker", &[ACTIVITY, "line.release"]).await;
    let output = refused_member_output();
    assert!(
        !output.to_string().contains(&secret_v1),
        "the report body must not contain the first secret value"
    );
    assert!(
        !output.to_string().contains(&secret_v2),
        "the report body must not contain the second secret value"
    );
    for index in 0_u32..16 {
        drive(&scheduler).await;
        let tasks = worker.poll(8).await;
        assert_eq!(tasks.len(), 1);
        let task = &tasks[0];
        let instance = task.instance.as_ref().expect("scoped task identity");
        assert_eq!(instance.definition_node_id, "reserve");
        assert_eq!(instance.instance_path.len(), 1);
        assert_eq!(instance.instance_path[0].iteration_index, index);
        let accepted = worker
            .complete(task, &output)
            .await
            .expect("a real bounded member report");
        assert_eq!(accepted.step_status, "SUCCEEDED");
    }
    drive(&scheduler).await;
    let mut tasks = worker.poll(8).await;
    assert_eq!(tasks.len(), 1);
    let seventeenth = tasks.remove(0);
    let seventeenth_instance = seventeenth
        .instance
        .as_ref()
        .expect("seventeenth scoped identity");
    assert_eq!(seventeenth_instance.definition_node_id, "reserve");
    assert_eq!(seventeenth_instance.instance_path.len(), 1);
    assert_eq!(seventeenth_instance.instance_path[0].iteration_index, 16);
    let premises: (i64, i64, String, String) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=scope.tenant_id AND checkpoint.run_id=scope.run_id), \
                usage.logical_output_bytes,scope.lifecycle,step.status \
           FROM iteration_scopes scope JOIN run_instance_usage usage \
             ON usage.tenant_id=scope.tenant_id AND usage.run_id=scope.run_id \
           JOIN steps step ON step.tenant_id=scope.tenant_id AND step.run_id=scope.run_id \
          WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='lines' \
            AND step.instance_path_nodes=ARRAY['lines']::text[] \
            AND step.instance_path_indices=ARRAY[16]::integer[] AND step.node='reserve'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("the public collection reached its seventeenth member");
    assert_eq!(premises.0, 16);
    assert_eq!(premises.1, 4_000_000);
    assert_eq!(premises.2, "OPEN");
    assert_eq!(premises.3, "LEASED");

    let before_report = refusal_snapshot(&platform, run, &[], 0).await;
    let catalog_before_rotation = secret_catalog_snapshot(&platform, namespace_id).await;
    let catalog_counts_before_rotation = secret_catalog_counts(&platform).await;
    let retained_before_rotation = retained_usage(&platform).await;

    let key = format!("done-{}", seventeenth.task_id);
    // Iterations reuse task_id; the full receipt address distinguishes this member and lease.
    let token_prefix = Token::hash_of(&seventeenth.lease_token)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let scope = format!("step:{run}:scoped:5:lines:16;7:reserve:{token_prefix}");
    let mut gate = platform
        .api
        .db
        .pool
        .begin()
        .await
        .expect("owned checkpoint table gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("checkpoint table gate pid");
    sqlx::query("LOCK TABLE public.iteration_checkpoints IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gate)
        .await
        .expect("hold exact full-restore table gate");

    let (first, (post_rotation, rotation_audits, rotation_charge)) = {
        let reporting = worker.complete(&seventeenth, &output);
        tokio::pin!(reporting);
        let observe_and_rotate = async {
            let writer_pid = waiting_full_checkpoint_restore(&platform, gate_pid).await;
            assert_ne!(writer_pid, gate_pid);
            assert_no_refusal_effect(&platform, run, &key, &scope).await;
            assert!(
                refusal_snapshot(&platform, run, &[], 0).await == before_report,
                "the blocked report must preserve the complete pre-report execution snapshot"
            );

            let v2_response = put_secret(&platform, &namespace, &secret_v2, 1, v2_request).await;
            assert_eq!(v2_response["version"], 2);
            let v2_metadata = secret_metadata(&platform, namespace_id).await;
            assert_eq!(v2_metadata["authority"], "openbao");
            assert_eq!(v2_metadata["currentVersion"], 2);
            let versions = v2_metadata["versions"].as_array().expect("versions");
            assert_eq!(versions.len(), 2);
            assert_eq!(versions[0]["version"], 1);
            assert_eq!(versions[1]["version"], 2);
            assert!(
                v1_metadata["versions"][0] == versions[0],
                "the public rotation must preserve version one metadata"
            );
            assert!(
                versions[0]["id"] != versions[1]["id"],
                "the rotation must append a distinct secret version identity"
            );
            assert!(
                versions[0]["providerOperation"] != versions[1]["providerOperation"],
                "the rotation must append a distinct provider operation identity"
            );
            assert!(
                versions
                    .iter()
                    .all(|version| version["providerActive"] == true)
            );
            assert!(
                versions
                    .iter()
                    .all(|version| version["hasLocalCiphertext"] == false)
            );
            assert!(
                v1_metadata != v2_metadata,
                "the public rotation must change the stored provider metadata"
            );
            let operation = Uuid::parse_str(
                versions[1]["providerOperation"]
                    .as_str()
                    .expect("version two provider operation"),
            )
            .expect("version two provider operation id");
            let catalog_after_rotation = secret_catalog_snapshot(&platform, namespace_id).await;
            assert_exact_rotation_catalog(
                &platform,
                namespace_id,
                v2_request,
                operation,
                secret_v2.len(),
                catalog_counts_before_rotation,
                (&catalog_before_rotation, &catalog_after_rotation),
            )
            .await;
            let rotation_audits =
                verified_rotation_audits(&platform, namespace_id, operation).await;
            let rotation_charge =
                i64::try_from(secret_v2.len()).expect("bounded provider value bytes");
            let retained_after_rotation = retained_usage(&platform).await;
            assert!(
                retained_after_rotation.0 == retained_before_rotation.0 + rotation_charge
                    && retained_after_rotation.1 == retained_before_rotation.1
                    && retained_after_rotation.2 == retained_before_rotation.2,
                "the retained quota ledger must charge exactly the new provider value bytes"
            );
            assert!(
                still_waiting_full_checkpoint_restore(&platform, gate_pid, writer_pid).await,
                "the same full restore query must stay blocked through the public rotation"
            );

            let post_rotation =
                refusal_snapshot(&platform, run, &rotation_audits, rotation_charge).await;
            assert!(
                post_rotation == before_report,
                "only the verified provider operation, its audits and exact quota charge may change during rotation"
            );
            assert_no_refusal_effect(&platform, run, &key, &scope).await;
            assert!(
                still_waiting_full_checkpoint_restore(&platform, gate_pid, writer_pid).await,
                "the exact full restore remains blocked through the post-rotation snapshot"
            );
            gate.rollback()
                .await
                .expect("release only the owned checkpoint table gate");
            (post_rotation, rotation_audits, rotation_charge)
        };
        tokio::pin!(observe_and_rotate);
        tokio::join!(&mut reporting, &mut observe_and_rotate)
    };
    let stale_scan = first.expect_err("rotating after the path scan rejects its stale evidence");
    assert_eq!(stale_scan.code(), tonic::Code::Unavailable);
    assert_eq!(machine_code(&stale_scan), "UNAVAILABLE");
    assert!(
        refusal_snapshot(&platform, run, &rotation_audits, rotation_charge).await == post_rotation,
        "the rejected stale scan has no execution, payload, usage, receipt, outbox or audit effect"
    );
    assert_no_refusal_effect(&platform, run, &key, &scope).await;

    let accepted = worker
        .complete(&seventeenth, &output)
        .await
        .expect("the same key and lease rescan current OpenBao versions");
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert_one_truthful_refusal(&platform, run, &key, &scope).await;
    let cold_store = PgRunStore::new(
        platform.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 64),
    )
    .with_scoped_worker_protocol();
    let cold_verification = cold_store
        .verify_run(platform.tenant, RunId::new(run))
        .await;
    assert!(
        cold_verification.is_ok(),
        "a fresh store and definition cache must rebuild the committed journal"
    );

    let committed = refusal_snapshot(&platform, run, &rotation_audits, rotation_charge).await;
    assert!(
        committed["outbox"] == post_rotation["outbox"],
        "the truthful refusal must create no outbox message"
    );
    let replay = worker
        .complete(&seventeenth, &output)
        .await
        .expect("the identical key replays the one refusal receipt");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_one_truthful_refusal(&platform, run, &key, &scope).await;
    assert!(
        refusal_snapshot(&platform, run, &rotation_audits, rotation_charge).await == committed,
        "receipt replay must preserve the complete committed execution snapshot"
    );

    let captured = logs.text();
    assert!(
        !captured.contains(&secret_v1),
        "captured logs must not contain the first secret value"
    );
    assert!(
        !captured.contains(&secret_v2),
        "captured logs must not contain the second secret value"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_scanned_output_refusal_rechecks_the_original_lease_after_path_drop() {
    let provider_token = Uuid::now_v7().to_string();
    let openbao = GenericImage::new("openbao/openbao", "2.7.1")
        .with_wait_for(WaitFor::message_on_stdout(
            "Development mode should NOT be used in production",
        ))
        .with_exposed_port(8200.into())
        .with_env_var("BAO_DEV_ROOT_TOKEN_ID", provider_token.clone())
        .with_cmd(["server", "-dev", "-dev-listen-address=0.0.0.0:8200"])
        .start()
        .await
        .expect("real OpenBao");
    let port = openbao
        .get_host_port_ipv4(8200)
        .await
        .expect("OpenBao port");
    let credential_directory = tempfile::tempdir().expect("private provider credential directory");
    let token_file = credential_directory.path().join("token");
    std::fs::write(&token_file, provider_token).expect("private provider credential");
    let provider: Arc<dyn SecretProvider> = Arc::new(
        OpenBao::new(OpenBaoConfig {
            address: format!("http://127.0.0.1:{port}"),
            mount: "secret".into(),
            token_file,
            timeout: Duration::from_secs(5),
            development_loopback: true,
            ca_pem: None,
        })
        .expect("official OpenBao adapter"),
    );
    let platform = platform_with(Options {
        runtime_role: true,
        statement_timeout: Duration::from_secs(30),
        secret_provider: Some(provider),
        ..Options::default()
    })
    .await;
    let namespace = platform.namespace_id().await;
    let namespace_id = Uuid::parse_str(&namespace).expect("namespace id");
    let secret_value = format!("current-lease-{}", Uuid::now_v7());
    let (_log_guard, logs) = crate::support::logs::capture_at(tracing::Level::TRACE);
    let secret_response = put_secret(&platform, &namespace, &secret_value, 0, Uuid::now_v7()).await;
    assert!(
        secret_response["version"] == 1,
        "the public managed secret must begin at version one"
    );

    let (quota_status, _) = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 4})),
        )
        .await;
    assert_eq!(quota_status, 200);
    let name = format!("refusal-current-lease-{}", Uuid::now_v7());
    let workflow = platform.publish(&name, &refusal_document(&name)).await;
    let run: Uuid = platform
        .start(
            &workflow,
            &json!({"lines": (0_u32..17).collect::<Vec<_>>() }),
        )
        .await
        .parse()
        .expect("run id");
    let scheduler = scheduler(&platform, None).await;
    let mut worker = scoped_worker(&platform, "scope-worker", &[ACTIVITY, "line.release"]).await;

    let (setup_status, setup_binding) = platform
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/scope-worker"),
            Some(&json!({"activities":[ACTIVITY,"line.release"]})),
        )
        .await;
    assert_eq!(setup_status, 200);
    assert!(
        setup_binding["subject"] == "scope-worker"
            && setup_binding["activities"] == json!(["line.release", ACTIVITY])
            && setup_binding["pools"] == json!([])
            && setup_binding["revoked"]["sessionsClosed"] == 0
            && setup_binding["revoked"]["leasesReleased"] == 0,
        "the initial public binding must normalize both advertised activities without revocation"
    );

    let output = refused_member_output();
    assert!(
        !output.to_string().contains(&secret_value),
        "the report body must not contain the managed secret value"
    );
    for index in 0_u32..16 {
        drive(&scheduler).await;
        let tasks = worker.poll(8).await;
        assert_eq!(tasks.len(), 1);
        let task = &tasks[0];
        let instance = task.instance.as_ref().expect("scoped task identity");
        assert!(
            instance.definition_node_id == "reserve"
                && instance.instance_path.len() == 1
                && instance.instance_path[0].iteration_index == index,
            "the public producer must lease the expected bounded member"
        );
        let accepted = worker
            .complete(task, &output)
            .await
            .expect("a real bounded member report");
        assert!(
            accepted.step_status == "SUCCEEDED",
            "each retained producer member must succeed"
        );
    }
    drive(&scheduler).await;
    let mut tasks = worker.poll(8).await;
    assert_eq!(tasks.len(), 1);
    let seventeenth = tasks.remove(0);
    let seventeenth_instance = seventeenth
        .instance
        .as_ref()
        .expect("seventeenth scoped identity");
    assert!(
        seventeenth_instance.definition_node_id == "reserve"
            && seventeenth_instance.instance_path.len() == 1
            && seventeenth_instance.instance_path[0].iteration_index == 16,
        "the held report must be the original seventeenth member"
    );
    let step = Uuid::parse_str(&seventeenth.step_id).expect("seventeenth step id");
    let premises: (i64, i64, String, i64, String) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=scope.tenant_id AND checkpoint.run_id=scope.run_id), \
                usage.logical_output_bytes,scope.lifecycle, \
                (SELECT count(*) FROM collection_iterations member \
                  WHERE member.tenant_id=scope.tenant_id AND member.run_id=scope.run_id \
                    AND member.scope_id=scope.scope_id),step.status \
           FROM iteration_scopes scope JOIN run_instance_usage usage \
             ON usage.tenant_id=scope.tenant_id AND usage.run_id=scope.run_id \
           JOIN steps step ON step.tenant_id=scope.tenant_id AND step.run_id=scope.run_id \
          WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='lines' \
            AND step.step_id=$3 AND step.instance_path_nodes=ARRAY['lines']::text[] \
            AND step.instance_path_indices=ARRAY[16]::integer[] AND step.node='reserve'",
    )
    .bind(platform.tenant.as_uuid())
    .bind(run)
    .bind(step)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("the public collection reached its seventeenth member");
    assert!(
        premises.0 == 16
            && premises.1 == 4_000_000
            && premises.2 == "OPEN"
            && premises.3 == 17
            && premises.4 == "LEASED",
        "the exact public refusal premise must be present"
    );
    assert!(
        usize::try_from(premises.1).expect("nonnegative retained output bytes")
            + output.to_string().len()
            > 4_194_304,
        "the seventeenth report must exceed the exact run output ceiling"
    );

    let sessions_before = worker_session_witness(&platform, namespace_id).await;
    assert!(
        sessions_before.len() == 1
            && sessions_before[0].1 == vec!["line.release".to_owned(), ACTIVITY.to_owned()]
            && sessions_before[0].2.is_empty(),
        "one registered session must advertise both exact activities and no pools"
    );
    let session = sessions_before[0].0;
    let authority_before = worker_authority_snapshot(&platform).await;
    let binding_before = binding_rows(&platform, namespace_id).await;
    assert!(
        binding_before
            == vec![
                ("line.release".to_owned(), "alice".to_owned()),
                (ACTIVITY.to_owned(), "alice".to_owned()),
            ],
        "the normalized public binding must contain both exact activities"
    );
    let binding_bytes_before = binding_physical_bytes(&platform, namespace_id).await;
    let pools_before = binding_pool_snapshot(&platform, namespace_id).await;
    let membership_before = worker_membership_snapshot(&platform).await;
    assert!(
        membership_before["role"] == "worker",
        "scope-worker must retain Worker membership"
    );
    let secret_catalog_before = secret_catalog_snapshot(&platform, namespace_id).await;
    let retained_before = retained_usage(&platform).await;
    let lease_before = refusal_lease_fence(&platform, run, step).await;
    assert!(
        lease_before.status == "LEASED"
            && lease_before.lease_token_hash == Token::hash_of(&seventeenth.lease_token)
            && lease_before.attempt == seventeenth.attempt
            && lease_before.generation == seventeenth.generation
            && lease_before.leased_by == session.to_string()
            && lease_before.lease_expires_at > lease_before.database_time
            && !lease_before.cancel_requested
            && lease_before.output_absent
            && lease_before.reported_lease_absent,
        "the original public lease must be exact, live, uncancelled and unreported"
    );
    let step_before = exact_step_row(&platform, run, step).await;
    let checkpoints_before = checkpoint_snapshot(&platform, run).await;
    let before_report = refusal_snapshot(&platform, run, &[], 0).await;

    let key = format!("done-{}", seventeenth.task_id);
    // Iterations reuse task_id; the full receipt address distinguishes this member and lease.
    let token_prefix = Token::hash_of(&seventeenth.lease_token)
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let scope = format!("step:{run}:scoped:5:lines:16;7:reserve:{token_prefix}");

    let mut gate = platform
        .api
        .db
        .pool
        .begin()
        .await
        .expect("owned checkpoint table gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("checkpoint table gate pid");
    sqlx::query("LOCK TABLE public.iteration_checkpoints IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gate)
        .await
        .expect("hold exact full-restore table gate");

    let reporting = worker.complete(&seventeenth, &output);
    tokio::pin!(reporting);
    let observe_and_narrow = async {
        let report_pid = waiting_full_checkpoint_restore(&platform, gate_pid).await;
        assert_ne!(report_pid, gate_pid);
        assert!(
            refusal_snapshot(&platform, run, &[], 0).await == before_report,
            "the blocked report must preserve the complete pre-report snapshot"
        );
        assert_no_refusal_effect(&platform, run, &key, &scope).await;
        let still_live = refusal_lease_fence(&platform, run, step).await;
        assert!(
            still_live.lease_expires_at == lease_before.lease_expires_at
                && still_live.updated_at == lease_before.updated_at
                && still_live.lease_expires_at > still_live.database_time,
            "the exact original lease must remain unchanged and live before binding replacement"
        );
        assert!(
            still_waiting_full_checkpoint_restore(&platform, gate_pid, report_pid).await,
            "the exact named runtime PID must remain blocked before binding replacement"
        );

        let replacement_started_at = time::OffsetDateTime::now_utc();
        let (status, trace_id, body, replacement_finished_at) =
            tokio::time::timeout(Duration::from_secs(5), async {
                let response = platform
                    .api
                    .as_user(
                        Method::PUT,
                        &format!("/namespaces/{namespace}/worker-bindings/scope-worker"),
                        "alice",
                        "acme",
                    )
                    .json(&json!({"activities":[ACTIVITY]}))
                    .send()
                    .await
                    .expect("actual binding replacement request");
                let status = response.status();
                let trace_id = response
                    .headers()
                    .get("x-trace-id")
                    .and_then(|value| value.to_str().ok())
                    .expect("native binding response trace id")
                    .to_owned();
                let body: Value = response.json().await.expect("binding replacement response");
                (status, trace_id, body, time::OffsetDateTime::now_utc())
            })
            .await
            .expect("binding replacement must complete while the report PID is blocked");
        assert!(
            status == reqwest::StatusCode::OK
                && body["subject"] == "scope-worker"
                && body["activities"] == json!([ACTIVITY])
                && body["pools"] == json!([])
                && body["revoked"]["sessionsClosed"] == 1
                && body["revoked"]["leasesReleased"] == 1,
            "the public replacement must narrow activity and report one nested session and lease revocation"
        );
        assert!(
            still_waiting_full_checkpoint_restore(&platform, gate_pid, report_pid).await,
            "the same named runtime PID must stay blocked through public binding replacement"
        );

        let binding_after = binding_rows(&platform, namespace_id).await;
        assert!(
            binding_after == vec![(ACTIVITY.to_owned(), "alice".to_owned())],
            "the current binding must retain only the report activity"
        );
        let binding_updated_at = exact_binding_updated_at(&platform, namespace_id).await;
        assert!(
            binding_updated_at >= replacement_started_at
                && binding_updated_at <= replacement_finished_at,
            "the retained binding metadata timestamp must belong to the public replacement request"
        );
        let binding_bytes_after = binding_physical_bytes(&platform, namespace_id).await;
        let binding_delta = binding_bytes_after - binding_bytes_before;
        let pools_after = binding_pool_snapshot(&platform, namespace_id).await;
        let membership_after = worker_membership_snapshot(&platform).await;
        let secret_catalog_after = secret_catalog_snapshot(&platform, namespace_id).await;
        assert!(
            pools_after == pools_before,
            "binding replacement must preserve the exact worker pool rows"
        );
        assert!(
            membership_after == membership_before,
            "binding replacement must preserve the complete Worker membership"
        );
        assert!(
            secret_catalog_after == secret_catalog_before,
            "binding replacement must preserve the complete managed secret catalog"
        );
        let sessions_after = worker_session_witness(&platform, namespace_id).await;
        assert!(
            sessions_after.is_empty(),
            "only the identified incompatible worker session must be removed"
        );
        let authority_after = worker_authority_snapshot(&platform).await;
        assert!(
            authority_narrowing_is_exact(
                &authority_before,
                &authority_after,
                namespace_id,
                session,
            ),
            "the public replacement must preserve every other complete tenant authority row and remove only the identified session"
        );

        let retained_after = retained_usage(&platform).await;
        assert!(
            retained_after.0 == retained_before.0 + binding_delta
                && retained_after.1 == retained_before.1
                && retained_after.2 == retained_before.2,
            "the retained quota ledger must apply only the exact binding byte delta"
        );

        let lease_after = refusal_lease_fence(&platform, run, step).await;
        assert!(
            lease_after.status == lease_before.status
                && lease_after.lease_token_hash == lease_before.lease_token_hash
                && lease_after.attempt == lease_before.attempt
                && lease_after.generation == lease_before.generation
                && lease_after.leased_by == lease_before.leased_by
                && lease_after.cancel_requested == lease_before.cancel_requested
                && lease_after.output_absent == lease_before.output_absent
                && lease_after.reported_lease_absent == lease_before.reported_lease_absent,
            "native binding replacement must preserve every non-time lease and report fact"
        );
        assert!(
            lease_after.lease_expires_at <= lease_after.database_time
                && lease_after.lease_expires_at < lease_before.lease_expires_at
                && lease_before.lease_expires_at > lease_after.database_time
                && lease_after.updated_at >= lease_before.updated_at,
            "native replacement alone must expire the still-naturally-live original lease"
        );
        let step_after = exact_step_row(&platform, run, step).await;
        assert!(
            only_native_expiry_fields_changed(&step_after, &step_before),
            "only native lease expiry and update time may change on the held step"
        );

        let mut audits: Vec<BindingAuditWitness> = sqlx::query_as(
            "SELECT id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at \
               FROM audit_records WHERE tenant_id=$1 AND trace_id=$2 ORDER BY id",
        )
        .bind(platform.tenant.as_uuid())
        .bind(&trace_id)
        .fetch_all(&platform.api.db.pool)
        .await
        .expect("trace-bound binding audit");
        assert!(
            audits.len() == 1,
            "the public binding replacement must append exactly one trace-bound audit"
        );
        let audit = audits.pop().expect("one trace-bound binding audit");
        assert!(
            audit.id != Uuid::nil()
                && audit.tenant_id == platform.tenant.as_uuid()
                && audit.actor == "alice"
                && audit.action == "worker.bound"
                && audit.resource_type == "worker_binding"
                && audit.resource_id == namespace
                && audit.reason.as_deref() == Some("scope-worker: line.reserve")
                && audit.trace_id.as_deref() == Some(trace_id.as_str())
                && audit.created_at >= replacement_started_at
                && audit.created_at <= replacement_finished_at,
            "the native worker.bound audit must have the exact identity, grant, trace and request bounds"
        );

        let post_mutation_raw = refusal_snapshot(&platform, run, &[], 0).await;
        let mut normalized = refusal_snapshot(&platform, run, &[audit.id], binding_delta).await;
        restore_native_expiry_fields(&mut normalized, step, &step_before);
        assert!(
            normalized == before_report,
            "the verified binding, session, expiry, audit and physical-byte changes must be the complete mutation delta"
        );
        assert_no_refusal_effect(&platform, run, &key, &scope).await;
        assert!(
            still_waiting_full_checkpoint_restore(&platform, gate_pid, report_pid).await,
            "the exact named runtime PID must remain blocked immediately before gate release"
        );
        gate.rollback()
            .await
            .expect("release only the owned checkpoint table gate");
        (
            post_mutation_raw,
            audit.id,
            binding_delta,
            binding_after,
            pools_after,
            membership_after,
            secret_catalog_after,
            authority_after,
        )
    };
    tokio::pin!(observe_and_narrow);
    let (reported, mutation) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(&mut reporting, &mut observe_and_narrow)
    })
    .await
    .expect("the original report and public binding replacement must finish");
    let stale = match reported {
        Err(stale) => stale,
        Ok(_) => panic!("the natively expired original lease must be rejected"),
    };
    assert_eq!(stale.code(), tonic::Code::Aborted);
    assert_eq!(machine_code(&stale), "STALE_ATTEMPT");

    let (
        post_mutation_raw,
        _binding_audit,
        _binding_delta,
        binding_after,
        pools_after,
        membership_after,
        secret_catalog_after,
        authority_after,
    ) = mutation;
    assert!(
        refusal_snapshot(&platform, run, &[], 0).await == post_mutation_raw,
        "the stale report must add no execution, payload, usage, receipt, audit, outbox or head effect"
    );
    assert!(
        checkpoint_snapshot(&platform, run).await == checkpoints_before,
        "the stale report must preserve every real producer checkpoint"
    );
    assert_no_refusal_effect(&platform, run, &key, &scope).await;
    assert!(
        binding_rows(&platform, namespace_id).await == binding_after
            && binding_pool_snapshot(&platform, namespace_id).await == pools_after
            && worker_membership_snapshot(&platform).await == membership_after
            && secret_catalog_snapshot(&platform, namespace_id).await == secret_catalog_after,
        "the stale report must preserve the exact current binding, pools, membership and secret catalog"
    );
    assert!(
        worker_authority_snapshot(&platform).await == authority_after,
        "the stale report must preserve the raw complete postmutation binding and session authority snapshot"
    );
    let final_lease = refusal_lease_fence(&platform, run, step).await;
    assert!(
        final_lease.lease_expires_at <= final_lease.database_time
            && final_lease.status == "LEASED"
            && final_lease.reported_lease_absent,
        "the rejected report must leave the original lease expired and unreported"
    );

    let cold_store = PgRunStore::new(
        platform.api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 64),
    )
    .with_scoped_worker_protocol();
    let cold_verification = cold_store
        .verify_run(platform.tenant, RunId::new(run))
        .await;
    assert!(
        cold_verification.is_ok(),
        "a fresh store and definition cache must rebuild the untouched journal"
    );

    let captured = logs.text();
    assert!(
        !captured.contains(&secret_value),
        "captured logs must not contain the managed secret value"
    );
}
