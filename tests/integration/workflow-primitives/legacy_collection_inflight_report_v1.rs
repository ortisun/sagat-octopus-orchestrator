//! One bounded old/current acceptance case: a genuine immutable-old ordinary success is reported
//! before any current scheduler advancement, then the engine's real compensation task is completed.
#![allow(clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use prost::Message as _;
use sagat_api_grpc::pb::{CompleteStepRequest, TaskLease};
use sagat_application::entropy::{SystemEntropy, Token};
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::maintenance::{MaintenanceStore, Window};
use sagat_application::worker::WorkerSettings;
use sagat_domain::execution::RunId;
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::{SHARDS, Shard};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgPool, PgRunStore, PoolConfig, connect, migrate};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::api::{Options, TestApi, start_on_db};
use crate::support::worker::{Platform, TENANT, Worker};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, attach_existing_db};

const FORMAT: &str = "q10c-ordinary-before-drain-v1";
const PINNED_OLD_COMMIT: &str = "6c44a0e49670cf19660792cd1518d241e6c99b3b";

const PROJECTION_WITNESS_SQL: &str = r#"WITH selected_runs AS (
  SELECT run.id FROM runs run WHERE run.tenant_id=$1 AND run.id=$2
) SELECT jsonb_build_object(
  'runs',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id)
    FROM runs row_data JOIN selected_runs selected ON selected.id=row_data.id
    WHERE row_data.tenant_id=$1),
  'usage',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id)
    FROM run_instance_usage row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'scopes',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_path_nodes,row_data.owner_path_indices,row_data.owner_node)
    FROM iteration_scopes row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'steps',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node)
    FROM steps row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'events',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.seq)
    FROM run_events row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'item_sources',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_scope_items row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'iterations',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_iterations row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'checkpoints',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM iteration_checkpoints row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'payloads',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex'))
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT run.input_digest FROM runs run JOIN selected_runs selected ON selected.id=run.id
        WHERE run.tenant_id=$1
      UNION SELECT step.output_digest FROM steps step JOIN selected_runs selected ON selected.id=step.run_id
        WHERE step.tenant_id=$1
      UNION SELECT scope.snapshot_digest FROM iteration_scopes scope
        JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT scope.result_digest FROM iteration_scopes scope
        JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT item.item_digest FROM collection_scope_items item
        JOIN selected_runs selected ON selected.id=item.run_id WHERE item.tenant_id=$1
      UNION SELECT member.item_digest FROM collection_iterations member
        JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT member.result_digest FROM collection_iterations member
        JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint
        JOIN selected_runs selected ON selected.id=checkpoint.run_id WHERE checkpoint.tenant_id=$1
      UNION SELECT event.payload_digest FROM run_events event
        JOIN selected_runs selected ON selected.id=event.run_id WHERE event.tenant_id=$1)),
  'workflow_versions',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id)
    FROM workflow_versions row_data WHERE row_data.tenant_id=$1 AND row_data.id IN (
      SELECT run.workflow_version_id FROM runs run JOIN selected_runs selected ON selected.id=run.id
        WHERE run.tenant_id=$1)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data
    JOIN selected_runs selected ON selected.id=row_data.run_id WHERE row_data.tenant_id=$1),
  'outbox',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id)
    FROM event_outbox row_data JOIN selected_runs selected ON selected.id=row_data.aggregate_id
    WHERE row_data.tenant_id=$1))"#;

const CLOSED_SOURCE_SQL: &str = r#"WITH selected_scopes AS (
  SELECT scope.* FROM iteration_scopes scope
   WHERE scope.tenant_id=$1 AND scope.run_id=$2
     AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[]
     AND scope.owner_node=ANY($3::text[])
), selected_events AS (
  SELECT event.* FROM run_events event
   WHERE event.tenant_id=$1 AND event.run_id=$2 AND event.seq <= $4
) SELECT jsonb_build_object(
  'scopes',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.owner_node)
    FROM selected_scopes row_data),
  'steps',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node)
    FROM steps row_data WHERE row_data.tenant_id=$1 AND row_data.run_id=$2
      AND ((cardinality(row_data.instance_path_nodes)=0 AND row_data.node=ANY($3::text[]))
        OR row_data.instance_path_nodes[1]=ANY($3::text[]))),
  'item_sources',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_scope_items row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'iterations',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_iterations row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'checkpoints',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM iteration_checkpoints row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'events',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.seq)
    FROM selected_events row_data),
  'payloads',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex'))
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT scope.snapshot_digest FROM selected_scopes scope
      UNION SELECT scope.result_digest FROM selected_scopes scope
      UNION SELECT step.output_digest FROM steps step
        WHERE step.tenant_id=$1 AND step.run_id=$2
          AND ((cardinality(step.instance_path_nodes)=0 AND step.node=ANY($3::text[]))
            OR step.instance_path_nodes[1]=ANY($3::text[]))
      UNION SELECT item.item_digest FROM collection_scope_items item JOIN selected_scopes scope
        ON scope.tenant_id=item.tenant_id AND scope.run_id=item.run_id AND scope.scope_id=item.scope_id
      UNION SELECT member.result_digest FROM collection_iterations member JOIN selected_scopes scope
        ON scope.tenant_id=member.tenant_id AND scope.run_id=member.run_id
       AND scope.scope_id=member.scope_id
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint
        JOIN selected_scopes scope ON scope.tenant_id=checkpoint.tenant_id
         AND scope.run_id=checkpoint.run_id AND scope.scope_id=checkpoint.scope_id
      UNION SELECT event.payload_digest FROM selected_events event)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data
    WHERE row_data.tenant_id=$1 AND row_data.run_id=$2 AND row_data.owner_node=ANY($3::text[]))
)"#;

struct Receipt {
    database_name: String,
    producer_api_role: String,
    tenant: TenantId,
    other_tenant: TenantId,
    namespace: Uuid,
    namespace_slug: String,
    worker_subject: String,
    tenant_alias: String,
    worker_id: String,
    session_token: String,
    run: RunId,
    selected_owner: String,
    scheduler_owner: String,
    lease: TaskLease,
    completion: CompleteStepRequest,
    projection_witness: String,
    source_witness: String,
}

#[derive(PartialEq, Eq)]
struct ReportReceipt {
    tenant: Uuid,
    scope: String,
    key: String,
    request_hash: Vec<u8>,
    resource: String,
    created_at: time::OffsetDateTime,
}

type SuccessfulRefusalState = (
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
);

type FinalCompensationState = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    bool,
    String,
    i64,
    i64,
    i64,
    i64,
);

impl Receipt {
    fn parse(bytes: &[u8]) -> Self {
        let mut value: Value =
            serde_json::from_slice(bytes).expect("parse private producer receipt");
        let object = value.as_object_mut().expect("receipt object");
        assert_eq!(object.len(), 19, "receipt has exactly the reviewed fields");
        let mut take = |name: &str| {
            object
                .remove(name)
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| panic!("receipt string: {name}"))
        };
        assert!(
            take("format") == FORMAT,
            "unexpected private receipt format"
        );
        assert!(
            take("source_commit") == PINNED_OLD_COMMIT,
            "unexpected private receipt source commit"
        );
        let database_name = take("database_name");
        assert!(owned_database_name(&database_name));
        let producer_api_role = take("producer_api_role");
        let tenant = take("tenant_id").parse().expect("tenant id");
        let other_tenant = take("other_tenant_id").parse().expect("other tenant id");
        let namespace = Uuid::parse_str(&take("namespace_id")).expect("namespace id");
        let namespace_slug = take("namespace_slug");
        let worker_subject = take("worker_subject");
        let tenant_alias = take("tenant_alias");
        let worker_id = take("worker_id");
        let session_token = take("session_token");
        let run = take("run_id").parse().expect("run id");
        let selected_owner = take("selected_owner");
        let scheduler_owner = take("scheduler_owner");
        let lease_bytes = base64::engine::general_purpose::STANDARD
            .decode(take("task_lease_b64"))
            .expect("task lease base64");
        let completion_bytes = base64::engine::general_purpose::STANDARD
            .decode(take("completion_request_b64"))
            .expect("completion request base64");
        let lease = TaskLease::decode(lease_bytes.as_slice()).expect("task lease protobuf");
        let completion = CompleteStepRequest::decode(completion_bytes.as_slice())
            .expect("completion request protobuf");
        assert!(
            lease.encode_to_vec() == lease_bytes,
            "task lease protobuf is not canonical"
        );
        assert!(
            completion.encode_to_vec() == completion_bytes,
            "completion protobuf is not canonical"
        );
        let projection_witness = take("producer_projection_witness_sha256");
        let source_witness = take("closed_source_witness_sha256");
        assert!(object.is_empty(), "receipt has no unknown fields");
        Self {
            database_name,
            producer_api_role,
            tenant,
            other_tenant,
            namespace,
            namespace_slug,
            worker_subject,
            tenant_alias,
            worker_id,
            session_token,
            run,
            selected_owner,
            scheduler_owner,
            lease,
            completion,
            projection_witness,
            source_witness,
        }
    }
}

fn owned_database_name(name: &str) -> bool {
    name.strip_prefix("t_").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn sha(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("canonical witness JSON"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn projection_witness(pool: &PgPool, tenant: TenantId, run: RunId) -> String {
    let value: Value = sqlx::query_scalar(PROJECTION_WITNESS_SQL)
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("bounded projection witness");
    sha(&value)
}

fn source_nodes() -> Vec<String> {
    (0..10).map(|index| format!("c{index}")).collect()
}

async fn source_witness(pool: &PgPool, tenant: TenantId, run: RunId) -> String {
    let cutoff: i64 = sqlx::query_scalar(
        "SELECT max(seq) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
           AND event_type='step.routed' AND node='c9' \
           AND cardinality(instance_path_nodes)=0",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("closed source cutoff");
    let value: Value = sqlx::query_scalar(CLOSED_SOURCE_SQL)
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .bind(source_nodes())
        .bind(cutoff)
        .fetch_one(pool)
        .await
        .expect("bounded closed source witness");
    sha(&value)
}

async fn identity_witness(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'tenants',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id) \
            FROM (SELECT id,slug FROM tenants WHERE id=ANY($1::uuid[])) row_data), \
          'memberships',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.subject) \
            FROM (SELECT tenant_id,subject,role FROM memberships \
              WHERE tenant_id=$2 AND subject=$3) row_data), \
          'bindings',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.pattern) \
            FROM (SELECT tenant_id,namespace_id,subject,pattern,updated_by FROM worker_bindings \
              WHERE tenant_id=$2 AND namespace_id=$4 AND subject=$3) row_data), \
          'sessions',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id) \
            FROM (SELECT id,tenant_id,namespace_id,subject,worker_name,encode(token_hash,'hex') token_hash, \
                         activities,languages,pools,max_inflight,accepted_protocol_minor \
                    FROM worker_sessions WHERE tenant_id=$2 AND id=$5) row_data))",
    )
    .bind(vec![receipt.tenant.as_uuid(), receipt.other_tenant.as_uuid()])
    .bind(receipt.tenant.as_uuid())
    .bind(&receipt.worker_subject)
    .bind(receipt.namespace)
    .bind(Uuid::parse_str(&receipt.worker_id).expect("worker id"))
    .fetch_one(pool)
    .await
    .expect("durable worker authority witness");
    sha(&value)
}

fn current_options() -> Options {
    Options {
        runtime_role: crate::support::as_runtime_role(),
        worker: WorkerSettings {
            lease_ttl: Duration::from_secs(3_600),
            session_ttl: Duration::from_secs(3_600),
            ..WorkerSettings::default()
        },
        ..Options::default()
    }
}

fn assert_wire_identity(receipt: &Receipt) {
    assert_eq!(receipt.namespace_slug, "payments");
    assert_eq!(receipt.tenant_alias, TENANT);
    assert_eq!(receipt.selected_owner, "c10");
    assert_eq!(receipt.lease.run_id, receipt.run.to_string());
    // The tenant is authenticated by the worker identity, not disclosed in the lease.
    assert!(receipt.lease.tenant_alias.is_empty());
    assert_eq!(receipt.lease.task_kind, "FORWARD");
    assert_eq!(receipt.lease.activity_name, "line.reserve");
    assert!(
        receipt.completion.output_json == "1",
        "retained completion output differs from the reviewed ordinary value"
    );
    assert!(receipt.completion.output_blob_ref.is_empty());
    assert!(receipt.completion.output_sha256.is_empty());
    assert!(2_100_000 + receipt.completion.output_json.len() < 4 * 1024 * 1024);
    let instance = receipt
        .lease
        .instance
        .as_ref()
        .expect("scoped lease identity");
    assert_eq!(instance.definition_node_id, "reserve");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, "c10");
    assert_eq!(instance.instance_path[0].iteration_index, 0);
    let mut expected = Worker::complete_message(&receipt.lease, "1");
    expected.idempotency_key = receipt.completion.idempotency_key.clone();
    assert!(
        expected.encode_to_vec() == receipt.completion.encode_to_vec(),
        "retained completion differs from the original lease identity"
    );
}

async fn effective_totals(pool: &PgPool, receipt: &Receipt) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT usage.logical_output_bytes, \
           COALESCE(sum(octet_length(payload.content)) \
             FILTER (WHERE owner.output_digest IS NULL),0)::bigint, \
           usage.logical_output_bytes + COALESCE(sum(octet_length(payload.content)) \
             FILTER (WHERE owner.output_digest IS NULL),0)::bigint \
         FROM run_instance_usage usage \
         JOIN iteration_scopes scope ON scope.tenant_id=usage.tenant_id AND scope.run_id=usage.run_id \
         JOIN steps owner ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id \
          AND owner.instance_path_nodes='{}'::text[] \
          AND owner.instance_path_indices='{}'::integer[] AND owner.node=scope.owner_node \
         JOIN payloads payload ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest \
        WHERE usage.tenant_id=$1 AND usage.run_id=$2 AND scope.lifecycle='SUCCEEDED' \
          AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[] \
        GROUP BY usage.logical_output_bytes",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("effective legacy totals")
}

async fn outboxes(pool: &PgPool, receipt: &Receipt) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2")
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("run outbox count")
}

async fn payloads(pool: &PgPool, receipt: &Receipt) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM payloads WHERE tenant_id=$1")
        .bind(receipt.tenant.as_uuid())
        .fetch_one(pool)
        .await
        .expect("tenant payload count")
}

async fn assert_live_target(pool: &PgPool, receipt: &Receipt) {
    let live: (String, i32, i64, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT step.status,step.attempt,step.generation,step.step_id=$4, \
                step.lease_token_hash=$5,step.leased_by=$6, \
                step.lease_expires_at>clock_timestamp(), \
                session.token_hash=$7 AND session.expires_at>clock_timestamp() \
           FROM steps step JOIN worker_sessions session \
             ON session.tenant_id=step.tenant_id AND session.id::text=step.leased_by \
          WHERE step.tenant_id=$1 AND step.run_id=$2 AND step.namespace_id=$3 \
            AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(receipt.namespace)
    .bind(Uuid::parse_str(&receipt.lease.step_id).expect("step id"))
    .bind(Token::hash_of(&receipt.lease.lease_token))
    .bind(&receipt.worker_id)
    .bind(Token::hash_of(&receipt.session_token))
    .fetch_one(pool)
    .await
    .expect("current live old target");
    assert_eq!(
        (live.0.as_str(), live.1, live.2),
        ("LEASED", receipt.lease.attempt, receipt.lease.generation)
    );
    assert!(live.3 && live.4 && live.5 && live.6 && live.7);
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000;
    assert!(receipt.lease.expires_at_epoch_ms > now_ms);
}

fn completion_fingerprint(receipt: &Receipt) -> Vec<u8> {
    let attempt = u32::try_from(receipt.completion.attempt).expect("nonnegative original attempt");
    let generation =
        u64::try_from(receipt.completion.generation).expect("nonnegative original generation");
    let attempt = attempt.to_be_bytes();
    let generation = generation.to_be_bytes();
    let parts: [&[u8]; 4] = [
        b"complete",
        &attempt,
        &generation,
        receipt.completion.output_json.as_bytes(),
    ];
    let mut fingerprint = Vec::new();
    for part in parts {
        fingerprint.extend_from_slice(&(part.len() as u64).to_be_bytes());
        fingerprint.extend_from_slice(part);
    }
    fingerprint
}

fn report_scope(receipt: &Receipt) -> String {
    use std::fmt::Write as _;

    let mut token_prefix = String::with_capacity(16);
    for byte in Token::hash_of(&receipt.lease.lease_token).iter().take(8) {
        let _ = write!(token_prefix, "{byte:02x}");
    }
    format!(
        "step:{}:scoped:3:c10:0;7:reserve:{token_prefix}",
        receipt.run
    )
}

async fn verified_report_receipt(
    pool: &PgPool,
    receipt: &Receipt,
    command_id: &str,
) -> ReportReceipt {
    let head: (i64, i64, i64) = sqlx::query_as(
        "SELECT run.run_version, \
          (SELECT COALESCE(max(event.seq),0)::bigint FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id), \
          telemetry.head_seq \
         FROM runs run JOIN telemetry_pending telemetry \
           ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
        WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("committed report head");
    assert_eq!(head.0, head.1, "run version differs from the journal head");
    assert_eq!(head.0, head.2, "telemetry differs from the journal head");
    let expected_command = format!("{}:{}", receipt.run, head.0);
    assert!(
        command_id == expected_command,
        "report ACK command does not identify the committed journal head"
    );

    let mut rows: Vec<(Uuid, String, String, Vec<u8>, String, time::OffsetDateTime)> =
        sqlx::query_as(
            "SELECT tenant_id,scope,idempotency_key,request_hash,resource_id,created_at \
         FROM command_dedupe \
          WHERE tenant_id=$1 AND idempotency_key=$2",
        )
        .bind(receipt.tenant.as_uuid())
        .bind(&receipt.completion.idempotency_key)
        .fetch_all(pool)
        .await
        .expect("complete report receipt identity");
    assert!(
        rows.len() == 1,
        "expected exactly one receipt for the original tenant and key"
    );
    let (tenant, scope, key, request_hash, resource, created_at) =
        rows.pop().expect("one checked receipt");
    assert!(
        tenant == receipt.tenant.as_uuid(),
        "report receipt changed the original tenant identity"
    );
    assert!(
        scope == report_scope(receipt),
        "report receipt has a noncanonical scoped lease identity"
    );
    assert!(
        key == receipt.completion.idempotency_key,
        "report receipt changed the original idempotency key"
    );
    let expected_hash = Sha256::digest(completion_fingerprint(receipt)).to_vec();
    assert!(
        request_hash == expected_hash,
        "report receipt fingerprint does not match the original request"
    );
    let expected_resource = format!("SUCCEEDED|CANCELLING|{}", head.0);
    assert!(
        resource == expected_resource,
        "report receipt resource does not match the committed ACK"
    );
    ReportReceipt {
        tenant,
        scope,
        key,
        request_hash,
        resource,
        created_at,
    }
}

async fn report_identity(pool: &PgPool, receipt: &Receipt) -> (i64, i64, i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_type='run' \
            AND resource_id=$2::text AND action='run.data_limit_exceeded' \
            AND reason='successful scoped output refused by run data limit'), \
          (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
            AND event_type='step.completed' AND node='reserve' \
            AND instance_path_nodes=ARRAY['c10']::text[] \
            AND instance_path_indices=ARRAY[0]::integer[]), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2), \
          (SELECT head_seq FROM telemetry_pending WHERE tenant_id=$1 AND run_id=$2), \
          (SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2), \
          (SELECT logical_output_bytes FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2)",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("report identity")
}

async fn assert_successful_refusal(
    api: &TestApi,
    receipt: &Receipt,
    outboxes_before: i64,
    payloads_before: i64,
) {
    let state: SuccessfulRefusalState = sqlx::query_as(
        "SELECT run.status,run.stop_reason,run.stop_code,scope.lifecycle,step.status, \
                    step.output_digest IS NULL,step.reported_lease_token_hash=$4, \
                    step.attempt=$5 AND step.generation=$6, \
                    telemetry.head_seq,run.run_version \
               FROM runs run JOIN iteration_scopes scope \
                 ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
                AND scope.owner_node='c10' AND scope.owner_path_nodes='{}'::text[] \
                AND scope.owner_path_indices='{}'::integer[] \
               JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
                AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
                AND step.instance_path_indices=ARRAY[0]::integer[] \
               JOIN telemetry_pending telemetry \
                 ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
              WHERE run.tenant_id=$1 AND run.id=$2 AND step.namespace_id=$3",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(receipt.namespace)
    .bind(Token::hash_of(&receipt.lease.lease_token))
    .bind(receipt.lease.attempt)
    .bind(receipt.lease.generation)
    .fetch_one(&api.db.pool)
    .await
    .expect("successful refusal projection");
    assert_eq!(state.0, "CANCELLING");
    assert_eq!(state.1.as_deref(), Some("FAILED"));
    assert_eq!(state.2.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(state.3, "OPEN");
    assert_eq!(state.4, "SUCCEEDED");
    assert!(state.5 && state.6 && state.7);
    assert_eq!(
        state.8, state.9,
        "telemetry reaches the committed journal head"
    );

    let event: (i64, Option<String>, bool, bool) = sqlx::query_as(
        "SELECT count(*),min(actor),bool_and(payload_digest IS NULL), \
                bool_and((detail->>'attempt')::integer=$3 \
                  AND (detail->>'generation')::bigint=$4) \
           FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
            AND event_type='step.completed' AND node='reserve' \
            AND instance_path_nodes=ARRAY['c10']::text[] \
            AND instance_path_indices=ARRAY[0]::integer[]",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(receipt.lease.attempt)
    .bind(receipt.lease.generation)
    .fetch_one(&api.db.pool)
    .await
    .expect("worker event provenance");
    assert_eq!(event.0, 1);
    let expected_actor = format!("worker:{}", receipt.worker_subject);
    assert_eq!(event.1.as_deref(), Some(expected_actor.as_str()));
    assert!(event.2 && event.3);

    let successors: (i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='returned' \
            AND instance_path_nodes=ARRAY['c10']::text[] \
            AND instance_path_indices=ARRAY[0]::integer[]), \
          (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='done' \
            AND cardinality(instance_path_nodes)=0)",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("refused forward successors");
    assert_eq!(successors, (0, 0));

    let audit: (i64, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT count(*),min(audit.actor),min(audit.reason), \
                bool_and(audit.trace_id IS NOT DISTINCT FROM run.trace_id) \
           FROM audit_records audit JOIN runs run \
             ON run.tenant_id=audit.tenant_id AND run.id=$2 \
          WHERE audit.tenant_id=$1 AND audit.resource_type='run' \
            AND audit.resource_id=$2::text AND audit.action='run.data_limit_exceeded' \
            AND audit.reason='successful scoped output refused by run data limit'",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("report refusal audit");
    assert_eq!(
        audit,
        (
            1,
            Some("system:engine".into()),
            Some("successful scoped output refused by run data limit".into()),
            true,
        )
    );

    assert_eq!(outboxes(&api.db.pool, receipt).await, outboxes_before);
    assert_eq!(payloads(&api.db.pool, receipt).await, payloads_before);
    assert_eq!(
        effective_totals(&api.db.pool, receipt).await,
        (2_100_000, 2_100_020, 4_200_020)
    );
    assert!(
        source_witness(&api.db.pool, receipt.tenant, receipt.run).await == receipt.source_witness,
        "closed source witness changed after the report"
    );

    let candidates = api
        .executions
        .collection_candidates(&Window::everything(8))
        .await
        .expect("discover stopped open collection without a scheduler call");
    assert!(
        candidates.iter().any(|candidate| {
            candidate.run_id == receipt.run && candidate.node.as_str() == "c10"
        })
    );
}

async fn fresh_verify(api: &TestApi, receipt: &Receipt) {
    let url = if crate::support::as_runtime_role() {
        api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
    } else {
        api.db.url.clone()
    };
    let pool = connect(&PoolConfig::new(url))
        .await
        .expect("fresh verification pool");
    let store = PgRunStore::new(pool, DefinitionCache::new(Arc::new(JsonIrCodec), 8))
        .with_scoped_worker_protocol();
    assert!(
        store.verify_run(receipt.tenant, receipt.run).await.is_ok(),
        "cold journal projection replay"
    );
}

async fn poll_compensation(
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    worker: &mut crate::support::worker::Worker<'_>,
) -> TaskLease {
    for _ in 0..32 {
        let tasks = worker.poll(8).await;
        if !tasks.is_empty() {
            assert_eq!(tasks.len(), 1, "only the selected compensation is offered");
            return tasks.into_iter().next().expect("one compensation");
        }
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "compensation scheduler pass: {report:?}");
    }
    panic!("the real compensation task did not become available");
}

async fn finish_after_compensation(
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    api: &TestApi,
    receipt: &Receipt,
) {
    for _ in 0..40 {
        let status: String =
            sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(receipt.tenant.as_uuid())
                .bind(receipt.run.as_uuid())
                .fetch_one(&api.db.pool)
                .await
                .expect("run status");
        if status == "COMPENSATED" {
            return;
        }
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "final scheduler pass: {report:?}");
    }
    panic!("the compensated run did not reach its real terminal outcome");
}

#[tokio::test]
#[ignore = "requires scripts/test-legacy-collection-inflight-report-v1.sh"]
async fn old_ordinary_success_before_drain_preserves_the_report_and_compensation() {
    let receipt_path = std::env::var_os("SAGAT_Q10C_ORDINARY_PHASE_RECEIPT")
        .map(std::path::PathBuf::from)
        .expect("phased runner receipt path");
    assert_eq!(
        std::fs::metadata(&receipt_path)
            .expect("private receipt metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let receipt = Receipt::parse(&std::fs::read(&receipt_path).expect("read private receipt"));
    assert_wire_identity(&receipt);

    let admin_url = std::env::var("SAGAT_TEST_DATABASE_URL")
        .expect("coordinator-owned external PostgreSQL admin URL");
    let db = attach_existing_db(&admin_url, &receipt.database_name).await;
    let actual_database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&db.pool)
        .await
        .expect("retained database identity");
    assert_eq!(actual_database, receipt.database_name);
    assert!(
        projection_witness(&db.pool, receipt.tenant, receipt.run).await
            == receipt.projection_witness,
        "immutable-old projection witness changed before migration"
    );
    assert!(
        source_witness(&db.pool, receipt.tenant, receipt.run).await == receipt.source_witness,
        "closed source witness changed before migration"
    );
    let authority_before = identity_witness(&db.pool, &receipt).await;

    migrate(&db.pool)
        .await
        .expect("apply current migrations as owner");
    assert!(
        projection_witness(&db.pool, receipt.tenant, receipt.run).await
            == receipt.projection_witness,
        "immutable-old projection witness changed during migration"
    );
    assert!(
        source_witness(&db.pool, receipt.tenant, receipt.run).await == receipt.source_witness,
        "closed source witness changed during migration"
    );
    let api = start_on_db(db, current_options()).await;
    let platform = Platform {
        api,
        tenant: receipt.tenant,
        other_tenant: receipt.other_tenant,
    };
    let current_api_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.api_pool)
        .await
        .expect("consumer API database role");
    assert_eq!(current_api_role, receipt.producer_api_role);
    if crate::support::as_runtime_role() {
        assert_eq!(receipt.producer_api_role, RUNTIME_ROLE);
        assert_eq!(current_api_role, RUNTIME_ROLE);
    } else {
        assert_ne!(receipt.producer_api_role, RUNTIME_ROLE);
        assert_ne!(current_api_role, RUNTIME_ROLE);
    }
    eprintln!(
        "q10c ordinary-before-drain API roles: producer={}, consumer={current_api_role}",
        receipt.producer_api_role
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "persisted worker authority changed during current API startup"
    );
    assert_live_target(&platform.api.db.pool, &receipt).await;
    assert_eq!(
        effective_totals(&platform.api.db.pool, &receipt).await,
        (2_100_000, 2_100_020, 4_200_020)
    );

    let mut worker = platform
        .connect(&receipt.worker_subject, &receipt.tenant_alias)
        .await;
    worker.worker_id = receipt.worker_id.clone();
    worker.session = receipt.session_token.clone();
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "transport connection changed persisted worker authority"
    );

    let outboxes_before = outboxes(&platform.api.db.pool, &receipt).await;
    let payloads_before = payloads(&platform.api.db.pool, &receipt).await;
    let lookups_before = platform.api.authentication_lookups();
    let request = worker.req(receipt.completion.clone());
    let accepted = worker
        .client
        .complete_step(request)
        .await
        .expect("submit exact immutable-old completion under current authentication")
        .into_inner();
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert!(platform.api.authentication_lookups() > lookups_before);
    assert_successful_refusal(&platform.api, &receipt, outboxes_before, payloads_before).await;
    let original_report_receipt =
        verified_report_receipt(&platform.api.db.pool, &receipt, &accepted.command_id).await;
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "ordinary report changed persisted worker authority"
    );

    fresh_verify(&platform.api, &receipt).await;
    let first_identity = report_identity(&platform.api.db.pool, &receipt).await;
    let replay_request = worker.req(receipt.completion.clone());
    let replay = worker
        .client
        .complete_step(replay_request)
        .await
        .expect("exact report receipt replay under fresh current authentication")
        .into_inner();
    assert!(replay.already_applied);
    assert_eq!(replay.command_id, accepted.command_id);
    assert_eq!(replay.step_status, "SUCCEEDED");
    assert_eq!(replay.run_status, "CANCELLING");
    assert_eq!(
        report_identity(&platform.api.db.pool, &receipt).await,
        first_identity
    );
    let replayed_report_receipt =
        verified_report_receipt(&platform.api.db.pool, &receipt, &replay.command_id).await;
    assert!(
        replayed_report_receipt == original_report_receipt,
        "exact replay changed the complete report receipt identity"
    );
    assert_eq!(
        payloads(&platform.api.db.pool, &receipt).await,
        payloads_before
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "exact replay changed persisted worker authority"
    );

    let stored_owner: (i64, i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE owner IS NOT NULL), \
                count(DISTINCT owner) FILTER (WHERE owner IS NOT NULL),min(owner),max(owner) \
           FROM scheduler_shards",
    )
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("immutable-old scheduler owner");
    assert_eq!(stored_owner.0, i64::from(SHARDS));
    assert_eq!(stored_owner.1, 1);
    assert_eq!(
        stored_owner.2.as_deref(),
        Some(receipt.scheduler_owner.as_str())
    );
    assert_eq!(
        stored_owner.3.as_deref(),
        Some(receipt.scheduler_owner.as_str())
    );

    let placement = Placement::new(receipt.scheduler_owner.clone());
    let round = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("renew immutable-old scheduler identity");
    assert_eq!(round.replicas, 1);
    assert_eq!(round.held, usize::from(SHARDS));
    assert!(placement.owned().contains(Shard::of(receipt.run.as_uuid())));
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);

    let drain_report = scheduler.tick().await;
    assert_eq!(
        drain_report.failed, 0,
        "normal durable discovery: {drain_report:?}"
    );
    assert_eq!(drain_report.collections_advanced, 1, "{drain_report:?}");
    let drain: (String, String, Option<String>, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
            AND event_type='run.stop_requested' AND detail->>'code'='RUN_DATA_LIMIT'), \
          (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
            AND event_type='collection.drain_started' AND node='c10'), \
          (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 \
            AND event_type='collection.drain.started' AND routing_key='c10'), \
          (SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_type='run' \
            AND resource_id=$2::text AND action='run.data_limit_exceeded' \
            AND reason='stopped_for_run_data_limit'), \
          (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND kind='FORWARD' \
            AND status IN ('READY','LEASED','RUNNING')) \
         FROM iteration_scopes scope WHERE scope.tenant_id=$1 AND scope.run_id=$2 \
          AND scope.owner_node='c10' AND scope.owner_path_nodes='{}'::text[] \
          AND scope.owner_path_indices='{}'::integer[]",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("normal stopped-limit drain");
    assert_eq!(
        drain,
        (
            "DRAINING".into(),
            "RUN_DATA_LIMIT".into(),
            Some("RUN_DATA_LIMIT".into()),
            1,
            1,
            1,
            1,
            0,
        )
    );
    assert_eq!(
        outboxes(&platform.api.db.pool, &receipt).await,
        outboxes_before + 1
    );
    assert!(
        source_witness(&platform.api.db.pool, receipt.tenant, receipt.run).await
            == receipt.source_witness,
        "closed source witness changed during collection drain"
    );

    let compensation = poll_compensation(&scheduler, &mut worker).await;
    assert_eq!(compensation.task_kind, "COMPENSATION");
    assert_eq!(compensation.activity_name, "line.release");
    assert_eq!(compensation.target_node, "reserve");
    assert_eq!(
        compensation.target_idempotency_key,
        receipt.lease.idempotency_key
    );
    assert_eq!(compensation.target_instance, receipt.lease.instance);
    let compensation_instance = compensation
        .instance
        .as_ref()
        .expect("compensation identity");
    assert_eq!(compensation_instance.definition_node_id, "reserve~c");
    assert_eq!(compensation_instance.instance_path.len(), 1);
    assert_eq!(compensation_instance.instance_path[0].scope_node_id, "c10");
    assert_eq!(compensation_instance.instance_path[0].iteration_index, 0);
    let before_compensation: (String, String, Option<String>) = sqlx::query_as(
        "SELECT run.status,saga.phase,saga.outcome FROM runs run JOIN scope_sagas saga \
          ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
         WHERE run.tenant_id=$1 AND run.id=$2 AND saga.owner_node='c10'",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("pre-compensation state");
    assert_eq!(before_compensation.0, "COMPENSATING");
    assert_eq!(before_compensation.1, "COMPENSATING");
    assert!(before_compensation.2.is_none());

    let compensation_ack = worker
        .complete(&compensation, &json!({"released": true}))
        .await
        .expect("complete the real engine compensation task");
    assert!(!compensation_ack.already_applied);
    assert_eq!(compensation_ack.step_status, "SUCCEEDED");
    finish_after_compensation(&scheduler, &platform.api, &receipt).await;

    let final_state: FinalCompensationState = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                    saga.phase,saga.outcome,forward.status,forward.output_digest IS NULL, \
                    compensation.status, \
                    (SELECT count(*) FROM scope_saga_entries entry \
                      WHERE entry.tenant_id=$1 AND entry.run_id=$2 AND entry.owner_node='c10' \
                        AND entry.iteration_index=0 AND entry.node='reserve' AND entry.kind='COMPENSATE'), \
                    (SELECT count(*) FROM run_events event \
                      WHERE event.tenant_id=$1 AND event.run_id=$2 \
                        AND event.instance_path_nodes=ARRAY['c10']::text[] \
                        AND event.instance_path_indices=ARRAY[0]::integer[] \
                        AND event.node='reserve~c' AND event.event_type='step.completed'), \
                    telemetry.head_seq,run.run_version \
               FROM iteration_scopes scope JOIN scope_sagas saga \
                 ON saga.tenant_id=scope.tenant_id AND saga.run_id=scope.run_id \
                AND saga.owner_path_nodes=scope.owner_path_nodes \
                AND saga.owner_path_indices=scope.owner_path_indices \
                AND saga.owner_node=scope.owner_node \
               JOIN runs run ON run.tenant_id=scope.tenant_id AND run.id=scope.run_id \
               JOIN telemetry_pending telemetry ON telemetry.tenant_id=run.tenant_id \
                AND telemetry.run_id=run.id \
               JOIN steps forward ON forward.tenant_id=run.tenant_id AND forward.run_id=run.id \
                AND forward.node='reserve' AND forward.instance_path_nodes=ARRAY['c10']::text[] \
                AND forward.instance_path_indices=ARRAY[0]::integer[] \
               JOIN steps compensation ON compensation.tenant_id=run.tenant_id \
                AND compensation.run_id=run.id AND compensation.node='reserve~c' \
                AND compensation.instance_path_nodes=ARRAY['c10']::text[] \
                AND compensation.instance_path_indices=ARRAY[0]::integer[] \
              WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='c10'",
        )
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("final compensated state");
    assert_eq!(final_state.0, "COMPENSATED");
    assert_eq!(final_state.1, "FAILED");
    assert_eq!(final_state.2, "RUN_DATA_LIMIT");
    assert_eq!(final_state.3.as_deref(), Some("RUN_DATA_LIMIT"));
    assert_eq!(final_state.4, "DONE");
    assert_eq!(final_state.5.as_deref(), Some("COMPENSATED"));
    assert_eq!(final_state.6, "SUCCEEDED");
    assert!(final_state.7);
    assert_eq!(final_state.8, "SUCCEEDED");
    assert_eq!(final_state.9, 1);
    assert_eq!(final_state.10, 1);
    assert_eq!(final_state.11, final_state.12);
    assert_eq!(
        effective_totals(&platform.api.db.pool, &receipt).await,
        (2_100_000, 2_100_020, 4_200_020)
    );
    assert!(
        source_witness(&platform.api.db.pool, receipt.tenant, receipt.run).await
            == receipt.source_witness,
        "closed source witness changed during compensation"
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "compensation changed persisted worker authority"
    );
    fresh_verify(&platform.api, &receipt).await;
}
