//! The existing byte ceilings (256 KiB per value, 4 MiB retained per run, 8 MiB per task context)
//! hold for collection and loop values too, against real PostgreSQL. A value over a ceiling is a
//! durable `RUN_DATA_LIMIT` failure, never a retry loop and never a silently stored oversize value.

use std::time::Duration;

use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{CompleteStepRequest, TaskLease};
use sagat_application::clock::SystemClock;
use sagat_application::definitions::Idempotency;
use sagat_application::entropy::SystemEntropy;
use sagat_application::entropy::Token;
use sagat_application::execution::{
    Actor, Effects, LeaseGrant, MAX_PAYLOAD_BYTES, MAX_RUN_OUTPUT_BYTES, PathExecutionStore,
    PathExecutionTx, ReportedLease, ScopeInput,
};
use sagat_application::maintenance::{MaintenanceStore, Window};
use sagat_application::runs::Lock;
use sagat_application::worker::{MAX_TASK_INPUT_BYTES, WorkerService};
use sagat_domain::execution::{
    ExecutionInstanceId, InstancePath, RunId, StepStatus, complete_path_instance,
    heartbeat_path_instance, lease_path_instance,
};
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::{ActivityName, NodeId};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use serde_json::{Value, json};

use crate::limits::{
    allow_iterations, assert_failed_with, register_scoped, scheduler, scoped_workers, scopes,
};
use crate::support::engine::ctx;
use crate::support::engine::{World, activity, document, end_node, task_ref, with, world};
use crate::support::scope_flow::{
    Flow, activity as scoped_activity, collection_document, iteration, node, retry, row, summary,
    worker_request,
};

type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;

const SCRIPT_DIGEST: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn text_of(bytes: usize) -> String {
    // A JSON string of exactly `bytes` bytes: the quotes count.
    format!("\"{}\"", "x".repeat(bytes - 2))
}

async fn ready_script(world: &World, retention_days: i16) -> ActivityName {
    sqlx::query(
        "INSERT INTO script_artifacts \
         (tenant_id,digest,language,content,signature,key_id,sbom,toolchain) \
         VALUES ($1,$2,'typescript','\\x01',decode(repeat('ab',64),'hex'),'key-1','\\x7b7d','node@sha256:aa')",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(SCRIPT_DIGEST)
    .execute(&world.db.pool)
    .await
    .expect("script artifact");
    let script = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO scripts \
         (id,tenant_id,namespace_id,name,language,source,policy,retention_days,created_by) \
         VALUES ($1,$2,$3,'bounded-script','typescript','export default 1','v1',$4,'test')",
    )
    .bind(script)
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(retention_days)
    .execute(&world.db.pool)
    .await
    .expect("script");
    sqlx::query(
        "INSERT INTO script_versions \
         (id,tenant_id,namespace_id,script_id,number,language,status,source,policy,retention_days, \
          content_digest,artifact_digest,created_by,created_at) \
         VALUES ($1,$2,$3,$4,1,'typescript','READY','export default 1','v1',$5,$6,$6,'test',now())",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(world.admin.tenant_id().as_uuid())
    .bind(world.namespace.id().as_uuid())
    .bind(script)
    .bind(retention_days)
    .bind(SCRIPT_DIGEST)
    .execute(&world.db.pool)
    .await
    .expect("READY script version");
    let activity = format!("script.sha256-{}", &SCRIPT_DIGEST["sha256:".len()..]);
    ActivityName::parse(&activity).expect("script activity")
}

async fn set_script_retention(world: &World, days: u64) {
    world
        .quotas
        .set(
            &world.admin,
            Some(world.namespace.id()),
            QuotaResource::ScriptRetentionDays,
            days,
            &ctx(),
        )
        .await
        .expect("set script retention policy");
}

fn successful_refusal_collection(name: &str, reconciliation: bool) -> Value {
    let mut reserve = json!({
        "compensateWith": "line.release",
        "compensationRetry": retry()
    });
    if reconciliation {
        reserve["reconcileWith"] = json!("line.lookup");
    }
    let mut document = collection_document(
        name,
        vec![
            scoped_activity("reserve", "line.reserve", "returned", reserve),
            json!({"id": "returned", "type": "return", "result": "1"}),
        ],
        1,
    );
    document["nodes"][0]["maxItems"] = json!(17);
    document
}

fn refused_member_output() -> Value {
    let output = Value::String("x".repeat(249_998));
    assert_eq!(output.to_string().len(), 250_000);
    output
}

async fn collection_at_seventeenth_member(
    name: &str,
    reconciliation: bool,
) -> (Flow, TaskLease, Value) {
    collection_at_seventeenth_member_with_output(name, reconciliation, |_| refused_member_output())
        .await
}

async fn collection_at_seventeenth_member_with_output(
    name: &str,
    reconciliation: bool,
    output_for: impl Fn(u32) -> Value,
) -> (Flow, TaskLease, Value) {
    let lines: Vec<u32> = (0..17).collect();
    let flow = Flow::start(
        name,
        successful_refusal_collection(name, reconciliation),
        json!(lines),
    )
    .await;
    for index in 0..16 {
        let output = output_for(index);
        flow.drive().await;
        let tasks = flow.poll().await;
        assert_eq!(
            summary(&tasks),
            vec![row(index, "reserve", "FORWARD", "line.reserve")],
            "{}",
            crate::support::scheduler_evidence(&flow.api.db.pool, flow.tenant, flow.run).await
        );
        flow.complete(&tasks[0], output.clone()).await;
    }
    flow.drive().await;
    let tasks = flow.poll().await;
    assert_eq!(
        summary(&tasks),
        vec![row(16, "reserve", "FORWARD", "line.reserve")]
    );
    let checkpoints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM iteration_checkpoints WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("first sixteen member checkpoints");
    assert_eq!(checkpoints, 16);
    let output = output_for(16);
    (flow, tasks[0].clone(), output)
}

async fn run_outboxes(flow: &Flow) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2")
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("run outbox count")
}

async fn refusal_identity(flow: &Flow, key: &str) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM audit_records WHERE tenant_id=$1 \
             AND resource_type='run' AND resource_id=$2::text \
             AND action='run.data_limit_exceeded' \
             AND reason='successful scoped output refused by run data limit'), \
           (SELECT count(*) FROM command_dedupe WHERE tenant_id=$1 AND idempotency_key=$3), \
           (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2), \
           (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
           (SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2)",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .bind(key)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("refusal identity")
}

#[derive(sqlx::FromRow)]
struct RefusalLeaseState {
    status: String,
    lease_token_hash: Vec<u8>,
    attempt: i32,
    generation: i64,
    leased_by: String,
    lease_expires_at: time::OffsetDateTime,
    updated_at: time::OffsetDateTime,
    expired: bool,
}

#[derive(sqlx::FromRow)]
struct RecordedAudit {
    id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    actor: String,
    action: String,
    resource_type: String,
    resource_id: String,
    reason: Option<String>,
    trace_id: Option<String>,
    created_at: time::OffsetDateTime,
}

#[derive(Debug, sqlx::FromRow)]
struct RejectedRefusalState {
    run_status: String,
    stop_reason: Option<String>,
    stop_code: Option<String>,
    step_status: String,
    output_absent: bool,
    report_absent: bool,
    logical_output_bytes: i64,
    head_seq: i64,
    run_version: i64,
    receipts: i64,
    refusal_audits: i64,
}

async fn refusal_lease_state(flow: &Flow, step: uuid::Uuid) -> RefusalLeaseState {
    sqlx::query_as(
        "SELECT status,lease_token_hash,attempt,generation,leased_by,lease_expires_at,updated_at, \
                lease_expires_at<=clock_timestamp() AS expired \
           FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3 \
             AND instance_path_nodes=ARRAY['lines']::text[] \
             AND instance_path_indices=ARRAY[16]::integer[] AND node='reserve'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .bind(step)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("exact seventeenth refusal lease")
}

fn positive_advisory_key() -> i64 {
    i64::try_from(uuid::Uuid::new_v4().as_u128() & ((1_u128 << 63) - 1))
        .expect("positive advisory key")
}

async fn install_refusal_receipt_fault(pool: &sqlx::PgPool, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION scoped_refusal_receipt_fault() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF NEW.resource_id LIKE 'SUCCEEDED|CANCELLING|%' THEN \
             PERFORM pg_advisory_xact_lock({key}); \
             RAISE EXCEPTION 'test-only scoped output refusal receipt abort' \
               USING ERRCODE='23514',CONSTRAINT='scoped_refusal_receipt_forced_abort'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER scoped_refusal_receipt_gate AFTER INSERT ON command_dedupe \
         FOR EACH ROW EXECUTE FUNCTION scoped_refusal_receipt_fault();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install labelled refusal receipt fault");
}

async fn remove_refusal_receipt_fault(pool: &sqlx::PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER scoped_refusal_receipt_gate ON command_dedupe; \
         DROP FUNCTION scoped_refusal_receipt_fault();",
    )
    .execute(pool)
    .await
    .expect("remove labelled refusal receipt fault");
}

async fn install_refusal_scoped_persist_fault(pool: &sqlx::PgPool, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION scoped_refusal_persist_fault() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           PERFORM pg_advisory_xact_lock({key}); \
           RAISE EXCEPTION 'test-only scoped output refusal between-persist abort' \
             USING ERRCODE='23514',CONSTRAINT='scoped_refusal_between_persist_forced_abort'; \
         END $$; \
         CREATE TRIGGER scoped_refusal_persist_gate BEFORE INSERT ON run_events \
         FOR EACH ROW WHEN (NEW.event_type='step.completed' AND NEW.node='reserve' \
           AND NEW.instance_path_nodes=ARRAY['lines']::text[] \
           AND NEW.instance_path_indices=ARRAY[16]::integer[]) \
         EXECUTE FUNCTION scoped_refusal_persist_fault();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install labelled between-persist refusal fault");
}

async fn remove_refusal_scoped_persist_fault(pool: &sqlx::PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER scoped_refusal_persist_gate ON run_events; \
         DROP FUNCTION scoped_refusal_persist_fault();",
    )
    .execute(pool)
    .await
    .expect("remove labelled between-persist refusal fault");
}

async fn install_collection_drain_outbox_fault(pool: &sqlx::PgPool, key: i64) {
    let ddl = format!(
        "CREATE FUNCTION collection_drain_outbox_fault() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           PERFORM pg_advisory_xact_lock({key}); \
           RAISE EXCEPTION 'test-only collection drain outbox abort' \
             USING ERRCODE='23514',CONSTRAINT='collection_drain_outbox_forced_abort'; \
         END $$; \
         CREATE TRIGGER collection_drain_outbox_gate AFTER INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type='collection.drain.started') \
         EXECUTE FUNCTION collection_drain_outbox_fault();"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
        .execute(pool)
        .await
        .expect("install labelled collection drain outbox fault");
}

async fn remove_collection_drain_outbox_fault(pool: &sqlx::PgPool) {
    sqlx::raw_sql(
        "DROP TRIGGER collection_drain_outbox_gate ON event_outbox; \
         DROP FUNCTION collection_drain_outbox_fault();",
    )
    .execute(pool)
    .await
    .expect("remove labelled collection drain outbox fault");
}

async fn refusal_transaction_snapshot(flow: &Flow) -> Value {
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
            'tenantRetainedUsage',(SELECT to_jsonb(retained) FROM tenant_retained_usage retained \
              WHERE retained.tenant_id=run.tenant_id), \
            'audits',(SELECT coalesce(jsonb_agg(to_jsonb(audit) ORDER BY audit.id),'[]'::jsonb) \
              FROM audit_records audit WHERE audit.tenant_id=run.tenant_id), \
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
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("complete refusal transaction snapshot")
}

fn after_expected_binding_withdrawal(
    before: &Value,
    after: &Value,
    step_id: uuid::Uuid,
    audit_id: uuid::Uuid,
) -> Value {
    let step_id = step_id.to_string();
    let mut matching_before_steps = before["steps"]
        .as_array()
        .expect("refusal snapshot steps")
        .iter()
        .filter(|step| step["step_id"].as_str() == Some(step_id.as_str()));
    let before_step = matching_before_steps
        .next()
        .expect("seventeenth refusal step in pre-unbind snapshot");
    assert!(
        matching_before_steps.next().is_none(),
        "the pre-unbind snapshot contains one exact target step"
    );
    let before_lease_expires_at = before_step["lease_expires_at"].clone();
    let before_updated_at = before_step["updated_at"].clone();
    let before_content_bytes = before["tenantRetainedUsage"]["content_bytes"].clone();

    let mut stable = without_exact_audit(after, audit_id);
    let mut matching_steps = stable["steps"]
        .as_array_mut()
        .expect("refusal snapshot steps")
        .iter_mut()
        .filter(|step| step["step_id"].as_str() == Some(step_id.as_str()));
    let target = matching_steps
        .next()
        .expect("seventeenth refusal step in post-unbind snapshot");
    assert!(
        matching_steps.next().is_none(),
        "the refusal snapshot contains one exact target step"
    );
    let target = target.as_object_mut().expect("refusal snapshot step");
    target.insert("lease_expires_at".into(), before_lease_expires_at);
    target.insert("updated_at".into(), before_updated_at);

    stable["tenantRetainedUsage"]
        .as_object_mut()
        .expect("tenant retained usage snapshot")
        .insert("content_bytes".into(), before_content_bytes);
    stable
}

fn without_exact_audit(snapshot: &Value, audit_id: uuid::Uuid) -> Value {
    let mut stable = snapshot.clone();
    let audit_id = audit_id.to_string();
    let audits = stable["audits"]
        .as_array_mut()
        .expect("refusal snapshot audits");
    let audit_count = audits.len();
    audits.retain(|audit| audit["id"].as_str() != Some(audit_id.as_str()));
    assert!(
        audits.len() + 1 == audit_count,
        "the snapshot contains exactly the traced native audit"
    );
    stable
}

async fn waiting_refusal_receipt(flow: &Flow, gate_pid: i32) -> i32 {
    crate::support::eventually(
        "actual refusal receipt waits after the root and scoped persists",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity \
                   WHERE datname=current_database() AND usename='sagat_app' \
                     AND state='active' AND wait_event_type='Lock' \
                     AND $1=ANY(pg_blocking_pids(pid)) \
                     AND position('INSERT INTO command_dedupe' in query)>0 LIMIT 1",
            )
            .bind(gate_pid)
            .fetch_optional(&flow.api.db.pool)
            .await
            .expect("refusal receipt advisory wait witness")
        },
    )
    .await
}

async fn waiting_refusal_scoped_persist(flow: &Flow, gate_pid: i32) -> i32 {
    crate::support::eventually(
        "actual refusal writer waits on the first scoped persist write",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity \
                   WHERE datname=current_database() AND usename='sagat_app' \
                     AND state='active' AND wait_event_type='Lock' \
                     AND $1=ANY(pg_blocking_pids(pid)) \
                     AND position('INSERT INTO run_events' in query)>0 LIMIT 1",
            )
            .bind(gate_pid)
            .fetch_optional(&flow.api.db.pool)
            .await
            .expect("between-persist refusal advisory wait witness")
        },
    )
    .await
}

async fn waiting_refusal_full_restore(flow: &Flow, gate_pid: i32) -> i32 {
    crate::support::eventually(
        "actual refusal full transaction waits on the checkpoint restoration query",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT activity.pid FROM pg_stat_activity activity \
                   WHERE activity.datname=current_database() AND activity.usename='sagat_app' \
                     AND activity.state='active' AND activity.wait_event_type='Lock' \
                     AND $1=ANY(pg_blocking_pids(activity.pid)) \
                     AND position('SELECT scope_id, iteration_index, return_instance_id, result_digest, journal_sequence' in activity.query)>0 \
                     AND position('FROM iteration_checkpoints WHERE tenant_id = $1 AND run_id = $2' in activity.query)>0 \
                     AND EXISTS(SELECT 1 FROM pg_locks relation_lock \
                       WHERE relation_lock.pid=activity.pid AND relation_lock.locktype='relation' \
                         AND relation_lock.relation=to_regclass('public.iteration_checkpoints') \
                         AND relation_lock.mode='AccessShareLock' AND NOT relation_lock.granted) \
                   LIMIT 1",
            )
            .bind(gate_pid)
            .fetch_optional(&flow.api.db.pool)
            .await
            .expect("full refusal restoration lock witness")
        },
    )
    .await
}

async fn waiting_collection_drain_outbox(flow: &Flow, gate_pid: i32) -> i32 {
    crate::support::eventually(
        "actual collection drain writer waits on its outbox insertion",
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity \
                   WHERE datname=current_database() AND usename='sagat_app' \
                     AND state='active' AND wait_event_type='Lock' \
                     AND $1=ANY(pg_blocking_pids(pid)) \
                     AND position('INSERT INTO event_outbox' in query)>0 LIMIT 1",
            )
            .bind(gate_pid)
            .fetch_optional(&flow.api.db.pool)
            .await
            .expect("collection drain outbox advisory wait witness")
        },
    )
    .await
}

#[allow(clippy::type_complexity)] // One query captures the atomic public report projection.
async fn assert_successful_refusal(
    flow: &Flow,
    reported: &TaskLease,
    key: &str,
    outboxes_before: i64,
) {
    let state: (
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        bool,
        bool,
        bool,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT run.status,run.stop_reason,run.stop_code,scope.lifecycle,step.status, \
                step.output_digest IS NULL,step.reported_lease_token_hash IS NOT NULL, \
                step.attempt=$4 AND step.generation=$5 \
                  AND step.reported_lease_token_hash=$6, \
                usage.logical_output_bytes,telemetry.head_seq,run.run_version \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
           JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
           JOIN run_instance_usage usage \
             ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
           JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2 \
            AND step.instance_path_nodes=ARRAY['lines']::text[] \
            AND step.instance_path_indices=ARRAY[16]::integer[] AND step.node=$3",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .bind(node(reported))
    .bind(reported.attempt)
    .bind(reported.generation)
    .bind(Token::hash_of(&reported.lease_token))
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("successful refusal projection");
    assert_eq!(state.0, "CANCELLING");
    assert_eq!(state.1.as_deref(), Some("FAILED"));
    assert_eq!(state.2.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(state.3, "OPEN");
    assert_eq!(state.4, "SUCCEEDED");
    assert!(state.5, "the refused successful output was not retained");
    assert!(state.6, "the actual reported lease was persisted");
    assert!(
        state.7,
        "reported lease identity differs from the public lease"
    );
    assert_eq!(state.8, 4_000_000);
    assert_eq!(
        state.9, state.10,
        "the report leaves the final telemetry head"
    );

    let successors: (i64, i64) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 \
             AND instance_path_nodes=ARRAY['lines']::text[] \
             AND instance_path_indices=ARRAY[16]::integer[] AND node='returned'), \
           (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 \
             AND cardinality(instance_path_nodes)=0 AND node='done')",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("refused successors");
    assert_eq!(successors, (0, 0), "no forward successor was released");

    let audit: (String, String, bool) = sqlx::query_as(
        "SELECT audit.actor,audit.reason, \
                audit.trace_id IS NOT DISTINCT FROM run.trace_id \
           FROM audit_records audit JOIN runs run \
             ON run.tenant_id=audit.tenant_id AND run.id=$2 \
          WHERE audit.tenant_id=$1 AND audit.resource_type='run' \
            AND audit.resource_id=$2::text AND audit.action='run.data_limit_exceeded' \
            AND audit.reason='successful scoped output refused by run data limit'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("run data-limit audit");
    assert_eq!(
        audit,
        (
            "system:engine".into(),
            "successful scoped output refused by run data limit".into(),
            true,
        )
    );

    let receipt: (String, i32) = sqlx::query_as(
        "SELECT resource_id,octet_length(request_hash) FROM command_dedupe \
          WHERE tenant_id=$1 AND idempotency_key=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(key)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("original report receipt");
    assert_eq!(receipt.1, 32);
    assert!(
        receipt.0.starts_with("SUCCEEDED|CANCELLING|"),
        "{receipt:?}"
    );
    assert_eq!(
        run_outboxes(flow).await,
        outboxes_before,
        "the report itself emits no outbox message"
    );

    let candidates = flow
        .api
        .executions
        .collection_candidates(&Window::everything(8))
        .await
        .expect("discover the stopped open collection");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.run_id == flow.run && candidate.node.as_str() == "lines"),
        "the durable CANCELLING/OPEN scope remains discoverable: {candidates:?}"
    );
}

async fn start_stopped_limit_drain(flow: &Flow) {
    let draining = flow.scheduler.tick().await;
    assert_eq!(draining.failed, 0, "{draining:?}");
    assert_eq!(draining.collections_advanced, 1, "{draining:?}");
    let drain: (String, String, Option<String>, i64) = sqlx::query_as(
        "SELECT lifecycle,drain_kind,drain_failure_code, \
                (SELECT count(*) FROM event_outbox WHERE tenant_id=scope.tenant_id \
                  AND aggregate_id=scope.run_id AND event_type='collection.drain.started') \
           FROM iteration_scopes scope WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("native stopped-limit drain");
    assert_eq!(
        drain,
        (
            "DRAINING".into(),
            "RUN_DATA_LIMIT".into(),
            Some("RUN_DATA_LIMIT".into()),
            1,
        )
    );
}

async fn drain_and_compensate_every_reservation(flow: &Flow) {
    start_stopped_limit_drain(flow).await;

    let mut released = Vec::new();
    for _ in 0..17 {
        flow.drive().await;
        let tasks = flow.poll().await;
        assert_eq!(
            tasks.len(),
            1,
            "{:?}\n{}",
            summary(&tasks),
            crate::support::scheduler_evidence(&flow.api.db.pool, flow.tenant, flow.run).await
        );
        assert_eq!(
            summary(&tasks),
            vec![row(
                iteration(&tasks[0]),
                "reserve~c",
                "COMPENSATION",
                "line.release"
            )]
        );
        released.push(iteration(&tasks[0]));
        flow.complete(&tasks[0], json!({"released": true})).await;
    }
    // Collection iterations are independent branches; only each branch has reverse topo order.
    released.sort_unstable();
    assert_eq!(released, (0..17).collect::<Vec<_>>());
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");

    let final_state: (
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                    saga.phase,saga.outcome,telemetry.head_seq,run.run_version \
               FROM iteration_scopes scope JOIN scope_sagas saga \
                 ON saga.tenant_id=scope.tenant_id AND saga.run_id=scope.run_id \
                AND saga.owner_path_nodes=scope.owner_path_nodes \
                AND saga.owner_path_indices=scope.owner_path_indices \
                AND saga.owner_node=scope.owner_node \
               JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
               JOIN telemetry_pending telemetry \
                 ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
              WHERE scope.tenant_id=$1 AND scope.run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("finished refused-output compensation");
    assert_eq!(final_state.0, "FAILED");
    assert_eq!(final_state.1, "RUN_DATA_LIMIT");
    assert_eq!(final_state.2.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(final_state.3, "DONE");
    assert_eq!(final_state.4.as_deref(), Some("COMPENSATED"));
    assert_eq!(final_state.5, final_state.6);
    flow.verify().await;
}

#[tokio::test]
async fn a_known_successful_seventeenth_member_is_preserved_without_its_refused_output() {
    let name = format!("scoped-output-refusal-{}", uuid::Uuid::now_v7());
    let (flow, task, output) = collection_at_seventeenth_member(&name, false).await;
    let outboxes_before = run_outboxes(&flow).await;
    let key = format!("refused-success-{}", uuid::Uuid::now_v7());

    let accepted = flow
        .try_complete(&task, output.clone(), &key)
        .await
        .expect("preserve the real successful effect while refusing its output");
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert_successful_refusal(&flow, &task, &key, outboxes_before).await;

    // Rebuild from the journal before exercising the public receipt path.
    flow.verify().await;
    let first_identity = refusal_identity(&flow, &key).await;
    let replay = flow
        .try_complete(&task, output.clone(), &key)
        .await
        .expect("replay the original completion receipt");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_eq!(refusal_identity(&flow, &key).await, first_identity);

    let wrong_fingerprint = flow
        .try_complete(&task, json!({"different": true}), &key)
        .await
        .expect_err("the receipt is bound to the original successful report");
    assert_eq!(wrong_fingerprint.code(), tonic::Code::Aborted);
    let stale = flow
        .try_complete(
            &task,
            output,
            &format!("stale-refusal-{}", uuid::Uuid::now_v7()),
        )
        .await
        .expect_err("a new command cannot reuse the completed lease fence");
    assert_eq!(stale.code(), tonic::Code::Aborted);
    assert_eq!(refusal_identity(&flow, &key).await, first_identity);

    drain_and_compensate_every_reservation(&flow).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_scoped_output_refusal_rechecks_binding_after_the_checked_path_is_dropped() {
    let name = format!("refusal-bind-{}", uuid::Uuid::now_v7());
    assert!(name.len() <= 63, "fixture name must remain a valid slug");
    let (flow, task, output) = collection_at_seventeenth_member(&name, false).await;
    let candidate_bytes = output.to_string().len();
    let premise: (i64, String, i64) = sqlx::query_as(
        "SELECT usage.logical_output_bytes,scope.lifecycle, \
                (SELECT count(*) FROM steps retained \
                  WHERE retained.tenant_id=run.tenant_id AND retained.run_id=run.id \
                    AND retained.instance_path_nodes=ARRAY['lines']::text[] \
                    AND retained.node='reserve' AND retained.status='SUCCEEDED' \
                    AND retained.output_digest IS NOT NULL) \
           FROM runs run JOIN run_instance_usage usage \
             ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
           JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("real retained-output refusal premise");
    assert_eq!(premise, (4_000_000, "OPEN".into(), 16));
    assert_eq!(candidate_bytes, 250_000);
    assert!(
        usize::try_from(premise.0).expect("nonnegative retained bytes") + candidate_bytes
            > MAX_RUN_OUTPUT_BYTES,
        "the genuine seventeenth output must select the refusal path"
    );

    let namespace: uuid::Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("run namespace");
    let binding_before: (i64, i64) = sqlx::query_as(
        "SELECT count(*), \
                coalesce(sum((octet_length(pattern)+octet_length(subject))::bigint),0)::bigint \
           FROM worker_bindings \
          WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(namespace)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("current exact binding");
    assert!(binding_before.0 > 0, "the worker must begin bound");
    assert!(
        binding_before.1 > 0,
        "the worker binding must have physical retained bytes"
    );
    let sessions_before: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM worker_sessions \
          WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker' ORDER BY id",
    )
    .bind(flow.tenant.as_uuid())
    .bind(namespace)
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("current exact worker session");
    assert_eq!(sessions_before.len(), 1);
    let session = sessions_before[0];
    let step = uuid::Uuid::parse_str(&task.step_id).expect("public refusal step id");
    let lease_before = refusal_lease_state(&flow, step).await;
    assert_eq!(lease_before.status, "LEASED");
    assert!(
        lease_before.lease_token_hash == Token::hash_of(&task.lease_token),
        "the public lease token matches its stored fence"
    );
    assert_eq!(lease_before.attempt, task.attempt);
    assert_eq!(lease_before.generation, task.generation);
    assert_eq!(lease_before.leased_by, session.to_string());
    assert!(!lease_before.expired, "the public lease starts live");

    let key = format!("fresh-binding-refusal-{}", uuid::Uuid::now_v7());
    let outboxes_before = run_outboxes(&flow).await;
    let before = refusal_transaction_snapshot(&flow).await;
    let retained_content_before = before["tenantRetainedUsage"]["content_bytes"]
        .as_i64()
        .expect("pre-unbind retained content bytes");
    let unbound_audits_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records \
          WHERE tenant_id=$1 AND resource_type='worker_binding' AND resource_id=$2::text \
            AND action='worker.unbound' AND reason='scope-worker'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(namespace)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("worker-unbound audit baseline");

    let mut report_client = WorkerServiceClient::connect(flow.api.grpc_url.clone())
        .await
        .expect("connect the actual worker transport");
    let report_trace = sagat_telemetry::TraceId::generate();
    let report_trace_id = report_trace.to_string();
    let report_traceparent =
        sagat_telemetry::traceparent(report_trace, sagat_telemetry::SpanId::generate(), true);
    let mut report_request = worker_request(
        &flow.api,
        CompleteStepRequest {
            task_id: task.task_id.clone(),
            run_id: task.run_id.clone(),
            step_id: task.step_id.clone(),
            attempt: task.attempt,
            generation: task.generation,
            lease_token: task.lease_token.clone(),
            idempotency_key: key.clone(),
            output_json: output.to_string(),
            output_blob_ref: String::new(),
            output_sha256: String::new(),
            traceparent: String::new(),
            instance: task.instance.clone(),
        },
    );
    report_request.metadata_mut().insert(
        "traceparent",
        tonic::metadata::MetadataValue::try_from(report_traceparent)
            .expect("valid generated report traceparent"),
    );

    let mut gate = flow
        .api
        .db
        .pool
        .begin()
        .await
        .expect("checkpoint table gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("checkpoint gate backend pid");
    sqlx::query("LOCK TABLE public.iteration_checkpoints IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *gate)
        .await
        .expect("hold full-only checkpoint restoration gate");

    let report_started_at = time::OffsetDateTime::now_utc();
    let reporting = report_client.complete_step(report_request);
    tokio::pin!(reporting);
    let observe = async {
        // The exact-path report does not read iteration_checkpoints. Reaching this full-only query
        // proves it selected refusal and dropped its path transaction before current full authority.
        let writer_pid = waiting_refusal_full_restore(&flow, gate_pid).await;
        assert_ne!(writer_pid, gate_pid);
        assert!(
            refusal_transaction_snapshot(&flow).await == before,
            "the report has no visible effect while fresh full restoration is blocked"
        );

        let unbind_started_at = time::OffsetDateTime::now_utc();
        let (status, trace_id, body, unbind_finished_at) =
            tokio::time::timeout(Duration::from_secs(5), async {
                let response = flow
                    .api
                    .as_user(
                        reqwest::Method::DELETE,
                        &format!("/namespaces/{namespace}/worker-bindings/scope-worker"),
                        "alice",
                        "acme",
                    )
                    .send()
                    .await
                    .expect("actual worker unbind request");
                let status = response.status();
                let trace_id = response
                    .headers()
                    .get("x-trace-id")
                    .and_then(|value| value.to_str().ok())
                    .expect("native unbind response trace id")
                    .to_owned();
                let body: Value = response.json().await.expect("worker unbind response");
                (status, trace_id, body, time::OffsetDateTime::now_utc())
            })
            .await
            .expect("native unbind must complete while full restoration is blocked");
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(body["sessionsClosed"], 1);
        assert_eq!(body["leasesReleased"], 1);

        let mut traced_audits: Vec<RecordedAudit> = sqlx::query_as(
            "SELECT id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at \
               FROM audit_records WHERE tenant_id=$1 AND trace_id=$2 ORDER BY id",
        )
        .bind(flow.tenant.as_uuid())
        .bind(&trace_id)
        .fetch_all(&flow.api.db.pool)
        .await
        .expect("native unbind traced audit");
        assert!(
            traced_audits.len() == 1,
            "the native unbind request appends exactly one audit"
        );
        let unbound_audit = traced_audits.pop().expect("one native unbind audit");
        assert!(
            unbound_audit.id != uuid::Uuid::nil(),
            "the native unbind audit has an identity"
        );
        assert!(
            unbound_audit.tenant_id == flow.tenant.as_uuid(),
            "the native unbind audit belongs to the exact tenant"
        );
        assert!(
            unbound_audit.actor == "alice",
            "the native unbind audit records the authenticated administrator"
        );
        assert!(
            unbound_audit.action == "worker.unbound",
            "the native unbind audit records the exact action"
        );
        assert!(
            unbound_audit.resource_type == "worker_binding",
            "the native unbind audit records the exact resource type"
        );
        assert!(
            unbound_audit.resource_id == namespace.to_string(),
            "the native unbind audit records the exact namespace"
        );
        assert!(
            unbound_audit.reason.as_deref() == Some("scope-worker"),
            "the native unbind audit records the exact worker subject"
        );
        assert!(
            unbound_audit.trace_id.as_deref() == Some(trace_id.as_str()),
            "the native unbind audit carries the response trace"
        );
        assert!(
            unbound_audit.created_at >= unbind_started_at
                && unbound_audit.created_at <= unbind_finished_at,
            "the native unbind audit timestamp belongs to this request"
        );

        let authority_after: (i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT count(*) FROM worker_bindings \
                 WHERE tenant_id=$1 AND namespace_id=$2 AND subject='scope-worker'), \
               (SELECT count(*) FROM worker_sessions \
                 WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 AND subject='scope-worker'), \
               (SELECT count(*) FROM audit_records \
                 WHERE tenant_id=$1 AND resource_type='worker_binding' \
                   AND resource_id=$2::text AND action='worker.unbound' \
                   AND reason='scope-worker')",
        )
        .bind(flow.tenant.as_uuid())
        .bind(namespace)
        .bind(session)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("committed exact binding withdrawal");
        assert_eq!(
            authority_after,
            (0, 0, unbound_audits_before + 1),
            "the exact binding and session are gone with one native audit"
        );
        let lease_after = refusal_lease_state(&flow, step).await;
        assert_eq!(lease_after.status, lease_before.status);
        assert!(
            lease_after.lease_token_hash == lease_before.lease_token_hash,
            "native unbind preserves the stored lease fence"
        );
        assert_eq!(lease_after.attempt, lease_before.attempt);
        assert_eq!(lease_after.generation, lease_before.generation);
        assert_eq!(lease_after.leased_by, lease_before.leased_by);
        assert!(
            lease_after.expired,
            "native unbind withdraws the live lease"
        );
        assert!(
            lease_after.lease_expires_at < lease_before.lease_expires_at,
            "native unbind must shorten the actual lease expiry"
        );
        assert!(lease_after.updated_at >= lease_before.updated_at);

        let after_unbind = refusal_transaction_snapshot(&flow).await;
        let retained_content_after = after_unbind["tenantRetainedUsage"]["content_bytes"]
            .as_i64()
            .expect("post-unbind retained content bytes");
        assert!(
            retained_content_after == retained_content_before - binding_before.1,
            "native unbind refunds exactly the deleted binding's physical content bytes"
        );
        assert!(
            after_expected_binding_withdrawal(&before, &after_unbind, step, unbound_audit.id,)
                == before,
            "unbind changes only its traced audit, target lease timestamps, and exact binding-byte refund"
        );
        let still_blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity \
               WHERE activity.pid=$1 AND activity.datname=current_database() \
                 AND activity.usename='sagat_app' AND activity.state='active' \
                 AND activity.wait_event_type='Lock' \
                 AND $2=ANY(pg_blocking_pids(activity.pid)) \
                 AND position('SELECT scope_id, iteration_index, return_instance_id, result_digest, journal_sequence' in activity.query)>0 \
                 AND position('FROM iteration_checkpoints WHERE tenant_id = $1 AND run_id = $2' in activity.query)>0 \
                 AND EXISTS(SELECT 1 FROM pg_locks relation_lock \
                   WHERE relation_lock.pid=activity.pid AND relation_lock.locktype='relation' \
                     AND relation_lock.relation=to_regclass('public.iteration_checkpoints') \
                     AND relation_lock.mode='AccessShareLock' AND NOT relation_lock.granted))",
        )
        .bind(writer_pid)
        .bind(gate_pid)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("full restoration remains blocked after binding withdrawal");
        assert!(
            still_blocked,
            "the same runtime PID must remain on the exact full-only relation lock"
        );
        gate.rollback()
            .await
            .expect("release only the owned checkpoint restoration gate");
        after_unbind
    };
    tokio::pin!(observe);
    let (refused, after_unbind) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(&mut reporting, &mut observe)
    })
    .await
    .expect("report and native authority withdrawal must finish");

    let report_finished_at = time::OffsetDateTime::now_utc();
    let refusal = match refused {
        Err(refusal) => refusal,
        Ok(_) => panic!("fresh full authority must reject the report"),
    };
    assert_eq!(refusal.code(), tonic::Code::PermissionDenied);
    assert_eq!(crate::support::worker::machine_code(&refusal), "NOT_BOUND");

    let mut denial_audits: Vec<RecordedAudit> = sqlx::query_as(
        "SELECT id,tenant_id,actor,action,resource_type,resource_id,reason,trace_id,created_at \
           FROM audit_records WHERE tenant_id=$1 AND trace_id=$2 ORDER BY id",
    )
    .bind(flow.tenant.as_uuid())
    .bind(&report_trace_id)
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("traced fresh-authority denial audit");
    assert!(
        denial_audits.len() == 1,
        "the rejected report appends exactly one traced policy audit"
    );
    let denial_audit = denial_audits.pop().expect("one traced policy audit");
    let denial_reason = sagat_application::error::AccessDenial::Refused("NOT_BOUND").reason();
    assert!(
        denial_audit.id != uuid::Uuid::nil(),
        "the policy denial audit has an identity"
    );
    assert!(
        denial_audit.tenant_id == flow.tenant.as_uuid(),
        "the policy denial audit belongs to the exact tenant"
    );
    assert!(
        denial_audit.actor == "scope-worker",
        "the policy denial audit records the authenticated worker"
    );
    assert!(
        denial_audit.action == "access.denied.permission",
        "the policy denial audit records the exact action"
    );
    assert!(
        denial_audit.resource_type == "endpoint",
        "the policy denial audit records the exact resource type"
    );
    assert!(
        denial_audit.resource_id == "grpc WorkerService/CompleteStep",
        "the policy denial audit records the exact RPC"
    );
    assert!(
        denial_audit.reason.as_deref() == Some(denial_reason.as_str()),
        "the policy denial audit records the closed refusal reason"
    );
    assert!(
        denial_audit.trace_id.as_deref() == Some(report_trace_id.as_str()),
        "the policy denial audit carries the supplied report trace"
    );
    assert!(
        denial_audit.created_at >= report_started_at
            && denial_audit.created_at <= report_finished_at,
        "the policy denial audit timestamp belongs to this report"
    );
    let after_denial = refusal_transaction_snapshot(&flow).await;
    assert!(
        without_exact_audit(&after_denial, denial_audit.id) == after_unbind,
        "the rejected report adds only its exact required policy denial audit"
    );
    let rejected: RejectedRefusalState = sqlx::query_as(
        "SELECT run.status AS run_status,run.stop_reason,run.stop_code, \
                step.status AS step_status,step.output_digest IS NULL AS output_absent, \
                step.reported_lease_token_hash IS NULL AS report_absent, \
                usage.logical_output_bytes,telemetry.head_seq,run.run_version, \
                (SELECT count(*) FROM command_dedupe \
                  WHERE tenant_id=run.tenant_id AND idempotency_key=$4) AS receipts, \
                (SELECT count(*) FROM audit_records audit \
                  WHERE audit.tenant_id=run.tenant_id AND audit.resource_type='run' \
                    AND audit.resource_id=run.id::text \
                    AND audit.action='run.data_limit_exceeded' \
                    AND audit.reason='successful scoped output refused by run data limit') \
                  AS refusal_audits \
           FROM runs run JOIN steps step \
             ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
           JOIN run_instance_usage usage \
             ON usage.tenant_id=run.tenant_id AND usage.run_id=run.id \
           JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
          WHERE run.tenant_id=$1 AND run.id=$2 AND step.step_id=$3",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .bind(step)
    .bind(&key)
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("rejected refusal projection");
    assert_eq!(rejected.run_status, "RUNNING");
    assert_eq!(rejected.stop_reason, None);
    assert_eq!(rejected.stop_code, None);
    assert_eq!(rejected.step_status, "LEASED");
    assert!(
        rejected.output_absent,
        "the refused candidate was not retained"
    );
    assert!(
        rejected.report_absent,
        "the rejected lease was not reported"
    );
    assert_eq!(rejected.logical_output_bytes, 4_000_000);
    assert_eq!(
        rejected.head_seq, rejected.run_version,
        "the journal head is unchanged"
    );
    assert_eq!(rejected.receipts, 0, "no report receipt was stored");
    assert_eq!(rejected.refusal_audits, 0, "no refusal audit was stored");
    assert_eq!(run_outboxes(&flow).await, outboxes_before);
    let cold_store = PgRunStore::new(
        flow.api.api_pool.clone(),
        sagat_application::execution::DefinitionCache::new(
            std::sync::Arc::new(sagat_infra_ir::JsonIrCodec),
            8,
        ),
    )
    .with_scoped_worker_protocol();
    let replay = cold_store.verify_run(flow.tenant, flow.run).await;
    assert!(
        replay.is_ok(),
        "a fresh store rebuilds the rejected report state"
    );
}

mod refused_output_compensation {
    use std::collections::BTreeSet;

    use super::*;

    const MISSING_OUTPUT_CODE: &str = "FIXTURE_RESERVATION_OUTPUT_REQUIRED";
    const RESOLUTION_REASON: &str = "missing reservation output prevents fixture release";

    #[derive(sqlx::FromRow)]
    struct OpenObligation {
        compensation_status: String,
        waiting_kind: Option<String>,
        failure_code: Option<String>,
        run_status: String,
        saga_phase: String,
        saga_outcome: Option<String>,
        forward_status: String,
        forward_output_absent: bool,
        open_entries: i64,
        false_finishes: i64,
        review_events: i64,
    }

    #[derive(sqlx::FromRow)]
    struct ResolvedObligation {
        compensation_status: String,
        failure_code: Option<String>,
        forward_status: String,
        forward_output_absent: bool,
        run_status: String,
        stop_reason: Option<String>,
        stop_code: Option<String>,
        saga_phase: String,
        saga_outcome: Option<String>,
    }

    struct ReservationOutput {
        reservation_id: String,
    }

    struct ForwardOutput {
        output: ReservationOutput,
    }

    struct CompensationSteps {
        reserve: ForwardOutput,
    }

    struct ReleaseTaskInput {
        steps: CompensationSteps,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ReleaseError {
        RequiredReservationOutputMissing,
        MalformedTaskInput,
        UnknownReservation(String),
    }

    impl ReleaseTaskInput {
        fn parse(input_json: &str) -> Result<Self, ReleaseError> {
            let document: Value =
                serde_json::from_str(input_json).map_err(|_| ReleaseError::MalformedTaskInput)?;
            let steps = document["steps"]
                .as_object()
                .ok_or(ReleaseError::MalformedTaskInput)?;
            let reserve = steps
                .get("reserve")
                .ok_or(ReleaseError::RequiredReservationOutputMissing)?;
            let reservation_id = reserve["output"]["reservationId"]
                .as_str()
                .ok_or(ReleaseError::MalformedTaskInput)?;
            Ok(Self {
                steps: CompensationSteps {
                    reserve: ForwardOutput {
                        output: ReservationOutput {
                            reservation_id: reservation_id.to_owned(),
                        },
                    },
                },
            })
        }
    }

    fn reservation_id(index: u32) -> String {
        format!("reservation-{index:02}")
    }

    fn reservation_output(index: u32) -> Value {
        let mut output = json!({
            "reservationId": reservation_id(index),
            "padding": "",
        });
        let padding = 250_000_usize
            .checked_sub(output.to_string().len())
            .expect("the reservation envelope fits below the report ceiling");
        output["padding"] = Value::String("x".repeat(padding));
        assert_eq!(
            output.to_string().len(),
            250_000,
            "the real reservation result contributes the intended retained bytes"
        );
        output
    }

    fn release_reservation(
        input_json: &str,
        reservations: &mut BTreeSet<String>,
    ) -> Result<String, ReleaseError> {
        let input = ReleaseTaskInput::parse(input_json)?;
        let reservation = input.steps.reserve.output.reservation_id;
        if reservations.remove(&reservation) {
            Ok(reservation)
        } else {
            Err(ReleaseError::UnknownReservation(reservation))
        }
    }

    async fn run_available_compensations(
        flow: &Flow,
        reservations: &mut BTreeSet<String>,
        released: &mut BTreeSet<String>,
        missing: &mut Option<TaskLease>,
    ) {
        for _ in 0..17 {
            flow.drive().await;
            let tasks = flow.poll().await;
            if tasks.is_empty() {
                break;
            }
            assert_eq!(tasks.len(), 1, "{:?}", summary(&tasks));
            let task = &tasks[0];
            assert_eq!(
                summary(&tasks),
                vec![row(
                    iteration(task),
                    "reserve~c",
                    "COMPENSATION",
                    "line.release"
                )]
            );
            match release_reservation(&task.input_json, reservations) {
                Ok(reservation) => {
                    assert_eq!(reservation, reservation_id(iteration(task)));
                    assert!(released.insert(reservation.clone()), "released twice");
                    flow.complete(task, json!({"releasedReservationId": reservation}))
                        .await;
                }
                Err(ReleaseError::RequiredReservationOutputMissing) => {
                    assert!(
                        missing.is_none(),
                        "the refused compensation was leased twice"
                    );
                    assert_eq!(iteration(task), 16);
                    let document: Value =
                        serde_json::from_str(&task.input_json).expect("valid task input document");
                    assert!(
                        document.pointer("/steps/reserve").is_none(),
                        "the refused forward output must not be defaulted or synthesized"
                    );
                    assert_eq!(task.target_node, "reserve");
                    assert!(task.idempotency_key.starts_with("compensation:"));
                    let instance = task.instance.as_ref().expect("compensation instance");
                    let target = task.target_instance.as_ref().expect("target instance");
                    assert_eq!(target.definition_node_id, "reserve");
                    assert_eq!(target.instance_path, instance.instance_path);
                    assert!(reservations.contains(&reservation_id(16)));
                    flow.fail(task, MISSING_OUTPUT_CODE, false).await;
                    *missing = Some(task.clone());
                }
                Err(other) => panic!("unexpected release handler failure: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_refused_required_output_keeps_its_compensation_open_until_audited_resolution() {
        let name = format!("required-refused-output-{}", uuid::Uuid::now_v7());
        let (flow, forward, output) =
            collection_at_seventeenth_member_with_output(&name, false, reservation_output).await;
        let mut reservations = (0..17).map(reservation_id).collect::<BTreeSet<_>>();
        let outboxes_before = run_outboxes(&flow).await;
        let key = format!("required-refused-output-{}", uuid::Uuid::now_v7());

        let accepted = flow
            .try_complete(&forward, output, &key)
            .await
            .expect("record the known forward success without its oversized result");
        assert!(!accepted.already_applied);
        assert_eq!(accepted.step_status, "SUCCEEDED");
        assert_eq!(accepted.run_status, "CANCELLING");
        assert_successful_refusal(&flow, &forward, &key, outboxes_before).await;
        let retained: (i64, i64) = sqlx::query_as(
            "SELECT count(*),coalesce(sum(octet_length(payload.content)),0)::bigint \
               FROM steps step JOIN payloads payload \
                 ON payload.tenant_id=step.tenant_id AND payload.digest=step.output_digest \
              WHERE step.tenant_id=$1 AND step.run_id=$2 \
                AND step.instance_path_nodes=ARRAY['lines']::text[] \
                AND step.node='reserve'",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("retained reservation results");
        assert_eq!(retained, (16, 4_000_000));

        start_stopped_limit_drain(&flow).await;
        let mut released = BTreeSet::new();
        let mut missing = None;
        run_available_compensations(&flow, &mut reservations, &mut released, &mut missing).await;
        let missing = missing.expect("the refused member's compensation reported its input error");
        assert_eq!(missing.target_idempotency_key, forward.idempotency_key);
        assert_eq!(missing.target_instance, forward.instance);
        flow.drive().await;

        let open: OpenObligation = sqlx::query_as(
            "SELECT compensation.status AS compensation_status,compensation.waiting_kind,compensation.failure_code, \
                    run.status AS run_status,saga.phase AS saga_phase,saga.outcome AS saga_outcome,forward.status AS forward_status, \
                    forward.output_digest IS NULL AS forward_output_absent, \
                    (SELECT count(*) FROM scope_saga_entries entry \
                      WHERE entry.tenant_id=run.tenant_id AND entry.run_id=run.id \
                        AND entry.iteration_index=16 AND entry.node='reserve' \
                        AND entry.kind='COMPENSATE') AS open_entries, \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.instance_path_nodes=ARRAY['lines']::text[] \
                        AND event.instance_path_indices=ARRAY[16]::integer[] \
                        AND event.node='reserve~c' \
                        AND event.event_type IN ('step.completed','step.skipped')) AS false_finishes, \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
                        AND event.instance_path_nodes=ARRAY['lines']::text[] \
                        AND event.instance_path_indices=ARRAY[16]::integer[] \
                        AND event.node='reserve~c' \
                        AND event.event_type='step.needs_review') AS review_events \
               FROM runs run JOIN steps compensation \
                 ON compensation.tenant_id=run.tenant_id AND compensation.run_id=run.id \
                AND compensation.instance_path_nodes=ARRAY['lines']::text[] \
                AND compensation.instance_path_indices=ARRAY[16]::integer[] \
                AND compensation.node='reserve~c' \
               JOIN steps forward \
                 ON forward.tenant_id=run.tenant_id AND forward.run_id=run.id \
                AND forward.instance_path_nodes=ARRAY['lines']::text[] \
                AND forward.instance_path_indices=ARRAY[16]::integer[] \
                AND forward.node='reserve' \
               JOIN scope_sagas saga ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
              WHERE run.tenant_id=$1 AND run.id=$2",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("open missing-output obligation");
        assert_eq!(open.compensation_status, "WAITING");
        assert_eq!(open.waiting_kind.as_deref(), Some("OPERATOR"));
        assert_eq!(open.failure_code.as_deref(), Some(MISSING_OUTPUT_CODE));
        assert_eq!(open.run_status, "MANUAL_REVIEW");
        assert_eq!(open.saga_phase, "COMPENSATING");
        assert_eq!(open.saga_outcome, None);
        assert_eq!(open.forward_status, "SUCCEEDED");
        assert!(
            open.forward_output_absent,
            "the refused forward output stays absent"
        );
        assert_eq!(
            open.open_entries, 1,
            "the saga retains the exact compensation entry"
        );
        assert_eq!(
            open.false_finishes, 0,
            "no completion or skip was manufactured"
        );
        assert_eq!(
            open.review_events, 1,
            "one typed failure waits for the operator"
        );
        assert_eq!(
            flow.drive().await,
            0,
            "the open obligation cannot self-settle"
        );
        assert!(flow.poll().await.is_empty());
        assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
        let finished_before_resolution: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_events \
              WHERE tenant_id=$1 AND run_id=$2 AND event_type='run.finished'",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("unfinished run evidence");
        assert_eq!(finished_before_resolution, 0);
        flow.verify().await;

        let (status, body) = flow
            .operate(json!({
                "action": "RESOLVE_STEP",
                "stepId": "reserve~c",
                "outcome": "FAILED",
                "errorCode": MISSING_OUTPUT_CODE,
                "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 16}],
                "reason": RESOLUTION_REASON,
            }))
            .await;
        assert_eq!(status, reqwest::StatusCode::ACCEPTED, "{body}");
        assert_ne!(body["runStatus"], "COMPENSATED");
        let audit: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT actor,resource_type,resource_id,reason FROM audit_records \
              WHERE tenant_id=$1 AND action='saga.step_resolved' AND resource_id=$2",
        )
        .bind(flow.tenant.as_uuid())
        .bind(format!("{}/lines[16]/reserve~c", flow.run))
        .fetch_all(&flow.api.db.pool)
        .await
        .expect("audited exact compensation resolution");
        assert_eq!(
            audit,
            vec![(
                "alice".into(),
                "run".into(),
                format!("{}/lines[16]/reserve~c", flow.run),
                Some(format!(
                    "declared FAILED({MISSING_OUTPUT_CODE}): {RESOLUTION_REASON}"
                )),
            )]
        );
        let resolution_events: (i64, i64) = sqlx::query_as(
            "SELECT \
                count(*) FILTER (WHERE event_type='step.resolved'), \
                count(*) FILTER (WHERE event_type IN ('step.completed','step.skipped')) \
               FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
                 AND instance_path_nodes=ARRAY['lines']::text[] \
                 AND instance_path_indices=ARRAY[16]::integer[] AND node='reserve~c'",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("resolved missing-output journal events");
        assert_eq!(resolution_events, (1, 0));

        let mut missing_after_resolution = Some(missing);
        run_available_compensations(
            &flow,
            &mut reservations,
            &mut released,
            &mut missing_after_resolution,
        )
        .await;
        flow.drive().await;
        assert_eq!(
            released.len(),
            16,
            "every retained result reached the same handler"
        );
        assert_eq!(reservations, BTreeSet::from([reservation_id(16)]));

        let final_state: ResolvedObligation = sqlx::query_as(
            "SELECT compensation.status AS compensation_status,compensation.failure_code,forward.status AS forward_status, \
                    forward.output_digest IS NULL AS forward_output_absent,run.status AS run_status,run.stop_reason,run.stop_code, \
                    saga.phase AS saga_phase,saga.outcome AS saga_outcome \
               FROM runs run JOIN steps compensation \
                 ON compensation.tenant_id=run.tenant_id AND compensation.run_id=run.id \
                AND compensation.instance_path_nodes=ARRAY['lines']::text[] \
                AND compensation.instance_path_indices=ARRAY[16]::integer[] \
                AND compensation.node='reserve~c' \
               JOIN steps forward \
                 ON forward.tenant_id=run.tenant_id AND forward.run_id=run.id \
                AND forward.instance_path_nodes=ARRAY['lines']::text[] \
                AND forward.instance_path_indices=ARRAY[16]::integer[] \
                AND forward.node='reserve' \
               JOIN scope_sagas saga ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
              WHERE run.tenant_id=$1 AND run.id=$2",
        )
        .bind(flow.tenant.as_uuid())
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("final failed compensation state");
        assert_eq!(final_state.compensation_status, "FAILED");
        assert_eq!(
            final_state.failure_code.as_deref(),
            Some(MISSING_OUTPUT_CODE)
        );
        assert_eq!(final_state.forward_status, "SUCCEEDED");
        assert!(final_state.forward_output_absent);
        assert_eq!(final_state.run_status, "COMPENSATION_FAILED");
        assert_eq!(final_state.stop_reason.as_deref(), Some("FAILED"));
        assert_eq!(final_state.stop_code.as_deref(), Some("RUN_DATA_LIMIT"));
        assert_eq!(final_state.saga_phase, "DONE");
        assert_eq!(
            final_state.saga_outcome.as_deref(),
            Some("COMPENSATION_FAILED")
        );
        flow.verify().await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_between_persist_fault_rolls_back_the_whole_refusal_and_same_lease_retries() {
    let name = format!("refusal-midpersist-{}", uuid::Uuid::now_v7());
    let (flow, task, output) = collection_at_seventeenth_member(&name, false).await;
    let outboxes_before = run_outboxes(&flow).await;
    let key = format!("between-persist-refusal-{}", uuid::Uuid::now_v7());
    let before = refusal_transaction_snapshot(&flow).await;
    let fault_key = positive_advisory_key();
    install_refusal_scoped_persist_fault(&flow.api.db.pool, fault_key).await;
    let mut gate = flow
        .api
        .db
        .pool
        .begin()
        .await
        .expect("owned between-persist refusal gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("between-persist refusal gate backend pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(fault_key)
        .execute(&mut *gate)
        .await
        .expect("hold between-persist refusal gate");
    let (_fault_log_guard, fault_logs) = crate::support::logs::capture_at(tracing::Level::ERROR);

    let reporting = flow.try_complete(&task, output.clone(), &key);
    tokio::pin!(reporting);
    let observe = async {
        // The trigger is on the exact first scoped journal write. Reaching it proves the root-only
        // persist ran first, while PostgreSQL still hides every uncommitted change from this reader.
        let writer_pid = waiting_refusal_scoped_persist(&flow, gate_pid).await;
        assert_ne!(writer_pid, gate_pid);
        assert_eq!(
            refusal_transaction_snapshot(&flow).await,
            before,
            "a second connection sees the old complete snapshot while the writer is between the root-only and scoped persists"
        );
        let still_blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
               WHERE pid=$1 AND state='active' AND wait_event_type='Lock' \
                 AND $2=ANY(pg_blocking_pids(pid)) \
                 AND position('INSERT INTO run_events' in query)>0)",
        )
        .bind(writer_pid)
        .bind(gate_pid)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("between-persist writer remains blocked after the complete observation");
        gate.rollback()
            .await
            .expect("release only the owned between-persist refusal gate");
        still_blocked
    };
    tokio::pin!(observe);
    let (failed, blocked_through_observation) = tokio::join!(&mut reporting, &mut observe);
    assert!(
        blocked_through_observation,
        "the actual scoped persist writer must remain blocked until observation releases the gate"
    );
    let failure = failed.expect_err("the between-persist fault returns no fabricated StepAck");
    assert_eq!(failure.code(), tonic::Code::Internal);
    assert_eq!(crate::support::worker::machine_code(&failure), "INTERNAL");
    let fault_log = fault_logs.text();
    assert!(
        fault_log.contains("test-only scoped output refusal between-persist abort"),
        "the public failure must come from the labelled scoped-persist trigger: {fault_log}"
    );
    assert!(
        !fault_log.contains("statement timeout"),
        "the report timed out before the labelled scoped-persist fault: {fault_log}"
    );
    assert_eq!(
        refusal_transaction_snapshot(&flow).await,
        before,
        "the labelled between-persist failure rolls back the root stop and every scoped report effect"
    );

    remove_refusal_scoped_persist_fault(&flow.api.db.pool).await;
    let accepted = flow
        .try_complete(&task, output.clone(), &key)
        .await
        .expect("the identical public report and lease apply after the fault is removed");
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert_successful_refusal(&flow, &task, &key, outboxes_before).await;
    flow.verify().await;

    let committed = refusal_transaction_snapshot(&flow).await;
    let identity = refusal_identity(&flow, &key).await;
    let replay = flow
        .try_complete(&task, output, &key)
        .await
        .expect("the committed retry returns its original truthful ACK");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_eq!(refusal_identity(&flow, &key).await, identity);
    assert_eq!(refusal_transaction_snapshot(&flow).await, committed);

    drain_and_compensate_every_reservation(&flow).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_drain_outbox_fault_preserves_the_committed_report_and_one_retry_compensates() {
    let name = format!("refusal-drain-{}", uuid::Uuid::now_v7());
    let (flow, task, output) = collection_at_seventeenth_member(&name, false).await;
    let outboxes_before = run_outboxes(&flow).await;
    let key = format!("drain-outbox-refusal-{}", uuid::Uuid::now_v7());
    let accepted = flow
        .try_complete(&task, output.clone(), &key)
        .await
        .expect("commit the successful report before the drain fault");
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert_successful_refusal(&flow, &task, &key, outboxes_before).await;
    flow.verify().await;
    let committed_report = refusal_transaction_snapshot(&flow).await;
    let report_identity = refusal_identity(&flow, &key).await;

    let fault_key = positive_advisory_key();
    install_collection_drain_outbox_fault(&flow.api.db.pool, fault_key).await;
    let mut gate = flow
        .api
        .db
        .pool
        .begin()
        .await
        .expect("owned collection drain outbox gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("collection drain outbox gate backend pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(fault_key)
        .execute(&mut *gate)
        .await
        .expect("hold collection drain outbox gate");
    let (_fault_log_guard, fault_logs) = crate::support::logs::capture_at(tracing::Level::ERROR);

    let draining = flow.scheduler.tick();
    tokio::pin!(draining);
    let observe = async {
        let writer_pid = waiting_collection_drain_outbox(&flow, gate_pid).await;
        assert_ne!(writer_pid, gate_pid);
        assert_eq!(
            refusal_transaction_snapshot(&flow).await,
            committed_report,
            "the committed report, stop and ACK remain exact while the actual drain outbox writer is blocked"
        );
        let still_blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
               WHERE pid=$1 AND state='active' AND wait_event_type='Lock' \
                 AND $2=ANY(pg_blocking_pids(pid)) \
                 AND position('INSERT INTO event_outbox' in query)>0)",
        )
        .bind(writer_pid)
        .bind(gate_pid)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("collection drain outbox writer remains blocked after observation");
        gate.rollback()
            .await
            .expect("release only the owned collection drain outbox gate");
        still_blocked
    };
    tokio::pin!(observe);
    let (failed, blocked_through_observation) = tokio::join!(&mut draining, &mut observe);
    assert!(
        blocked_through_observation,
        "the actual drain outbox writer must remain blocked until observation releases the gate"
    );
    assert_eq!(
        failed.failed, 1,
        "the labelled drain outbox fault is observed"
    );
    assert_eq!(
        failed.collections_advanced, 0,
        "the rejected outbox insertion cannot count a drain transition"
    );
    let fault_log = fault_logs.text();
    assert!(
        fault_log.contains("test-only collection drain outbox abort"),
        "the scheduler failure must come from the labelled drain outbox trigger: {fault_log}"
    );
    assert!(
        !fault_log.contains("statement timeout"),
        "the drain timed out before the labelled outbox fault: {fault_log}"
    );
    assert_eq!(
        refusal_transaction_snapshot(&flow).await,
        committed_report,
        "the outbox fault rolls back the entire drain and preserves the earlier report transaction"
    );
    assert_eq!(refusal_identity(&flow, &key).await, report_identity);
    assert_successful_refusal(&flow, &task, &key, outboxes_before).await;
    flow.verify().await;

    let replay = flow
        .try_complete(&task, output, &key)
        .await
        .expect("the original ACK remains replayable after the rejected drain");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_eq!(refusal_identity(&flow, &key).await, report_identity);
    assert_eq!(refusal_transaction_snapshot(&flow).await, committed_report);

    remove_collection_drain_outbox_fault(&flow.api.db.pool).await;
    drain_and_compensate_every_reservation(&flow).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_late_receipt_fault_rolls_back_the_whole_successful_refusal_and_same_key_retries() {
    let name = format!("scoped-refusal-fault-{}", uuid::Uuid::now_v7());
    let (flow, task, output) = collection_at_seventeenth_member(&name, false).await;
    let outboxes_before = run_outboxes(&flow).await;
    let key = format!("refusal-fault-{}", uuid::Uuid::now_v7());
    let before = refusal_transaction_snapshot(&flow).await;
    let fault_key = positive_advisory_key();
    install_refusal_receipt_fault(&flow.api.db.pool, fault_key).await;
    let mut gate = flow
        .api
        .db
        .pool
        .begin()
        .await
        .expect("owned refusal receipt gate");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("refusal gate backend pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(fault_key)
        .execute(&mut *gate)
        .await
        .expect("hold refusal receipt gate");
    let (_fault_log_guard, fault_logs) = crate::support::logs::capture_at(tracing::Level::ERROR);

    let reporting = flow.try_complete(&task, output.clone(), &key);
    tokio::pin!(reporting);
    let observe = async {
        let writer_pid = waiting_refusal_receipt(&flow, gate_pid).await;
        assert_ne!(writer_pid, gate_pid);
        assert_eq!(
            refusal_transaction_snapshot(&flow).await,
            before,
            "a second connection sees no root stop, scoped success, reported lease, journal/head, audit, receipt, output/accounting, payload, saga or outbox write"
        );
        let still_blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
               WHERE pid=$1 AND state='active' AND wait_event_type='Lock' \
                 AND $2=ANY(pg_blocking_pids(pid)) \
                 AND position('INSERT INTO command_dedupe' in query)>0)",
        )
        .bind(writer_pid)
        .bind(gate_pid)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("refusal writer remains blocked after the complete observation");
        gate.rollback()
            .await
            .expect("release only the owned refusal receipt gate");
        still_blocked
    };
    tokio::pin!(observe);
    let (failed, blocked_through_observation) = tokio::join!(&mut reporting, &mut observe);
    assert!(
        blocked_through_observation,
        "the actual receipt writer must remain blocked until observation releases the gate"
    );
    let failure = failed.expect_err("the late receipt fault returns no fabricated StepAck");
    assert_eq!(failure.code(), tonic::Code::Internal);
    assert_eq!(crate::support::worker::machine_code(&failure), "INTERNAL");
    let fault_log = fault_logs.text();
    assert!(
        fault_log.contains("test-only scoped output refusal receipt abort"),
        "the public failure must come from the labelled receipt trigger: {fault_log}"
    );
    assert!(
        !fault_log.contains("statement timeout"),
        "the report timed out before the labelled fault: {fault_log}"
    );
    assert_eq!(
        refusal_transaction_snapshot(&flow).await,
        before,
        "the labelled late failure rolls back both persists, audit and receipt"
    );

    remove_refusal_receipt_fault(&flow.api.db.pool).await;
    let accepted = flow
        .try_complete(&task, output.clone(), &key)
        .await
        .expect("the identical public report applies after the fault is removed");
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert_successful_refusal(&flow, &task, &key, outboxes_before).await;
    flow.verify().await;

    let committed = refusal_transaction_snapshot(&flow).await;
    let identity = refusal_identity(&flow, &key).await;
    assert_eq!(identity.0, 1, "one refusal audit");
    assert_eq!(identity.1, 1, "one original receipt");
    assert_eq!(identity.2, outboxes_before, "the report creates no outbox");
    assert_eq!(identity.3, identity.4, "the journal reaches the Run head");
    let replay = flow
        .try_complete(&task, output, &key)
        .await
        .expect("the identical public report returns its original receipt");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_eq!(refusal_identity(&flow, &key).await, identity);
    assert_eq!(
        refusal_transaction_snapshot(&flow).await,
        committed,
        "receipt replay adds no stop, success, audit, receipt, payload, accounting, journal, head, saga or outbox effect"
    );

    drain_and_compensate_every_reservation(&flow).await;
}

#[tokio::test]
async fn an_applied_lookup_preserves_both_real_successes_without_the_substitute_output() {
    let name = format!("output-refusal-applied-{}", uuid::Uuid::now_v7());
    let (flow, forward, output) = collection_at_seventeenth_member(&name, true).await;
    flow.fail(&forward, "OUTCOME_UNKNOWN", false).await;
    flow.drive().await;
    let tasks = flow.poll().await;
    assert_eq!(
        summary(&tasks),
        vec![row(16, "reserve~r", "RECONCILIATION", "line.lookup")]
    );
    let lookup = &tasks[0];
    let applied = json!({"effect": "applied", "output": output});
    assert!(
        applied.to_string().len() <= MAX_PAYLOAD_BYTES,
        "the genuine Applied wrapper must pass the per-report ceiling"
    );
    let outboxes_before = run_outboxes(&flow).await;
    let key = format!("refused-applied-{}", uuid::Uuid::now_v7());

    let accepted = flow
        .try_complete(lookup, applied.clone(), &key)
        .await
        .expect("preserve the real Applied verdict while refusing its substitute output");
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert_successful_refusal(&flow, lookup, &key, outboxes_before).await;

    let effects: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT node,status,output_digest IS NULL FROM steps \
          WHERE tenant_id=$1 AND run_id=$2 \
            AND instance_path_nodes=ARRAY['lines']::text[] \
            AND instance_path_indices=ARRAY[16]::integer[] \
            AND node IN ('reserve','reserve~r') ORDER BY node",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("forward and lookup effects");
    assert_eq!(
        effects,
        vec![
            ("reserve".into(), "SUCCEEDED".into(), true),
            ("reserve~r".into(), "SUCCEEDED".into(), true),
        ]
    );

    flow.verify().await;
    let first_identity = refusal_identity(&flow, &key).await;
    let replay = flow
        .try_complete(lookup, applied, &key)
        .await
        .expect("replay the original Applied receipt");
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_eq!(refusal_identity(&flow, &key).await, first_identity);

    drain_and_compensate_every_reservation(&flow).await;
}

fn script_collection(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {
                "id": "work", "type": "script", "artifactDigest": SCRIPT_DIGEST,
                "language": "typescript", "next": "returned"
            },
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {
                "id": "items", "type": "forEach", "items": "input.items", "body": "line",
                "result": "bodyResult", "maxItems": 2, "maxConcurrency": 2, "next": "done"
            },
            {"id": "done", "type": "end"}
        ]
    })
}

pub(crate) fn collection(
    name: &str,
    items: &str,
    max_items: u32,
    max_concurrency: u16,
    result: &str,
) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            {"id": "work", "type": "activity", "activity": "limits.step", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": items, "body": "line",
             "result": result, "maxItems": max_items, "maxConcurrency": max_concurrency,
             "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

fn looping(name: &str, initial: &str, carry: &str) -> Value {
    looping_until(name, "while", initial, carry, "size(carry) >= 0", 3)
}

fn looping_until(
    name: &str,
    kind: &str,
    initial: &str,
    carry: &str,
    condition: &str,
    max_iterations: u32,
) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "again",
        "bodies": {"iteration": {"entrypoint": "step", "nodes": [
            {"id": "step", "type": "activity", "activity": "limits.step", "next": "returned"},
            {"id": "returned", "type": "return", "result": "steps.step.output"}
        ]}},
        "nodes": [
            {"id": "again", "type": kind, "condition": condition, "body": "iteration",
             "initial": initial, "carry": carry, "maxIterations": max_iterations,
             "timeoutMs": 600_000, "next": "done"},
            {"id": "done", "type": "end"}
        ]
    })
}

/// Polls what is offered and completes each task with `output` (raw JSON text).
pub(crate) async fn answer(
    world: &World,
    workers: &Workers,
    session: &(uuid::Uuid, String),
    output: &str,
) -> usize {
    let tasks = workers
        .poll(
            &world.worker,
            session.0,
            &session.1,
            8,
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("poll the scope");
    for task in &tasks {
        workers
            .complete(
                &world.worker,
                &task_ref(task),
                output.as_bytes(),
                &format!("answer-{}", uuid::Uuid::now_v7()),
            )
            .await
            .expect("complete the member");
    }
    tasks.len()
}

/// Whether the run is stopping or finished: it is the end of what a test drives.
async fn is_over(world: &World, run: RunId) -> bool {
    matches!(
        world.run_status(run).await.as_str(),
        "CANCELLING" | "FAILED" | "SUCCEEDED" | "CANCELLED" | "TIMED_OUT"
    )
}

async fn start_collection(world: &World, document: Value, input: &Value) -> RunId {
    let (workflow, _) = world.publish(document).await;
    world.start(workflow, input).await
}

#[tokio::test]
async fn an_end_output_created_with_the_report_is_charged_once_as_a_logical_output() {
    let world = world().await;
    let work = with(activity("work"), "next", json!("done"));
    let done = with(end_node("done"), "outputExpr", json!("steps.work.output"));
    let (workflow, _) = world
        .publish(document("root-output-ledger", "work", vec![work, done]))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let session = world.session(&["act.work"]).await;
    let task = world.one_task(&session).await;
    let output = json!({"answer": "retained twice"});
    world
        .complete(&session, &task, &output)
        .await
        .expect("the report and end result commit together");

    let charged: i64 =
        sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("logical output ledger");
    let one_output = serde_json::to_vec(&output).expect("output JSON").len() as i64;
    assert_eq!(
        charged,
        2 * one_output,
        "the worker output and the end result are distinct logical references"
    );
}

#[tokio::test]
async fn a_collection_snapshot_over_the_value_ceiling_fails_the_run_before_it_starts() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Two 80 KiB items fit the 256 KiB input; the doubled list is 320 KiB.
    let big = "y".repeat(80 * 1024);
    let run = start_collection(
        &world,
        collection(
            "bytes-snapshot",
            "input.items + input.items",
            4,
            2,
            "bodyResult",
        ),
        &json!({"items": [big.clone(), big]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-snapshot").await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    assert_eq!(
        scopes(&world, run).await,
        0,
        "no oversize value was retained"
    );
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_member_output_over_the_value_ceiling_is_refused_and_the_lease_survives() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let run = start_collection(
        &world,
        collection("bytes-member", "input.items", 2, 1, "bodyResult"),
        &json!({"items": [1]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-member").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    let tasks = workers
        .poll(&world.worker, session.0, &session.1, 1, None)
        .await
        .expect("poll");
    assert_eq!(tasks.len(), 1);
    let oversize = text_of(MAX_PAYLOAD_BYTES + 1);
    assert!(
        workers
            .complete(
                &world.worker,
                &task_ref(&tasks[0]),
                oversize.as_bytes(),
                "oversize"
            )
            .await
            .is_err(),
        "a scoped member output is held to the same 256 KiB as a root one"
    );
    let fits = text_of(MAX_PAYLOAD_BYTES);
    workers
        .complete(
            &world.worker,
            &task_ref(&tasks[0]),
            fits.as_bytes(),
            "at-the-ceiling",
        )
        .await
        .expect("exactly 256 KiB is allowed");
    let charged: i64 =
        sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("logical output ledger");
    assert_eq!(charged, MAX_PAYLOAD_BYTES as i64);
    workers
        .complete(
            &world.worker,
            &task_ref(&tasks[0]),
            fits.as_bytes(),
            "at-the-ceiling",
        )
        .await
        .expect("completion replay");
    let replayed: i64 =
        sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("logical output ledger after replay");
    assert_eq!(replayed, charged, "a replay charges zero bytes");
    let node_kind: String = sqlx::query_scalar(
        "SELECT node_kind FROM steps WHERE run_id=$1 AND node='work' \
           AND cardinality(instance_path_nodes)>0",
    )
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("scoped node kind");
    assert_eq!(node_kind, "activity");
    let path = InstancePath::root()
        .child(NodeId::parse("items").expect("owner"), 0)
        .expect("member path");
    let bounded = world
        .store
        .load_path(world.admin.tenant_id(), run, path.clone())
        .await
        .expect("bounded path read")
        .expect("run");
    assert_eq!(
        bounded.state.local_instances().count(),
        2,
        "the completed activity and its local return are the complete path body"
    );
    assert_eq!(bounded.state.ancestors().len(), 1);
    assert_eq!(
        bounded.data.outputs_in(bounded.state.path()).count(),
        1,
        "only the exact path's completed activity output is loaded"
    );
    assert!(matches!(
        bounded.data.scope_input(bounded.state.path()),
        Some(ScopeInput::Collection { index: 0, .. })
    ));
    assert_eq!(world.run_status(run).await.as_str(), "RUNNING");
    let stale = bounded.clone();
    let contended = bounded.clone();
    let mut fenced = world
        .store
        .begin_path(bounded, Lock::Wait)
        .await
        .expect("bounded mutation fence")
        .expect("run");
    assert_eq!(fenced.snapshot().state.path(), &path);
    assert!(
        !fenced
            .worker_drained(world.worker.subject().as_str())
            .await
            .expect("drain fence")
    );
    assert!(
        fenced
            .worker_session_room(
                session.0,
                &Token::hash_of(&session.1),
                world.worker.subject().as_str(),
            )
            .await
            .expect("session room")
            > 0
    );
    assert!(
        !fenced
            .bound_patterns(world.worker.subject().as_str())
            .await
            .expect("bindings")
            .is_empty()
    );
    assert!(
        fenced
            .current_worker_authorized(world.worker.subject().as_str())
            .await
            .expect("current worker authority")
    );
    assert_eq!(
        fenced
            .required_worker_protocol_minor()
            .await
            .expect("protocol fence"),
        0
    );
    assert!(
        fenced
            .activity_admitted(&ActivityName::parse("limits.step").expect("activity"))
            .await
            .expect("activity quota")
    );
    let completed =
        ExecutionInstanceId::in_path(path.clone(), NodeId::parse("work").expect("activity node"));
    assert!(
        fenced
            .script_execution_admitted(
                &completed,
                &ActivityName::parse("limits.step").expect("activity"),
            )
            .await
            .expect("script retention fence")
    );
    let reported_token = tasks[0].lease_token.hash();
    assert!(
        !fenced
            .lease_matches(&completed, &reported_token)
            .await
            .expect("lease match")
    );
    assert!(
        !fenced
            .lease_unexpired(&completed, &reported_token)
            .await
            .expect("lease expiry")
    );
    assert!(
        fenced
            .reported_by(&completed, &reported_token)
            .await
            .expect("report token")
    );
    assert_eq!(
        fenced.lease_expiry(&completed).await.expect("lease expiry"),
        None
    );
    assert!(
        world
            .store
            .begin_path(contended, Lock::SkipLocked)
            .await
            .expect("skip locked fence")
            .is_none(),
        "a competing bounded writer does not wait behind the run lock"
    );
    fenced.commit().await.expect("read-only fenced commit");
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert!(matches!(
        world.store.begin_path(stale, Lock::Wait).await,
        Err(sagat_application::error::StoreError::RevisionConflict { .. })
    ));
}

#[tokio::test]
async fn a_bounded_path_read_does_not_wait_for_the_run_write_lock() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let run = start_collection(
        &world,
        collection("bounded-read-lock", "input.items", 1, 1, "bodyResult"),
        &json!({"items": [1]}),
    )
    .await;
    let scheduler = scheduler(&world, "bounded-read-lock").await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    let path = InstancePath::root()
        .child(NodeId::parse("items").expect("owner"), 0)
        .expect("member path");

    let mut writer = world.db.pool.begin().await.expect("writer transaction");
    sqlx::query("SELECT id FROM runs WHERE id = $1 FOR UPDATE")
        .bind(run.as_uuid())
        .execute(&mut *writer)
        .await
        .expect("hold the run write lock");
    let bounded = tokio::time::timeout(
        Duration::from_secs(1),
        world.store.load_path(world.admin.tenant_id(), run, path),
    )
    .await
    .expect("a bounded snapshot read must not queue behind the writer")
    .expect("bounded path read")
    .expect("run");
    assert_eq!(bounded.state.global().run_id(), run);
    writer.rollback().await.expect("release writer lock");
}

#[tokio::test]
async fn a_bounded_path_loads_only_its_indexed_scope_saga_authority() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let run = start_collection(
        &world,
        collection("bounded-saga-authority", "input.items", 2, 2, "bodyResult"),
        &json!({"items": [1, 2]}),
    )
    .await;
    let scheduler = scheduler(&world, "bounded-saga-authority").await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    let admitted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_iterations WHERE run_id=$1 AND lifecycle='ADMITTED'",
    )
    .bind(run.as_uuid())
    .fetch_one(&world.db.pool)
    .await
    .expect("two admitted fixtures");
    assert_eq!(admitted, 2, "both sibling paths must actually exist");

    sqlx::query(
        "INSERT INTO scope_sagas \
         (tenant_id,run_id,owner_path_nodes,owner_path_indices,owner_node,phase,cause_code, \
          outcome,during_stop,planned_sequence,created_at,updated_at) \
         VALUES ($1,$2,'{}','{}','items','COMPENSATING','BRANCH_FAILED',NULL,false,1,now(),now())",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(run.as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("scope saga header");
    for (iteration, node, kind, compensation) in [
        (0_i32, "work", "COMPENSATE", Some(uuid::Uuid::now_v7())),
        (1_i32, "returned", "IRREVERSIBLE", None),
    ] {
        sqlx::query(
            "INSERT INTO scope_saga_entries \
             (tenant_id,run_id,owner_path_nodes,owner_path_indices,owner_node,iteration_index, \
              node,kind,compensation_id,after_iterations,after_nodes) \
             VALUES ($1,$2,'{}','{}','items',$3,$4,$5,$6,'{}','{}')",
        )
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .bind(iteration)
        .bind(node)
        .bind(kind)
        .bind(compensation)
        .execute(&world.db.pool)
        .await
        .expect("scope saga entry");
    }

    let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner"));
    let first = InstancePath::root()
        .child(owner.node().clone(), 0)
        .expect("first member");
    let first = world
        .store
        .load_path(world.admin.tenant_id(), run, first)
        .await
        .expect("first bounded path")
        .expect("run");
    let first_saga = first.state.scope_saga(&owner).expect("ancestor saga");
    assert_eq!(first_saga.entries().len(), 1);
    assert!(
        first_saga
            .entries()
            .contains_key(&NodeId::parse("work").expect("forward"))
    );
    assert!(first_saga.irreversible().is_empty());
    let first_compensation = ExecutionInstanceId::in_path(
        first.state.path().clone(),
        NodeId::parse("work").expect("forward").compensation(),
    );
    assert!(
        first
            .state
            .authorizes_scope_compensation(&first_compensation)
    );

    let second = InstancePath::root()
        .child(owner.node().clone(), 1)
        .expect("second member");
    let second = world
        .store
        .load_path(world.admin.tenant_id(), run, second)
        .await
        .expect("second bounded path")
        .expect("run");
    let second_saga = second.state.scope_saga(&owner).expect("ancestor saga");
    assert!(second_saga.entries().is_empty());
    assert_eq!(
        second_saga.irreversible(),
        [NodeId::parse("returned").expect("irreversible")]
    );
    let sibling_compensation = ExecutionInstanceId::in_path(
        second.state.path().clone(),
        NodeId::parse("work").expect("forward").compensation(),
    );
    assert!(
        !second
            .state
            .authorizes_scope_compensation(&sibling_compensation),
        "authority from iteration zero must not authorize its sibling"
    );
}

#[tokio::test]
async fn a_bounded_completion_commits_without_reopening_the_run_history() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let run = start_collection(
        &world,
        collection("bounded-completion", "input.items", 1, 1, "bodyResult"),
        &json!({"items": [1]}),
    )
    .await;
    let scheduler = scheduler(&world, "bounded-completion").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    let task = workers
        .poll(&world.worker, session.0, &session.1, 1, None)
        .await
        .expect("poll")
        .pop()
        .expect("task");

    let bounded = world
        .store
        .load_path(world.admin.tenant_id(), run, task.instance.path().clone())
        .await
        .expect("bounded path")
        .expect("run");
    let mut tx = world
        .store
        .begin_path(bounded, Lock::Wait)
        .await
        .expect("bounded transaction")
        .expect("run");
    let heartbeat = heartbeat_path_instance(&mut tx.snapshot().state, &task.instance, task.fence)
        .expect("bounded heartbeat");
    let definition = tx.snapshot().definition.clone();
    let progress = complete_path_instance(
        &definition,
        &mut tx.snapshot().state,
        &task.instance,
        task.fence,
    )
    .expect("bounded completion");
    let mut events = heartbeat.event.into_iter().collect::<Vec<_>>();
    events.extend(progress.events);
    let mut effects = Effects::by(Actor::Worker(session.0.to_string()), None);
    let idempotency = Idempotency {
        scope: format!("bounded-test:{run}"),
        key: "complete".into(),
        fingerprint: vec![1, 2, 3],
    };
    effects.idempotency = Some((idempotency.clone(), "ack".into()));
    effects.reported = Some(ReportedLease {
        instance: task.instance.clone(),
        token_hash: task.lease_token.hash(),
    });
    tx.persist_addressed(&events, &[(task.instance.clone(), b"{}".to_vec())], effects)
        .await
        .expect("bounded persist");
    tx.commit().await.expect("bounded commit");

    let persisted = world
        .store
        .load_path(world.admin.tenant_id(), run, task.instance.path().clone())
        .await
        .expect("persisted path")
        .expect("run");
    assert_eq!(
        persisted
            .state
            .local_instance(task.instance.node())
            .map(|step| step.status()),
        Some(StepStatus::Succeeded)
    );
    assert_eq!(persisted.data.output(&task.instance), Some(&b"{}"[..]));
    let payload_events: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM run_events WHERE run_id=$1 AND payload_digest IS NOT NULL \
         ORDER BY seq",
    )
    .bind(run.as_uuid())
    .fetch_all(&world.db.pool)
    .await
    .expect("payload journal events");
    assert_eq!(payload_events, ["step.completed"]);
    let charged: i64 =
        sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("bounded output ledger");
    assert_eq!(charged, 2);
    let mut replay = world
        .store
        .begin_path(persisted, Lock::Wait)
        .await
        .expect("replay transaction")
        .expect("run");
    assert_eq!(
        replay.replay(&idempotency).await.expect("bounded replay"),
        Some("ack".into())
    );
    replay.commit().await.expect("read-only replay commit");
}

#[tokio::test]
async fn a_bounded_script_lease_pins_retention_and_zero_withholds_a_new_member() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let script_activity = ready_script(&world, 30).await;
    set_script_retention(&world, 5).await;
    let run = start_collection(
        &world,
        script_collection("bounded-script-retention"),
        &json!({"items": [1]}),
    )
    .await;
    let active_scheduler = scheduler(&world, "bounded-script-retention").await;
    assert_eq!(active_scheduler.tick().await.collections_advanced, 1);
    assert_eq!(active_scheduler.tick().await.collections_advanced, 1);

    let owner = NodeId::parse("items").expect("collection owner");
    let node = NodeId::parse("work").expect("script node");
    let first_path = InstancePath::root()
        .child(owner.clone(), 0)
        .expect("first member path");
    let first = ExecutionInstanceId::in_path(first_path.clone(), node.clone());
    let bounded = world
        .store
        .load_path(world.admin.tenant_id(), run, first_path)
        .await
        .expect("first bounded path")
        .expect("run");
    assert_eq!(
        bounded
            .state
            .local_instance(&node)
            .map(|step| step.status()),
        Some(StepStatus::Ready)
    );
    let mut tx = world
        .store
        .begin_path(bounded, Lock::Wait)
        .await
        .expect("first bounded transaction")
        .expect("run");
    assert!(
        tx.script_execution_admitted(&first, &script_activity)
            .await
            .expect("script retention fence")
    );
    let (_, leased) = lease_path_instance(&mut tx.snapshot().state, &first)
        .expect("lease admitted bounded script");
    let token = Token::from_bytes([7_u8; 32]).expect("lease token");
    let mut effects = Effects::by(Actor::Worker("bounded-script-test".into()), None);
    effects.lease = Some(LeaseGrant {
        instance: first.clone(),
        token_hash: token.hash(),
        worker_id: "bounded-script-test".into(),
        ttl: Duration::from_secs(30),
        attempt_timeout: None,
    });
    tx.persist_addressed(&[leased], &[], effects)
        .await
        .expect("persist bounded script lease");
    tx.commit().await.expect("commit bounded script lease");

    let detail: serde_json::Value = sqlx::query_scalar(
        "SELECT detail FROM run_events WHERE run_id=$1 AND node='work' \
         AND instance_path_nodes=$2 AND instance_path_indices=$3 \
         AND event_type='step.leased' AND attempt=1",
    )
    .bind(run.as_uuid())
    .bind(vec!["items".to_owned()])
    .bind(vec![0_i32])
    .fetch_one(&world.db.pool)
    .await
    .expect("bounded script lease detail");
    assert_eq!(detail["scriptRetentionPolicyVersion"], 1);
    assert_eq!(detail["scriptRetentionDays"], 5);

    set_script_retention(&world, 0).await;
    let blocked_run = start_collection(
        &world,
        script_collection("bounded-script-retention-blocked"),
        &json!({"items": [1]}),
    )
    .await;
    assert_eq!(active_scheduler.tick().await.collections_advanced, 1);
    assert_eq!(active_scheduler.tick().await.collections_advanced, 1);
    let second_path = InstancePath::root()
        .child(owner, 0)
        .expect("new member path");
    let second = ExecutionInstanceId::in_path(second_path.clone(), node);
    let bounded = world
        .store
        .load_path(world.admin.tenant_id(), blocked_run, second_path)
        .await
        .expect("second bounded path")
        .expect("run");
    let mut tx = world
        .store
        .begin_path(bounded, Lock::Wait)
        .await
        .expect("second bounded transaction")
        .expect("run");
    assert!(
        !tx.script_execution_admitted(&second, &script_activity)
            .await
            .expect("closed script retention fence"),
        "a zero policy leaves new scoped script work ready"
    );
    tx.commit().await.expect("commit withheld script probe");
    let leases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND node='work' \
         AND instance_path_nodes=$2 AND instance_path_indices=$3 AND event_type='step.leased'",
    )
    .bind(blocked_run.as_uuid())
    .bind(vec!["items".to_owned()])
    .bind(vec![0_i32])
    .fetch_one(&world.db.pool)
    .await
    .expect("withheld member leases");
    assert_eq!(leases, 0);
}

#[tokio::test]
async fn member_outputs_that_pass_four_mib_in_all_fail_the_run_with_run_data_limit() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // 250,000-byte outputs: sixteen retain 4,000,000 bytes, the seventeenth passes 4 MiB.
    let items: Vec<u32> = (0..17).collect();
    let run = start_collection(
        &world,
        collection("bytes-run", "input.items", 17, 1, "1"),
        &json!({"items": items}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-run").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(250_000);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..40 {
        scheduler.tick().await;
        answer(&world, &workers, &session, &output).await;
        scheduler.tick().await;
        if is_over(&world, run).await {
            break;
        }
    }
    const { assert!(16 * 250_000 < MAX_RUN_OUTPUT_BYTES && 17 * 250_000 > MAX_RUN_OUTPUT_BYTES) };
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    let charged: i64 =
        sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(&world.db.pool)
            .await
            .expect("logical output ledger");
    assert_eq!(
        charged,
        16 * 250_000,
        "equal payload contents are charged once for every logical output reference"
    );
}

#[tokio::test]
async fn an_aggregate_over_the_value_ceiling_fails_the_scope_instead_of_storing_it() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Three 100 KiB outputs are each fine and together 300 KiB, over the 256 KiB value ceiling.
    let run = start_collection(
        &world,
        collection("bytes-aggregate", "input.items", 3, 1, "bodyResult"),
        &json!({"items": [1, 2, 3]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-aggregate").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(100 * 1024);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..12 {
        scheduler.tick().await;
        answer(&world, &workers, &session, &output).await;
        scheduler.tick().await;
        if is_over(&world, run).await {
            break;
        }
    }
    let last = scheduler.tick().await;
    assert_eq!(last.failed, 0, "{last:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payloads WHERE tenant_id=$1 AND octet_length(content) > $2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(i64::try_from(MAX_PAYLOAD_BYTES).expect("ceiling"))
    .fetch_one(&world.db.pool)
    .await
    .expect("oversize payloads");
    assert_eq!(stored, 0, "nothing over 256 KiB was retained");
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_loop_whose_initial_carry_is_over_the_value_ceiling_fails_before_it_starts() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let big = "z".repeat(200 * 1024);
    let (workflow, _) = world
        .publish(looping(
            "bytes-loop-initial",
            "[input.big, input.big]",
            "bodyResult",
        ))
        .await;
    let run = world.start(workflow, &json!({"big": big})).await;
    let report = scheduler(&world, "bytes-loop-initial").await.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    assert_eq!(scopes_of_loops(&world, run).await, 0);
}

async fn scopes_of_loops(world: &World, run: RunId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM loop_scopes WHERE tenant_id=$1 AND run_id=$2")
        .bind(world.admin.tenant_id().as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&world.db.pool)
        .await
        .expect("loop scope count")
}

#[tokio::test]
async fn a_loop_carry_over_the_value_ceiling_fails_the_loop_and_keeps_the_ceiling() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    let (workflow, _) = world
        .publish(looping(
            "bytes-loop-carry",
            "[0]",
            "[bodyResult, bodyResult]",
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "bytes-loop-carry").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    // 200 KiB is a legal output; the carry that repeats it twice is 400 KiB.
    assert_eq!(
        answer(&world, &workers, &session, &text_of(200 * 1024)).await,
        1
    );
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM payloads WHERE tenant_id=$1 AND octet_length(content) > $2",
    )
    .bind(world.admin.tenant_id().as_uuid())
    .bind(i64::try_from(MAX_PAYLOAD_BYTES).expect("ceiling"))
    .fetch_one(&world.db.pool)
    .await
    .expect("oversize payloads");
    assert_eq!(stored, 0, "the oversize carry was never retained");
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_scoped_task_context_built_from_the_largest_values_stays_under_eight_mib() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // The run input and the loop's carry are each about 250 KiB; the iteration task is built from
    // both. Only the iteration's own outputs join the context, so it stays far below the limit.
    let big = "q".repeat(250 * 1024);
    let (workflow, _) = world
        .publish(looping("bytes-context", "input.big", "bodyResult"))
        .await;
    world.start(workflow, &json!({"big": big})).await;
    let scheduler = scheduler(&world, "bytes-context").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    let tasks = workers
        .poll(&world.worker, session.0, &session.1, 1, None)
        .await
        .expect("the iteration is offered with its context");
    assert_eq!(tasks.len(), 1);
    assert_eq!(MAX_TASK_INPUT_BYTES, 8 * 1024 * 1024);
    assert!(
        tasks[0].input.len() > 2 * 250 * 1024,
        "the context carries the input and the carry: {} bytes",
        tasks[0].input.len()
    );
    assert!(
        tasks[0].input.len() < MAX_TASK_INPUT_BYTES,
        "{} bytes",
        tasks[0].input.len()
    );
}

#[tokio::test]
async fn the_carry_that_ends_a_loop_must_fit_beside_the_outputs_the_run_retains() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Sixteen 255,000-byte outputs retain 4,080,000 bytes, which fits 4 MiB. The final carry is
    // the last output again: another 255,000 bytes would make the loop's own output unretainable.
    let (workflow, _) = world
        .publish(looping_until(
            "bytes-loop-final",
            "repeatUntil",
            "\"start\"",
            "bodyResult",
            "iteration >= 15",
            20,
        ))
        .await;
    let run = world.start(workflow, &json!({})).await;
    let scheduler = scheduler(&world, "bytes-loop-final").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(255_000);
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    for round in 0..40 {
        let answered = answer(&world, &workers, &session, &output).await;
        assert_eq!(
            answered,
            1,
            "round {round}: {:?} {:?}",
            world.run_status(run).await,
            world.journal(run).await.last()
        );
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if is_over(&world, run).await {
            break;
        }
    }
    const { assert!(16 * 255_000 < MAX_RUN_OUTPUT_BYTES && 17 * 255_000 > MAX_RUN_OUTPUT_BYTES) };
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn an_aggregate_that_fits_alone_but_not_beside_the_retained_outputs_fails_the_scope() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Sixteen 262,000-byte outputs retain 4,192,000 bytes, 2,304 short of 4 MiB. The aggregate is
    // sixteen small lists (about 3 KB, far under the 256 KiB value ceiling) and does not fit in
    // what is left.
    let members = (1..=64)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let items: Vec<u32> = (0..16).collect();
    let run = start_collection(
        &world,
        collection(
            "bytes-aggregate-retained",
            "input.items",
            16,
            1,
            &format!("[{members}]"),
        ),
        &json!({"items": items}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-aggregate-retained").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    let output = text_of(262_000);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..60 {
        scheduler.tick().await;
        answer(&world, &workers, &session, &output).await;
        scheduler.tick().await;
        if is_over(&world, run).await {
            break;
        }
    }
    const { assert!(16 * 262_000 < MAX_RUN_OUTPUT_BYTES && MAX_RUN_OUTPUT_BYTES - 16 * 262_000 < 2_500) };
    assert_failed_with(&world, run, "RUN_DATA_LIMIT").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn an_aggregate_that_cannot_be_computed_fails_the_scope_with_expression_error() {
    let world = world().await;
    allow_iterations(&world, 4).await;
    // Dividing by zero fails on every member, so the aggregate can never be built.
    let run = start_collection(
        &world,
        collection(
            "bytes-aggregate-error",
            "input.items",
            2,
            1,
            "bodyResult / 0",
        ),
        &json!({"items": [1, 2]}),
    )
    .await;
    let scheduler = scheduler(&world, "bytes-aggregate-error").await;
    let workers = scoped_workers(&world);
    let session = register_scoped(&world, &workers).await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    for _ in 0..8 {
        scheduler.tick().await;
        answer(&world, &workers, &session, "1").await;
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        if is_over(&world, run).await {
            break;
        }
    }
    assert_failed_with(&world, run, "EXPRESSION_ERROR").await;
    world.assert_consistent(run).await;
}

#[tokio::test]
async fn a_bounded_reconciliation_persists_its_companion_and_manual_review_status() {
    use sagat_application::worker::Capabilities;
    use sagat_domain::execution::{FailureCode, RunStatus};
    use sagat_domain::saga::ReconcileVerdict;

    for verdict in [ReconcileVerdict::Unknown, ReconcileVerdict::Applied] {
        let world = world().await;
        allow_iterations(&world, 4).await;
        let mut doc = collection("bounded-reconciliation", "input.items", 1, 1, "bodyResult");
        doc["bodies"]["line"]["nodes"][0]["reconcileWith"] = json!("limits.lookup");
        doc["bodies"]["line"]["nodes"][0]["highRisk"] = json!(true);
        let run = start_collection(&world, doc, &json!({"items": [1]})).await;
        let scheduler = scheduler(&world, "bounded-reconciliation").await;
        let workers = scoped_workers(&world);
        world.bind_everything(&world.worker).await;
        let session = workers
            .register(
                &world.worker,
                "payments",
                "bounded-reconciliation-worker",
                &Capabilities {
                    activities: vec!["limits.step".into(), "limits.lookup".into()],
                    languages: vec!["rust".into()],
                    pools: Vec::new(),
                    max_inflight: 1,
                    protocol_version: "1.5".into(),
                },
            )
            .await
            .expect("register scoped forward/lookup worker");
        assert_eq!(scheduler.tick().await.collections_advanced, 1);
        assert_eq!(scheduler.tick().await.collections_advanced, 1);
        let task = workers
            .poll(
                &world.worker,
                session.worker_id,
                session.session_token.expose(),
                1,
                None,
            )
            .await
            .expect("claim forward")
            .pop()
            .expect("forward task");
        let path = task.instance.path().clone();
        let lookup =
            ExecutionInstanceId::in_path(path.clone(), task.instance.node().reconciliation());
        let snapshot = world
            .store
            .load_path(world.admin.tenant_id(), run, path.clone())
            .await
            .expect("bounded snapshot")
            .expect("run");
        let mut tx = world
            .store
            .begin_path(snapshot, Lock::Wait)
            .await
            .expect("bounded tx")
            .expect("run");
        assert!(
            tx.current_worker_authorized(world.worker.subject().as_str())
                .await
                .expect("current authority")
        );
        assert!(
            tx.lease_unexpired(&task.instance, &task.lease_token.hash())
                .await
                .expect("forward fence")
        );
        let definition = tx.snapshot().definition.clone();
        let now = tx.now();
        let events = sagat_domain::execution::fail_path_instance(
            &definition,
            &mut tx.snapshot().state,
            &task.instance,
            sagat_domain::execution::Failure {
                fence: task.fence,
                code: FailureCode::outcome_unknown(),
                retryable: true,
                jitter_sample: 0.0,
            },
            now,
        )
        .expect("bounded unknown-outcome command")
        .events;
        let mut effects = Effects::by(Actor::Worker(session.worker_id.to_string()), None);
        effects.reported = Some(ReportedLease {
            instance: task.instance.clone(),
            token_hash: task.lease_token.hash(),
        });
        tx.persist_addressed(&events, &[], effects)
            .await
            .expect("persist request and lookup hint");
        tx.commit().await.expect("request commit");
        world.assert_consistent(run).await;

        let reconciliation = workers
            .poll(
                &world.worker,
                session.worker_id,
                session.session_token.expose(),
                1,
                None,
            )
            .await
            .expect("claim lookup")
            .pop()
            .expect("lookup task");
        assert_eq!(reconciliation.instance, lookup);
        let snapshot = world
            .store
            .load_path(world.admin.tenant_id(), run, path.clone())
            .await
            .expect("lookup snapshot")
            .expect("run");
        let mut tx = world
            .store
            .begin_path(snapshot, Lock::Wait)
            .await
            .expect("bounded tx")
            .expect("run");
        assert!(
            tx.lease_unexpired(&lookup, &reconciliation.lease_token.hash())
                .await
                .expect("lookup fence")
        );
        let definition = tx.snapshot().definition.clone();
        let now = tx.now();
        let events = sagat_domain::execution::complete_reconciliation_path_instance(
            &definition,
            &mut tx.snapshot().state,
            &lookup,
            reconciliation.fence,
            verdict,
            0.0,
            now,
        )
        .expect("bounded reconciliation command")
        .events;
        let expected_status = if verdict == ReconcileVerdict::Unknown {
            RunStatus::ManualReview
        } else {
            RunStatus::Running
        };
        assert_eq!(tx.snapshot().state.global().status(), expected_status);
        let mut effects = Effects::by(Actor::Worker(session.worker_id.to_string()), None);
        effects.reported = Some(ReportedLease {
            instance: lookup.clone(),
            token_hash: reconciliation.lease_token.hash(),
        });
        let mut outputs = vec![(lookup.clone(), b"{}".to_vec())];
        if verdict == ReconcileVerdict::Applied {
            outputs.push((task.instance.clone(), br#"{"ok":true}"#.to_vec()));
        }
        tx.persist_addressed(&events, &outputs, effects)
            .await
            .expect("persist verdict");
        tx.commit().await.expect("verdict commit");
        assert_eq!(world.run_status(run).await, expected_status);
        let loaded = world
            .store
            .load_path(world.admin.tenant_id(), run, path)
            .await
            .expect("reload")
            .expect("run");
        assert_eq!(loaded.state.global().status(), expected_status);
        assert_eq!(loaded.data.output(&lookup), Some(b"{}".as_slice()));
        if verdict == ReconcileVerdict::Applied {
            assert_eq!(
                loaded.data.output(&task.instance),
                Some(br#"{"ok":true}"#.as_slice())
            );
        } else {
            assert_eq!(loaded.data.output(&task.instance), None);
        }
        world.assert_consistent(run).await;
    }
}

#[tokio::test]
async fn a_bounded_scheduler_fence_lasts_to_commit_and_deadlines_are_exact_path() {
    use sagat_application::shards::Lease;
    use sagat_domain::sharding::{SHARDS, Shard};
    let world = world().await;
    allow_iterations(&world, 4).await;
    let mut definition = collection("bounded-scheduler-fence", "input.items", 2, 2, "bodyResult");
    definition["bodies"]["line"]["nodes"][0]["timeoutMs"] = json!(10_000);
    let run = start_collection(&world, definition, &json!({"items":[1,2]})).await;
    let scheduler = scheduler(&world, "bounded-scheduler-fence").await;
    for _ in 0..3 {
        assert_eq!(scheduler.tick().await.collections_advanced, 1);
    }
    let workers = scoped_workers(&world);
    world.bind_everything(&world.worker).await;
    let registered = workers
        .register(
            &world.worker,
            "payments",
            "bounded-fence-worker",
            &sagat_application::worker::Capabilities {
                activities: vec!["limits.step".into()],
                languages: vec!["rust".into()],
                pools: Vec::new(),
                max_inflight: 2,
                protocol_version: "1.5".into(),
            },
        )
        .await
        .expect("register two-slot worker");
    let session = (
        registered.worker_id,
        registered.session_token.expose().to_owned(),
    );
    let mut tasks = workers
        .poll(&world.worker, session.0, &session.1, 2, None)
        .await
        .expect("leases");
    // A fair claim may hand out only one member of the same run per poll.
    if tasks.len() < 2 {
        tasks.extend(
            workers
                .poll(&world.worker, session.0, &session.1, 1, None)
                .await
                .expect("second scoped lease"),
        );
    }
    assert_eq!(tasks.len(), 2);
    let task = &tasks[0];
    let shard = Shard::of(run.as_uuid());
    let (owner, generation): (String, i64) =
        sqlx::query_as("SELECT owner,generation FROM scheduler_shards WHERE shard=$1")
            .bind(i16::from(shard.index()))
            .fetch_one(&world.db.pool)
            .await
            .expect("shard lease");
    let lease = Lease {
        shard,
        generation: u64::try_from(generation).expect("generation"),
    };
    let stamp = || {
        sqlx::query_as::<_, (i64,i64,i64)>(
        "SELECT run_version, (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
         (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2) \
         FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(world.admin.tenant_id().as_uuid()).bind(run.as_uuid()).fetch_one(&world.db.pool)
    };
    let before = stamp().await.expect("before fence reads");
    let snapshot = world
        .store
        .load_path(world.admin.tenant_id(), run, task.instance.path().clone())
        .await
        .expect("snapshot")
        .expect("run");
    let mut tx = world
        .store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("bounded tx")
        .expect("run");
    let deadlines = tx
        .lease_deadlines(&task.instance)
        .await
        .expect("own deadlines")
        .expect("lease");
    assert_eq!(deadlines.expires_at, task.expires_at);
    assert_eq!(
        deadlines.attempt_deadline_at,
        Some(task.expires_at - workers.settings().lease_ttl + Duration::from_secs(10)),
        "attempt timeout and renewable lease expiry are distinct clocks"
    );
    assert_eq!(
        Some(deadlines.expires_at),
        tx.lease_expiry(&task.instance).await.expect("expiry")
    );
    assert!(
        tx.lease_deadlines(&tasks[1].instance)
            .await
            .expect("other path")
            .is_none()
    );
    assert!(
        tx.lease_expiry(&tasks[1].instance)
            .await
            .expect("other expiry")
            .is_none()
    );
    assert!(
        !tx.holds_shard("wrong-owner", lease)
            .await
            .expect("owner fence")
    );
    assert!(
        !tx.holds_shard(
            &owner,
            Lease {
                generation: u64::MAX,
                ..lease
            }
        )
        .await
        .expect("generation range")
    );
    assert!(
        !tx.holds_shard(
            &owner,
            Lease {
                generation: lease.generation + 1,
                ..lease
            }
        )
        .await
        .expect("generation fence")
    );
    let other = Shard::new((shard.index() + 1) % SHARDS).expect("other shard");
    assert!(
        !tx.holds_shard(
            &owner,
            Lease {
                shard: other,
                ..lease
            }
        )
        .await
        .expect("run shard fence")
    );
    assert!(
        tx.holds_shard(&owner, lease)
            .await
            .expect("current shard fence")
    );
    let mut contender = world.db.pool.acquire().await.expect("takeover connection");
    sqlx::query("SET lock_timeout='100ms'")
        .execute(&mut *contender)
        .await
        .expect("bounded wait");
    let blocked = sqlx::query("UPDATE scheduler_shards SET generation=generation+1 WHERE shard=$1")
        .bind(i16::from(shard.index()))
        .execute(&mut *contender)
        .await
        .expect_err("shared fence prevents takeover");
    assert_eq!(
        blocked
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("55P03")
    );
    sqlx::query("SET lock_timeout='0'")
        .execute(&mut *contender)
        .await
        .expect("reset wait");
    tx.commit().await.expect("release shared fence");
    sqlx::query("UPDATE scheduler_shards SET generation=generation+1 WHERE shard=$1")
        .bind(i16::from(shard.index()))
        .execute(&mut *contender)
        .await
        .expect("takeover after commit");
    let snapshot = world
        .store
        .load_path(world.admin.tenant_id(), run, task.instance.path().clone())
        .await
        .expect("new snapshot")
        .expect("run");
    let mut tx = world
        .store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("new bounded tx")
        .expect("run");
    assert!(
        !tx.holds_shard(&owner, lease)
            .await
            .expect("old generation refused")
    );
    tx.commit().await.expect("read-only commit");
    sqlx::query("UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 second' WHERE shard=$1")
        .bind(i16::from(shard.index())).execute(&mut *contender).await.expect("expire current ownership");
    let snapshot = world
        .store
        .load_path(world.admin.tenant_id(), run, task.instance.path().clone())
        .await
        .expect("expired owner snapshot")
        .expect("run");
    let mut tx = world
        .store
        .begin_path(snapshot, Lock::Wait)
        .await
        .expect("expired owner tx")
        .expect("run");
    assert!(
        !tx.holds_shard(
            &owner,
            Lease {
                generation: lease.generation + 1,
                ..lease
            }
        )
        .await
        .expect("expired owner refused")
    );
    tx.commit().await.expect("expired owner read commit");
    assert_eq!(stamp().await.expect("after fence reads"), before);
}

#[tokio::test]
async fn a_bounded_transaction_observes_database_time_after_waiting_for_the_run_lock() {
    assert_transaction_clock_after_run_lock(true).await;
}

#[tokio::test]
async fn a_full_transaction_observes_database_time_after_waiting_for_the_run_lock() {
    assert_transaction_clock_after_run_lock(false).await;
}

async fn assert_transaction_clock_after_run_lock(bounded: bool) {
    use sagat_application::execution::RunTx;
    use sagat_application::runs::RunStore;
    let world = world().await;
    allow_iterations(&world, 4).await;
    let run = start_collection(
        &world,
        collection("bounded-lock-clock", "input.items", 1, 1, "bodyResult"),
        &json!({"items":[1]}),
    )
    .await;
    let scheduler = scheduler(&world, "bounded-lock-clock").await;
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    assert_eq!(scheduler.tick().await.collections_advanced, 1);
    let path = InstancePath::root()
        .child(NodeId::parse("items").expect("owner"), 0)
        .expect("path");
    let snapshot = world
        .store
        .load_path(world.admin.tenant_id(), run, path)
        .await
        .expect("snapshot")
        .expect("run");
    let mut writer = world.db.pool.begin().await.expect("writer");
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(run.as_uuid())
        .execute(&mut *writer)
        .await
        .expect("hold run");
    let store = world.store.clone();
    let tenant = world.admin.tenant_id();
    let waiter = tokio::spawn(async move {
        if bounded {
            let tx = store
                .begin_path(snapshot, Lock::Wait)
                .await
                .expect("waiting path tx")
                .expect("run");
            let observed = tx.now();
            tx.commit().await.expect("read-only path commit");
            observed
        } else {
            let tx = store
                .begin(tenant, run, Lock::Wait)
                .await
                .expect("waiting full tx")
                .expect("run");
            let observed = tx.now();
            tx.commit().await.expect("read-only full commit");
            observed
        }
    });
    crate::support::eventually("bounded transaction is waiting on the run row", || async {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
             AND wait_event_type='Lock' AND query LIKE '%FROM runs%FOR UPDATE%')",
        )
        .fetch_one(&world.db.pool)
        .await
        .expect("observe real row-lock wait");
        waiting.then_some(())
    })
    .await;
    let released_at: time::OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *writer)
        .await
        .expect("release time at database");
    writer.rollback().await.expect("release run");
    let observed = waiter.await.expect("bounded transaction completed");
    assert!(
        observed >= released_at,
        "mutation clock predates its run lock (bounded={bounded}): {observed} < {released_at}"
    );
}
