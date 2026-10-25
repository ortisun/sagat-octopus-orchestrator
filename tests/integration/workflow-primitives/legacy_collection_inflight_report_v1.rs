//! Bounded immutable-old/current ordinary-success controls: both retained-byte conditions,
//! report/drain/cancellation order, live-work guard and original-report/frontier CAS.
//! The old-only Waiting guard control is separate from positive upgrade handoffs.
#![allow(clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use prost::Message as _;
use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::{CompleteStepRequest, TaskLease};
use sagat_application::entropy::{SystemEntropy, Token};
use sagat_application::execution::{DefinitionCache, Engine, RunTx};
use sagat_application::maintenance::{MaintenanceStore, Window};
use sagat_application::primitives::{
    CollectionDataLimitEvidence, CollectionDrain, CollectionScopeId, CollectionStore,
    CollectionTransition, PrimitiveStoreError, SchedulerAuthority,
};
use sagat_application::runs::{Lock, RunStore};
use sagat_application::worker::WorkerSettings;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::{ExecutionInstanceId, RunId};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::sharding::{SHARDS, Shard};
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgCollectionStore, PgPool, PgRunStore, PoolConfig, connect, migrate};
use sagat_scheduler::{
    CollectionAdvance, Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::Row as _;
use uuid::Uuid;

use crate::support::api::{Options, TestApi, start_on_db};
use crate::support::worker::{Platform, TENANT, Worker};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, attach_existing_db};

const FORMAT: &str = "q10c-ordinary-before-drain-v1";
const UNDER_LIMIT_FORMAT: &str = "q10c-ordinary-under-limit-increment-v1";
const PINNED_OLD_COMMIT: &str = "6c44a0e49670cf19660792cd1518d241e6c99b3b";
const CANCELLATION_KEY: &str = "legacy-cancellation-before-report-v1";
const CANCELLATION_REASON: &str = "Cancel before the legacy drain";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FixturePreset {
    Ordinary,
    UnderLimitIncrement,
}

impl FixturePreset {
    const fn format(self) -> &'static str {
        match self {
            Self::Ordinary => FORMAT,
            Self::UnderLimitIncrement => UNDER_LIMIT_FORMAT,
        }
    }

    const fn expected_totals(self) -> (i64, i64, i64) {
        match self {
            Self::Ordinary => (2_100_000, 2_100_020, 4_200_020),
            Self::UnderLimitIncrement => (2_000_000, 2_000_020, 4_000_020),
        }
    }

    fn completion_json(self) -> String {
        match self {
            Self::Ordinary => "1".to_owned(),
            Self::UnderLimitIncrement => serde_json::to_string(&Value::String("y".repeat(249_998)))
                .expect("fixed under-limit completion JSON"),
        }
    }
}

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
    preset: FixturePreset,
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

#[derive(PartialEq, Eq)]
struct OperationReceipt {
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
    bool,
    String,
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
        Self::parse_for(bytes, FixturePreset::Ordinary)
    }

    fn parse_for(bytes: &[u8], preset: FixturePreset) -> Self {
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
            take("format") == preset.format(),
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
            preset,
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
    let expected_output = receipt.preset.completion_json();
    assert!(
        receipt.completion.output_json == expected_output,
        "retained completion output differs from the selected fixed value"
    );
    assert!(receipt.completion.output_blob_ref.is_empty());
    assert!(receipt.completion.output_sha256.is_empty());
    assert!(receipt.completion.output_json.len() < 262_144);
    let expected_totals = receipt.preset.expected_totals();
    assert!(
        expected_totals.0
            + i64::try_from(receipt.completion.output_json.len()).expect("candidate length")
            < 4 * 1024 * 1024
    );
    if receipt.preset == FixturePreset::UnderLimitIncrement {
        assert_eq!(receipt.completion.output_json.len(), 250_000);
        assert_eq!(
            expected_totals.0
                + i64::try_from(receipt.completion.output_json.len()).expect("candidate length"),
            2_250_000
        );
        assert_eq!(
            expected_totals.2
                + i64::try_from(receipt.completion.output_json.len()).expect("candidate length"),
            4_250_020
        );
        assert!(
            expected_totals.2
                + i64::try_from(receipt.completion.output_json.len()).expect("candidate length")
                > 4 * 1024 * 1024
        );
    }
    let instance = receipt
        .lease
        .instance
        .as_ref()
        .expect("scoped lease identity");
    assert_eq!(instance.definition_node_id, "reserve");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].scope_node_id, "c10");
    assert_eq!(instance.instance_path[0].iteration_index, 0);
    let mut expected = Worker::complete_message(&receipt.lease, &expected_output);
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

async fn assert_live_target(pool: &PgPool, receipt: &Receipt, cancel_requested: bool) {
    let live: (String, i32, i64, bool, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT step.status,step.attempt,step.generation,step.step_id=$4, \
                step.lease_token_hash=$5,step.leased_by=$6, \
                step.lease_expires_at>clock_timestamp(), \
                session.token_hash=$7 AND session.expires_at>clock_timestamp(), \
                step.cancel_requested=$8 \
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
    .bind(cancel_requested)
    .fetch_one(pool)
    .await
    .expect("current live old target");
    assert_eq!(
        (live.0.as_str(), live.1, live.2),
        ("LEASED", receipt.lease.attempt, receipt.lease.generation)
    );
    assert!(
        live.3 && live.4 && live.5 && live.6 && live.7 && live.8,
        "the original attempt, lease, session and cancellation state must remain exact"
    );
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

fn cancel_operation_fingerprint(expected_version: i64) -> Vec<u8> {
    let expected_version = u64::try_from(expected_version)
        .expect("the retained journal version must be nonnegative")
        .to_be_bytes();
    let parts: [&[u8]; 3] = [b"cancel", CANCELLATION_REASON.as_bytes(), &expected_version];
    let mut fingerprint = Vec::new();
    for part in parts {
        fingerprint.extend_from_slice(&(part.len() as u64).to_be_bytes());
        fingerprint.extend_from_slice(part);
    }
    fingerprint
}

fn expected_operation_id(receipt: &Receipt) -> String {
    let digest =
        Sha256::digest(format!("operation/{}/{}", receipt.run, CANCELLATION_KEY).as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn operation_scope(receipt: &Receipt) -> String {
    format!("run.operate:{}", receipt.run)
}

async fn verified_operation_receipt(
    pool: &PgPool,
    receipt: &Receipt,
    expected_version: i64,
) -> OperationReceipt {
    let scope = operation_scope(receipt);
    let mut rows: Vec<(Uuid, String, String, Vec<u8>, String, time::OffsetDateTime)> =
        sqlx::query_as(
            "SELECT tenant_id,scope,idempotency_key,request_hash,resource_id,created_at \
               FROM command_dedupe \
              WHERE tenant_id=$1 AND scope=$2 AND idempotency_key=$3",
        )
        .bind(receipt.tenant.as_uuid())
        .bind(&scope)
        .bind(CANCELLATION_KEY)
        .fetch_all(pool)
        .await
        .expect("exact public cancellation receipt");
    assert!(
        rows.len() == 1,
        "public cancellation must have one exact receipt"
    );
    let (tenant, stored_scope, key, request_hash, resource, created_at) =
        rows.pop().expect("one checked public cancellation receipt");
    assert!(
        tenant == receipt.tenant.as_uuid(),
        "public cancellation receipt changed tenant"
    );
    assert!(
        stored_scope == scope,
        "public cancellation receipt changed scope"
    );
    assert!(
        key == CANCELLATION_KEY,
        "public cancellation receipt changed key"
    );
    let expected_hash = Sha256::digest(cancel_operation_fingerprint(expected_version)).to_vec();
    assert!(
        request_hash == expected_hash,
        "public cancellation receipt changed request fingerprint"
    );
    assert!(
        resource == "cancel",
        "public cancellation receipt changed resource"
    );
    OperationReceipt {
        tenant,
        scope: stored_scope,
        key,
        request_hash,
        resource,
        created_at,
    }
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

    let expected_scope = report_scope(receipt);
    let mut rows: Vec<(Uuid, String, String, Vec<u8>, String, time::OffsetDateTime)> =
        sqlx::query_as(
            "SELECT tenant_id,scope,idempotency_key,request_hash,resource_id,created_at \
         FROM command_dedupe \
          WHERE tenant_id=$1 AND scope=$2 AND idempotency_key=$3",
        )
        .bind(receipt.tenant.as_uuid())
        .bind(&expected_scope)
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
        scope == expected_scope,
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

async fn stored_report_receipt(pool: &PgPool, receipt: &Receipt) -> ReportReceipt {
    let scope = report_scope(receipt);
    let row: (Uuid, String, String, Vec<u8>, String, time::OffsetDateTime) = sqlx::query_as(
        "SELECT tenant_id,scope,idempotency_key,request_hash,resource_id,created_at \
           FROM command_dedupe WHERE tenant_id=$1 AND scope=$2 AND idempotency_key=$3",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(&scope)
    .bind(&receipt.completion.idempotency_key)
    .fetch_one(pool)
    .await
    .expect("exact original complete report receipt");
    ReportReceipt {
        tenant: row.0,
        scope: row.1,
        key: row.2,
        request_hash: row.3,
        resource: row.4,
        created_at: row.5,
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
    expected_scope_lifecycle: &str,
    expected_stop_reason: &str,
    expected_stop_code: Option<&str>,
) {
    let state: SuccessfulRefusalState = sqlx::query_as(
        "SELECT run.status,run.stop_reason,run.stop_code,scope.lifecycle,step.status, \
                    step.output_digest IS NULL,step.reported_lease_token_hash=$4, \
                    step.attempt=$5 AND step.generation=$6,step.cancel_requested, \
                    member.lifecycle,member.slot_held, \
                    telemetry.head_seq,run.run_version \
               FROM runs run JOIN iteration_scopes scope \
                 ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
                AND scope.owner_node='c10' AND scope.owner_path_nodes='{}'::text[] \
                AND scope.owner_path_indices='{}'::integer[] \
               JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
                AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
                AND step.instance_path_indices=ARRAY[0]::integer[] \
               JOIN collection_iterations member ON member.tenant_id=scope.tenant_id \
                AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
                AND member.iteration_index=0 \
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
    assert_eq!(state.1.as_deref(), Some(expected_stop_reason));
    assert_eq!(state.2.as_deref(), expected_stop_code);
    assert_eq!(state.3, expected_scope_lifecycle);
    assert_eq!(state.4, "SUCCEEDED");
    assert!(state.5 && state.6 && state.7);
    assert!(
        !state.8,
        "successful report clears the cancellation request"
    );
    assert_eq!(state.9, "ADMITTED");
    assert!(
        state.10,
        "the scheduler has not drained the reported member yet"
    );
    assert_eq!(
        state.11, state.12,
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
        receipt.preset.expected_totals()
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

async fn assert_under_limit_report_boundary(pool: &PgPool, receipt: &Receipt) {
    let row = sqlx::query(
        "SELECT \
          (SELECT min(event.seq) FROM run_events event WHERE event.tenant_id=$1 \
            AND event.run_id=$2 AND event.event_type='run.stop_requested' \
            AND event.detail->>'reason'='FAILED' AND event.detail->>'code'='RUN_DATA_LIMIT') \
              AS stop_seq, \
          (SELECT min(event.seq) FROM run_events event WHERE event.tenant_id=$1 \
            AND event.run_id=$2 AND event.event_type='step.completed' AND event.node='reserve' \
            AND event.instance_path_nodes=ARRAY['c10']::text[] \
            AND event.instance_path_indices=ARRAY[0]::integer[]) AS completion_seq, \
          (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
            AND event.node='reserve' AND event.instance_path_nodes=ARRAY['c10']::text[] \
            AND event.instance_path_indices=ARRAY[0]::integer[] \
            AND event.event_type IN ('step.failed','step.cancelled','step.retry_scheduled')) \
              AS other_forward_outcomes, \
          (SELECT count(*) FROM steps step WHERE step.tenant_id=$1 AND step.run_id=$2 \
            AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[] \
            AND step.node IN ('reserve~r','reserve~c')) AS derived_steps",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("under-limit report boundary order");
    let stop_seq = row.get::<i64, _>("stop_seq");
    let completion_seq = row.get::<i64, _>("completion_seq");
    assert!(
        stop_seq < completion_seq,
        "data-limit stop must precede the known success"
    );
    assert_eq!(row.get::<i64, _>("other_forward_outcomes"), 0);
    assert_eq!(row.get::<i64, _>("derived_steps"), 0);
}

async fn output_reference_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'runResult',encode(run.result_digest,'hex'), \
          'stepOutputs',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'pathNodes',step.instance_path_nodes,'pathIndices',step.instance_path_indices, \
              'node',step.node,'output',encode(step.output_digest,'hex')) \
            ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node),'[]'::jsonb) \
            FROM steps step WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id), \
          'scopeResults',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'owner',scope.owner_node,'result',encode(scope.result_digest,'hex')) \
            ORDER BY scope.owner_node),'[]'::jsonb) FROM iteration_scopes scope \
            WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id), \
          'memberResults',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'scope',member.scope_id,'index',member.iteration_index, \
              'result',encode(member.result_digest,'hex')) \
            ORDER BY member.scope_id,member.iteration_index),'[]'::jsonb) \
            FROM collection_iterations member \
            WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id), \
          'checkpoints',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'scope',checkpoint.scope_id,'index',checkpoint.iteration_index, \
              'result',encode(checkpoint.result_digest,'hex')) \
            ORDER BY checkpoint.scope_id,checkpoint.iteration_index),'[]'::jsonb) \
            FROM iteration_checkpoints checkpoint \
            WHERE checkpoint.tenant_id=run.tenant_id AND checkpoint.run_id=run.id), \
          'journalPayloads',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'sequence',event.seq,'digest',encode(event.payload_digest,'hex')) \
            ORDER BY event.seq),'[]'::jsonb) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
              AND event.payload_digest IS NOT NULL), \
          'blobs',(SELECT coalesce(jsonb_agg(to_jsonb(blob) ORDER BY blob.id),'[]'::jsonb) \
            FROM blobs blob WHERE blob.tenant_id=run.tenant_id AND blob.run_id=run.id)) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("bounded output-reference snapshot");
    sha(&value)
}

async fn retained_output_reference_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'runResults',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'run',result_run.id,'digest',encode(result_run.result_digest,'hex')) \
            ORDER BY result_run.id),'[]'::jsonb) FROM runs result_run \
            WHERE result_run.tenant_id=$1 AND result_run.id=$2 \
              AND result_run.result_digest IS NOT NULL), \
          'stepOutputs',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'pathNodes',step.instance_path_nodes,'pathIndices',step.instance_path_indices, \
              'node',step.node,'output',encode(step.output_digest,'hex')) \
            ORDER BY step.instance_path_nodes,step.instance_path_indices,step.node),'[]'::jsonb) \
            FROM steps step WHERE step.tenant_id=$1 AND step.run_id=$2 \
              AND step.output_digest IS NOT NULL), \
          'scopeResults',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'pathNodes',scope.owner_path_nodes,'pathIndices',scope.owner_path_indices, \
              'owner',scope.owner_node,'result',encode(scope.result_digest,'hex')) \
            ORDER BY scope.owner_path_nodes,scope.owner_path_indices,scope.owner_node),'[]'::jsonb) \
            FROM iteration_scopes scope WHERE scope.tenant_id=$1 AND scope.run_id=$2 \
              AND scope.result_digest IS NOT NULL), \
          'memberResults',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'scope',member.scope_id,'index',member.iteration_index, \
              'result',encode(member.result_digest,'hex')) \
            ORDER BY member.scope_id,member.iteration_index),'[]'::jsonb) \
            FROM collection_iterations member WHERE member.tenant_id=$1 AND member.run_id=$2 \
              AND member.result_digest IS NOT NULL), \
          'checkpoints',(SELECT coalesce(jsonb_agg(jsonb_build_object( \
              'scope',checkpoint.scope_id,'index',checkpoint.iteration_index, \
              'result',encode(checkpoint.result_digest,'hex')) \
            ORDER BY checkpoint.scope_id,checkpoint.iteration_index),'[]'::jsonb) \
            FROM iteration_checkpoints checkpoint \
            WHERE checkpoint.tenant_id=$1 AND checkpoint.run_id=$2 \
              AND checkpoint.result_digest IS NOT NULL))",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("nonnull retained output-reference snapshot");
    sha(&value)
}

async fn reservation_usage_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'member',(SELECT jsonb_build_object('reserved',member.reserved_instances, \
              'remaining',member.remaining_reservations,'slotHeld',member.slot_held) \
            FROM collection_iterations member JOIN iteration_scopes scope \
              USING(tenant_id,run_id,scope_id) \
            WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id \
              AND scope.owner_path_nodes='{}'::text[] \
              AND scope.owner_path_indices='{}'::integer[] \
              AND scope.owner_node='c10' AND member.iteration_index=0), \
          'runUsage',(SELECT to_jsonb(run_usage) FROM run_instance_usage run_usage \
            WHERE run_usage.tenant_id=run.tenant_id AND run_usage.run_id=run.id), \
          'tenantIterations',(SELECT to_jsonb(tenant_usage) FROM tenant_iteration_usage tenant_usage \
            WHERE tenant_usage.tenant_id=run.tenant_id)) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("live member reservation and iteration usage snapshot");
    sha(&value)
}

async fn public_cancellation_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'stop',jsonb_build_object('reason',run.stop_reason,'code',run.stop_code), \
          'stopEvent',(SELECT to_jsonb(stop_event) FROM run_events stop_event \
            WHERE stop_event.tenant_id=run.tenant_id AND stop_event.run_id=run.id \
              AND stop_event.event_type='run.stop_requested'), \
          'audit',(SELECT to_jsonb(cancel_audit) FROM audit_records cancel_audit \
            WHERE cancel_audit.tenant_id=run.tenant_id \
              AND cancel_audit.resource_type='run' AND cancel_audit.resource_id=run.id::text \
              AND cancel_audit.action='run.cancel_requested'), \
          'receipt',(SELECT to_jsonb(cancel_receipt) FROM command_dedupe cancel_receipt \
            WHERE cancel_receipt.tenant_id=run.tenant_id \
              AND cancel_receipt.scope=concat('run.operate:',run.id::text) \
              AND cancel_receipt.idempotency_key=$3)) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(CANCELLATION_KEY)
    .fetch_one(pool)
    .await
    .expect("immutable public cancellation identity");
    sha(&value)
}

async fn assert_cancellation_data_limit_audits(pool: &PgPool, receipt: &Receipt, expected: i64) {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND resource_type='run' \
          AND resource_id=$2 AND action='run.data_limit_exceeded'",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.to_string())
    .fetch_one(pool)
    .await
    .expect("complete cancellation-specific data-limit audit count");
    assert_eq!(
        count, expected,
        "cancellation gained an unexpected data-limit audit"
    );
}

async fn cancellation_drain_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'publicCancel',$3::text, \
          'scope',(SELECT jsonb_build_object( \
              'kind',scope.drain_kind,'iteration',scope.drain_iteration, \
              'code',scope.drain_failure_code,'sequence',scope.drain_sequence, \
              'outbox',scope.drain_outbox_id) FROM iteration_scopes scope \
            WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
              AND scope.owner_path_nodes='{}'::text[] \
              AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10'), \
          'drainEvent',(SELECT to_jsonb(drain_event) FROM run_events drain_event \
            WHERE drain_event.tenant_id=run.tenant_id AND drain_event.run_id=run.id \
              AND drain_event.event_type='collection.drain_started' \
              AND drain_event.node='c10'), \
          'cancelEvent',(SELECT to_jsonb(cancel_event) FROM run_events cancel_event \
            WHERE cancel_event.tenant_id=run.tenant_id AND cancel_event.run_id=run.id \
              AND cancel_event.event_type='step.cancel_requested' \
              AND cancel_event.node='reserve' \
              AND cancel_event.instance_path_nodes=ARRAY['c10']::text[] \
              AND cancel_event.instance_path_indices=ARRAY[0]::integer[]), \
          'outbox',(SELECT to_jsonb(drain_message) FROM event_outbox drain_message \
            WHERE drain_message.tenant_id=run.tenant_id \
              AND drain_message.aggregate_id=run.id \
              AND drain_message.event_type='collection.drain.started' \
              AND drain_message.routing_key='c10')) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(public_cancellation_fingerprint(pool, receipt).await)
    .fetch_one(pool)
    .await
    .expect("immutable cancellation drain identity");
    sha(&value)
}

async fn assert_public_cancellation(
    pool: &PgPool,
    receipt: &Receipt,
    expected_version: i64,
    outboxes_before: i64,
    payloads_before: i64,
    output_fingerprint: &str,
) -> (OperationReceipt, String) {
    let row = sqlx::query(
        "SELECT run.status AS run_status,run.stop_reason,run.stop_code,run.run_version, \
                telemetry.head_seq,scope.lifecycle,step.status AS step_status,step.cancel_requested, \
                (SELECT count(*) FROM run_events stop_event \
                  WHERE stop_event.tenant_id=$1 AND stop_event.run_id=$2 \
                    AND stop_event.event_type='run.stop_requested') AS stops, \
                COALESCE((SELECT bool_and(stop_event.detail->>'reason'='CANCELLED' \
                    AND stop_event.detail->>'code' IS NULL) FROM run_events stop_event \
                  WHERE stop_event.tenant_id=$1 AND stop_event.run_id=$2 \
                    AND stop_event.event_type='run.stop_requested'),false) AS stop_exact, \
                (SELECT count(*) FROM run_events drain_event \
                  WHERE drain_event.tenant_id=$1 AND drain_event.run_id=$2 \
                    AND drain_event.event_type='collection.drain_started' \
                    AND drain_event.node='c10') AS drains, \
                (SELECT count(*) FROM run_events cancel_event \
                  WHERE cancel_event.tenant_id=$1 AND cancel_event.run_id=$2 \
                    AND cancel_event.event_type='step.cancel_requested' \
                    AND cancel_event.node='reserve' \
                    AND cancel_event.instance_path_nodes=ARRAY['c10']::text[] \
                    AND cancel_event.instance_path_indices=ARRAY[0]::integer[]) AS cancellations, \
                (SELECT count(*) FROM audit_records cancel_audit \
                  WHERE cancel_audit.tenant_id=$1 AND cancel_audit.resource_type='run' \
                    AND cancel_audit.resource_id=$2::text \
                    AND cancel_audit.action='run.cancel_requested') AS cancel_audits, \
                COALESCE((SELECT bool_and(cancel_audit.actor='alice' \
                    AND cancel_audit.reason=$3) FROM audit_records cancel_audit \
                  WHERE cancel_audit.tenant_id=$1 AND cancel_audit.resource_type='run' \
                    AND cancel_audit.resource_id=$2::text \
                    AND cancel_audit.action='run.cancel_requested'),false) AS cancel_audit_exact, \
                COALESCE((SELECT bool_and(cancel_audit.trace_id ~ '^[0-9a-f]{32}$' \
                    AND cancel_audit.trace_id IS NOT DISTINCT FROM stop_event.trace_id) \
                  FROM audit_records cancel_audit JOIN run_events stop_event \
                    ON stop_event.tenant_id=cancel_audit.tenant_id \
                   AND stop_event.run_id=$2 AND stop_event.event_type='run.stop_requested' \
                  WHERE cancel_audit.tenant_id=$1 AND cancel_audit.resource_type='run' \
                    AND cancel_audit.resource_id=$2::text \
                    AND cancel_audit.action='run.cancel_requested'),false) AS http_trace, \
                (SELECT count(*) FROM audit_records limit_audit \
                  WHERE limit_audit.tenant_id=$1 AND limit_audit.resource_type='run' \
                    AND limit_audit.resource_id=$2::text \
                    AND limit_audit.action='run.data_limit_exceeded') AS data_limit_audits \
           FROM runs run JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id \
            AND scope.run_id=run.id AND scope.owner_path_nodes='{}'::text[] \
            AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10' \
           JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
            AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(CANCELLATION_REASON)
    .fetch_one(pool)
    .await
    .expect("public cancellation before scheduler drain");
    assert_eq!(row.get::<String, _>("run_status"), "CANCELLING");
    assert_eq!(
        row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("CANCELLED")
    );
    assert!(row.get::<Option<String>, _>("stop_code").is_none());
    assert_eq!(row.get::<i64, _>("run_version"), expected_version + 1);
    assert_eq!(
        row.get::<i64, _>("run_version"),
        row.get::<i64, _>("head_seq")
    );
    assert_eq!(row.get::<String, _>("lifecycle"), "OPEN");
    assert_eq!(row.get::<String, _>("step_status"), "LEASED");
    assert!(!row.get::<bool, _>("cancel_requested"));
    assert_eq!(row.get::<i64, _>("stops"), 1);
    assert!(row.get::<bool, _>("stop_exact"));
    assert_eq!(row.get::<i64, _>("drains"), 0);
    assert_eq!(row.get::<i64, _>("cancellations"), 0);
    assert_eq!(row.get::<i64, _>("cancel_audits"), 1);
    assert!(row.get::<bool, _>("cancel_audit_exact"));
    assert!(row.get::<bool, _>("http_trace"));
    assert_eq!(row.get::<i64, _>("data_limit_audits"), 0);
    assert_eq!(outboxes(pool, receipt).await, outboxes_before);
    assert_eq!(payloads(pool, receipt).await, payloads_before);
    assert_eq!(
        effective_totals(pool, receipt).await,
        receipt.preset.expected_totals()
    );
    assert!(
        output_reference_fingerprint(pool, receipt).await == output_fingerprint,
        "public cancellation changed output or blob references"
    );
    let operation_receipt = verified_operation_receipt(pool, receipt, expected_version).await;
    let fingerprint = public_cancellation_fingerprint(pool, receipt).await;
    (operation_receipt, fingerprint)
}

async fn stop_drain_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'stop',jsonb_build_object('reason',run.stop_reason,'code',run.stop_code), \
          'stopEvent',(SELECT to_jsonb(event) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
              AND event.event_type='run.stop_requested'), \
          'scope',(SELECT jsonb_build_object( \
              'kind',scope.drain_kind,'iteration',scope.drain_iteration, \
              'code',scope.drain_failure_code,'sequence',scope.drain_sequence, \
              'outbox',scope.drain_outbox_id) FROM iteration_scopes scope \
            WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
              AND scope.owner_path_nodes='{}'::text[] \
              AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10'), \
          'drainEvent',(SELECT to_jsonb(event) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
              AND event.event_type='collection.drain_started' AND event.node='c10'), \
          'cancelEvent',(SELECT to_jsonb(event) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
              AND event.event_type='step.cancel_requested' AND event.node='reserve' \
              AND event.instance_path_nodes=ARRAY['c10']::text[] \
              AND event.instance_path_indices=ARRAY[0]::integer[]), \
          'audit',(SELECT to_jsonb(audit) FROM audit_records audit \
            WHERE audit.tenant_id=run.tenant_id AND audit.resource_type='run' \
              AND audit.resource_id=run.id::text AND audit.action='run.data_limit_exceeded' \
              AND audit.reason='existing_outputs'), \
          'outbox',(SELECT to_jsonb(message) FROM event_outbox message \
            WHERE message.tenant_id=run.tenant_id AND message.aggregate_id=run.id \
              AND message.event_type='collection.drain.started' AND message.routing_key='c10')) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("immutable stop and drain identity");
    sha(&value)
}

async fn stopped_limit_drain_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'stop',jsonb_build_object('reason',run.stop_reason,'code',run.stop_code), \
          'stopEvent',(SELECT to_jsonb(event) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
              AND event.event_type='run.stop_requested'), \
          'scope',(SELECT jsonb_build_object( \
              'kind',scope.drain_kind,'iteration',scope.drain_iteration, \
              'code',scope.drain_failure_code,'sequence',scope.drain_sequence, \
              'outbox',scope.drain_outbox_id) FROM iteration_scopes scope \
            WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
              AND scope.owner_path_nodes='{}'::text[] \
              AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10'), \
          'drainEvent',(SELECT to_jsonb(event) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id \
              AND event.event_type='collection.drain_started' AND event.node='c10'), \
          'audit',(SELECT to_jsonb(audit) FROM audit_records audit \
            WHERE audit.tenant_id=run.tenant_id AND audit.resource_type='run' \
              AND audit.resource_id=run.id::text AND audit.action='run.data_limit_exceeded' \
              AND audit.reason='stopped_for_run_data_limit'), \
          'outbox',(SELECT to_jsonb(message) FROM event_outbox message \
            WHERE message.tenant_id=run.tenant_id AND message.aggregate_id=run.id \
              AND message.event_type='collection.drain.started' AND message.routing_key='c10')) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("immutable stopped-limit drain identity");
    sha(&value)
}

async fn drain_wait_fingerprint(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'run',jsonb_build_object('status',run.status,'reason',run.stop_reason, \
            'code',run.stop_code,'version',run.run_version), \
          'scope',(SELECT to_jsonb(scope) FROM iteration_scopes scope \
            WHERE scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
              AND scope.owner_path_nodes='{}'::text[] \
              AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10'), \
          'member',(SELECT to_jsonb(member) FROM collection_iterations member \
            JOIN iteration_scopes scope USING(tenant_id,run_id,scope_id) \
            WHERE member.tenant_id=run.tenant_id AND member.run_id=run.id \
              AND scope.owner_node='c10' AND member.iteration_index=0), \
          'step',(SELECT to_jsonb(step) FROM steps step \
            WHERE step.tenant_id=run.tenant_id AND step.run_id=run.id \
              AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
              AND step.instance_path_indices=ARRAY[0]::integer[]), \
          'events',(SELECT jsonb_agg(to_jsonb(event) ORDER BY event.seq) FROM run_events event \
            WHERE event.tenant_id=run.tenant_id AND event.run_id=run.id), \
          'sagas',(SELECT jsonb_agg(to_jsonb(saga)) FROM scope_sagas saga \
            WHERE saga.tenant_id=run.tenant_id AND saga.run_id=run.id), \
          'telemetry',(SELECT head.head_seq FROM telemetry_pending head \
            WHERE head.tenant_id=run.tenant_id AND head.run_id=run.id)) \
         FROM runs run WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("exact drain wait snapshot");
    sha(&value)
}

async fn assert_initial_drain_first_state(
    pool: &PgPool,
    receipt: &Receipt,
    output_fingerprint: &str,
) {
    let row = sqlx::query(
        "SELECT run.status,run.stop_reason,run.stop_code,run.result_digest IS NULL AS no_run_result, \
                scope.lifecycle,scope.item_count,scope.next_admission_index,scope.admitted_count, \
                scope.completed_count,scope.active_count,scope.result_digest IS NULL AS no_scope_result, \
                scope.drain_kind IS NULL AS no_drain_kind,scope.drain_iteration IS NULL AS no_drain_iteration, \
                scope.drain_sequence IS NULL AS no_drain_sequence, \
                member.lifecycle AS member_lifecycle,member.slot_held,member.reserved_instances, \
                member.remaining_reservations,step.status AS step_status,step.cancel_requested, \
                (SELECT count(*) FROM iteration_scopes old_scope JOIN steps owner \
                  ON owner.tenant_id=old_scope.tenant_id AND owner.run_id=old_scope.run_id \
                 AND owner.node=old_scope.owner_node AND owner.instance_path_nodes='{}'::text[] \
                 AND owner.instance_path_indices='{}'::integer[] \
                 WHERE old_scope.tenant_id=$1 AND old_scope.run_id=$2 \
                   AND old_scope.owner_node=ANY($3::text[]) AND old_scope.lifecycle='SUCCEEDED' \
                   AND owner.status='SUCCEEDED' AND owner.output_digest IS NULL) AS old_successes, \
                (SELECT count(*) FROM leaf_collection_success_certifications certificate \
                  WHERE certificate.tenant_id=$1 AND certificate.run_id=$2) AS certificates, \
                (SELECT count(*) FROM iteration_checkpoints checkpoint \
                  WHERE checkpoint.tenant_id=$1 AND checkpoint.run_id=$2 \
                    AND checkpoint.scope_id=scope.scope_id) AS checkpoints, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='step.completed' AND event.node='reserve' \
                  AND event.instance_path_nodes=ARRAY['c10']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[]) AS selected_completions, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='step.cancel_requested' AND event.node='reserve' \
                  AND event.instance_path_nodes=ARRAY['c10']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[]) AS cancellations, \
                (SELECT count(*) FROM scope_sagas saga WHERE saga.tenant_id=$1 AND saga.run_id=$2) AS sagas, \
                (SELECT count(*) FROM command_dedupe receipt_row WHERE receipt_row.tenant_id=$1 \
                  AND receipt_row.scope=$4 AND receipt_row.idempotency_key=$5) AS receipts \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
            AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[] \
            AND scope.owner_node='c10' \
           JOIN collection_iterations member ON member.tenant_id=scope.tenant_id \
            AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
            AND member.iteration_index=0 \
           JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
            AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(source_nodes())
    .bind(report_scope(receipt))
    .bind(&receipt.completion.idempotency_key)
    .fetch_one(pool)
    .await
    .expect("genuine pre-drain old state");
    let initial_status = row.get::<String, _>("status");
    assert!(
        matches!(initial_status.as_str(), "RUNNING" | "WAITING"),
        "the genuine old run must still be live before current scheduling"
    );
    assert!(row.get::<Option<String>, _>("stop_reason").is_none());
    assert!(row.get::<Option<String>, _>("stop_code").is_none());
    assert!(row.get::<bool, _>("no_run_result"));
    assert_eq!(row.get::<String, _>("lifecycle"), "OPEN");
    assert_eq!(row.get::<i32, _>("item_count"), 1);
    assert_eq!(row.get::<i32, _>("next_admission_index"), 1);
    assert_eq!(row.get::<i32, _>("admitted_count"), 1);
    assert_eq!(row.get::<i32, _>("completed_count"), 0);
    assert_eq!(row.get::<i32, _>("active_count"), 1);
    assert!(row.get::<bool, _>("no_scope_result"));
    assert!(row.get::<bool, _>("no_drain_kind"));
    assert!(row.get::<bool, _>("no_drain_iteration"));
    assert!(row.get::<bool, _>("no_drain_sequence"));
    assert_eq!(row.get::<String, _>("member_lifecycle"), "ADMITTED");
    assert!(row.get::<bool, _>("slot_held"));
    assert!(row.get::<i32, _>("reserved_instances") >= 1);
    assert!(row.get::<i32, _>("remaining_reservations") >= 1);
    assert_eq!(row.get::<String, _>("step_status"), "LEASED");
    assert!(!row.get::<bool, _>("cancel_requested"));
    assert_eq!(row.get::<i64, _>("old_successes"), 10);
    assert_eq!(row.get::<i64, _>("certificates"), 0);
    assert_eq!(row.get::<i64, _>("checkpoints"), 0);
    assert_eq!(row.get::<i64, _>("selected_completions"), 0);
    assert_eq!(row.get::<i64, _>("cancellations"), 0);
    assert_eq!(row.get::<i64, _>("sagas"), 0);
    assert_eq!(row.get::<i64, _>("receipts"), 0);
    assert!(
        output_reference_fingerprint(pool, receipt).await == output_fingerprint,
        "the initial current read changed output or blob references"
    );
}

async fn assert_first_drain(
    pool: &PgPool,
    receipt: &Receipt,
    outboxes_before: i64,
    payloads_before: i64,
    output_fingerprint: &str,
) -> String {
    let row = sqlx::query(
        "SELECT run.status,run.stop_reason,run.stop_code,run.result_digest IS NULL AS no_run_result, \
                run.run_version,telemetry.head_seq,scope.lifecycle,scope.drain_kind, \
                scope.drain_iteration,scope.drain_failure_code,scope.drain_sequence, \
                scope.result_digest IS NULL AS no_scope_result,member.lifecycle AS member_lifecycle, \
                member.slot_held,step.status AS step_status,step.cancel_requested, \
                step.step_id=$3 AS same_step,step.lease_token_hash=$4 AS same_token, \
                step.leased_by=$5 AS same_worker,step.attempt=$6 AS same_attempt, \
                step.generation=$7 AS same_generation,step.lease_expires_at>clock_timestamp() AS live_lease, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='run.stop_requested' AND event.detail->>'reason'='FAILED' \
                  AND event.detail->>'code'='RUN_DATA_LIMIT') AS stops, \
                (SELECT min(event.seq) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='run.stop_requested') AS stop_seq, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='collection.drain_started' AND event.node='c10' \
                  AND event.detail->>'reason'='run_data_limit') AS drains, \
                (SELECT min(event.seq) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='collection.drain_started' AND event.node='c10') AS drain_seq, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='step.cancel_requested' AND event.node='reserve' \
                  AND event.instance_path_nodes=ARRAY['c10']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[]) AS cancellations, \
                (SELECT min(event.seq) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='step.cancel_requested' AND event.node='reserve' \
                  AND event.instance_path_nodes=ARRAY['c10']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[]) AS cancel_seq, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND ((event.event_type IN ('step.completed','step.failed','step.cancelled', \
                                             'step.retry_scheduled') \
                       AND event.node='reserve' AND event.instance_path_nodes=ARRAY['c10']::text[] \
                       AND event.instance_path_indices=ARRAY[0]::integer[]) \
                    OR (event.event_type IN ('collection.iteration_returned', \
                                             'collection.iteration_checkpointed', \
                                             'collection.iteration_drained','collection.finished') \
                       AND event.node='c10') OR event.event_type='run.finished')) AS premature_events, \
                (SELECT count(*) FROM scope_sagas saga WHERE saga.tenant_id=$1 AND saga.run_id=$2) AS sagas, \
                (SELECT count(*) FROM steps extra WHERE extra.tenant_id=$1 AND extra.run_id=$2 \
                  AND extra.instance_path_nodes=ARRAY['c10']::text[] \
                  AND extra.instance_path_indices=ARRAY[0]::integer[] \
                  AND (extra.node='reserve~c' OR extra.node='reserve~r')) AS extra_effects, \
                (SELECT count(*) FROM audit_records audit WHERE audit.tenant_id=$1 \
                  AND audit.resource_type='run' AND audit.resource_id=$2::text \
                  AND audit.action='run.data_limit_exceeded' AND audit.reason='existing_outputs' \
                  AND audit.actor='system:engine' AND audit.trace_id IS NOT DISTINCT FROM run.trace_id) AS audits, \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=$1 \
                  AND message.aggregate_id=$2 AND message.event_type='collection.drain.started' \
                  AND message.routing_key='c10' AND message.payload->>'reason'='FAILED') AS drain_outboxes, \
                (SELECT count(*) FROM command_dedupe receipt_row WHERE receipt_row.tenant_id=$1 \
                  AND receipt_row.scope=$8 AND receipt_row.idempotency_key=$9) AS receipts \
           FROM runs run JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
            AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[] \
            AND scope.owner_node='c10' \
           JOIN collection_iterations member ON member.tenant_id=scope.tenant_id \
            AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
            AND member.iteration_index=0 \
           JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
            AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(Uuid::parse_str(&receipt.lease.step_id).expect("step id"))
    .bind(Token::hash_of(&receipt.lease.lease_token))
    .bind(&receipt.worker_id)
    .bind(receipt.lease.attempt)
    .bind(receipt.lease.generation)
    .bind(report_scope(receipt))
    .bind(&receipt.completion.idempotency_key)
    .fetch_one(pool)
    .await
    .expect("first normal drain state");
    assert_eq!(row.get::<String, _>("status"), "CANCELLING");
    assert_eq!(
        row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("FAILED")
    );
    assert_eq!(
        row.get::<Option<String>, _>("stop_code").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<bool, _>("no_run_result"));
    assert_eq!(
        row.get::<i64, _>("run_version"),
        row.get::<i64, _>("head_seq")
    );
    assert_eq!(row.get::<String, _>("lifecycle"), "DRAINING");
    assert_eq!(
        row.get::<Option<String>, _>("drain_kind").as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<Option<i32>, _>("drain_iteration").is_none());
    assert_eq!(
        row.get::<Option<String>, _>("drain_failure_code")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert!(row.get::<bool, _>("no_scope_result"));
    assert_eq!(row.get::<String, _>("member_lifecycle"), "ADMITTED");
    assert!(row.get::<bool, _>("slot_held"));
    assert_eq!(row.get::<String, _>("step_status"), "LEASED");
    assert!(row.get::<bool, _>("cancel_requested"));
    assert!(row.get::<bool, _>("same_step"));
    assert!(row.get::<bool, _>("same_token"));
    assert!(row.get::<bool, _>("same_worker"));
    assert!(row.get::<bool, _>("same_attempt"));
    assert!(row.get::<bool, _>("same_generation"));
    assert!(row.get::<bool, _>("live_lease"));
    assert_eq!(row.get::<i64, _>("stops"), 1);
    assert_eq!(row.get::<i64, _>("drains"), 1);
    assert_eq!(row.get::<i64, _>("cancellations"), 1);
    let stop_seq = row.get::<i64, _>("stop_seq");
    let drain_seq = row.get::<i64, _>("drain_seq");
    let cancel_seq = row.get::<i64, _>("cancel_seq");
    assert!(
        stop_seq < drain_seq && drain_seq < cancel_seq,
        "stop, drain and cancellation order changed"
    );
    assert_eq!(
        row.get::<Option<i64>, _>("drain_sequence"),
        Some(cancel_seq)
    );
    assert_eq!(row.get::<i64, _>("run_version"), cancel_seq);
    assert_eq!(row.get::<i64, _>("premature_events"), 0);
    assert_eq!(row.get::<i64, _>("sagas"), 0);
    assert_eq!(row.get::<i64, _>("extra_effects"), 0);
    assert_eq!(row.get::<i64, _>("audits"), 1);
    assert_eq!(row.get::<i64, _>("drain_outboxes"), 1);
    assert_eq!(row.get::<i64, _>("receipts"), 0);
    assert_eq!(outboxes(pool, receipt).await, outboxes_before + 1);
    assert_eq!(payloads(pool, receipt).await, payloads_before);
    assert_eq!(
        effective_totals(pool, receipt).await,
        (2_100_000, 2_100_020, 4_200_020)
    );
    assert!(
        output_reference_fingerprint(pool, receipt).await == output_fingerprint,
        "the drain retained or rewired an output or blob"
    );
    assert!(
        source_witness(pool, receipt.tenant, receipt.run).await == receipt.source_witness,
        "the drain changed the immutable old source"
    );
    stop_drain_fingerprint(pool, receipt).await
}

async fn assert_cancellation_first_drain(
    pool: &PgPool,
    receipt: &Receipt,
    outboxes_before: i64,
    payloads_before: i64,
    output_fingerprint: &str,
) -> String {
    let row = sqlx::query(
        "SELECT run.status AS run_status,run.stop_reason,run.stop_code,run.run_version, \
                telemetry.head_seq,scope.lifecycle,scope.drain_kind,scope.drain_iteration, \
                scope.drain_failure_code,scope.drain_sequence, \
                member.lifecycle AS member_lifecycle,member.slot_held, \
                step.status AS step_status,step.cancel_requested, \
                step.step_id=$3 AS same_step,step.lease_token_hash=$4 AS same_token, \
                step.leased_by=$5 AS same_worker,step.attempt=$6 AS same_attempt, \
                step.generation=$7 AS same_generation, \
                step.lease_expires_at>clock_timestamp() AS live_lease, \
                (SELECT count(*) FROM run_events stop_event \
                  WHERE stop_event.tenant_id=$1 AND stop_event.run_id=$2 \
                    AND stop_event.event_type='run.stop_requested') AS stops, \
                COALESCE((SELECT bool_and(stop_event.detail->>'reason'='CANCELLED' \
                    AND stop_event.detail->>'code' IS NULL) FROM run_events stop_event \
                  WHERE stop_event.tenant_id=$1 AND stop_event.run_id=$2 \
                    AND stop_event.event_type='run.stop_requested'),false) AS stop_exact, \
                (SELECT min(stop_event.seq) FROM run_events stop_event \
                  WHERE stop_event.tenant_id=$1 AND stop_event.run_id=$2 \
                    AND stop_event.event_type='run.stop_requested') AS stop_seq, \
                (SELECT count(*) FROM run_events drain_event \
                  WHERE drain_event.tenant_id=$1 AND drain_event.run_id=$2 \
                    AND drain_event.event_type='collection.drain_started' \
                    AND drain_event.node='c10' \
                    AND drain_event.detail->>'reason'='cancelled') AS drains, \
                (SELECT min(drain_event.seq) FROM run_events drain_event \
                  WHERE drain_event.tenant_id=$1 AND drain_event.run_id=$2 \
                    AND drain_event.event_type='collection.drain_started' \
                    AND drain_event.node='c10') AS drain_seq, \
                (SELECT count(*) FROM run_events cancel_event \
                  WHERE cancel_event.tenant_id=$1 AND cancel_event.run_id=$2 \
                    AND cancel_event.event_type='step.cancel_requested' \
                    AND cancel_event.node='reserve' \
                    AND cancel_event.instance_path_nodes=ARRAY['c10']::text[] \
                    AND cancel_event.instance_path_indices=ARRAY[0]::integer[]) AS cancellations, \
                (SELECT min(cancel_event.seq) FROM run_events cancel_event \
                  WHERE cancel_event.tenant_id=$1 AND cancel_event.run_id=$2 \
                    AND cancel_event.event_type='step.cancel_requested' \
                    AND cancel_event.node='reserve' \
                    AND cancel_event.instance_path_nodes=ARRAY['c10']::text[] \
                    AND cancel_event.instance_path_indices=ARRAY[0]::integer[]) AS cancel_seq, \
                (SELECT count(*) FROM run_events premature_event \
                  WHERE premature_event.tenant_id=$1 AND premature_event.run_id=$2 \
                    AND ((premature_event.event_type IN ('step.completed','step.failed', \
                          'step.cancelled','step.retry_scheduled') \
                         AND premature_event.node='reserve' \
                         AND premature_event.instance_path_nodes=ARRAY['c10']::text[] \
                         AND premature_event.instance_path_indices=ARRAY[0]::integer[]) \
                      OR (premature_event.event_type IN ('collection.iteration_returned', \
                          'collection.iteration_checkpointed','collection.iteration_drained', \
                          'collection.finished') AND premature_event.node='c10') \
                      OR premature_event.event_type='run.finished')) AS premature_events, \
                (SELECT count(*) FROM scope_sagas saga \
                  WHERE saga.tenant_id=$1 AND saga.run_id=$2) AS sagas, \
                (SELECT count(*) FROM steps extra \
                  WHERE extra.tenant_id=$1 AND extra.run_id=$2 \
                    AND extra.instance_path_nodes=ARRAY['c10']::text[] \
                    AND extra.instance_path_indices=ARRAY[0]::integer[] \
                    AND (extra.node='reserve~c' OR extra.node='reserve~r')) AS extra_effects, \
                (SELECT count(*) FROM audit_records limit_audit \
                  WHERE limit_audit.tenant_id=$1 AND limit_audit.resource_type='run' \
                    AND limit_audit.resource_id=$2::text \
                    AND limit_audit.action='run.data_limit_exceeded') AS data_limit_audits, \
                (SELECT count(*) FROM event_outbox drain_message \
                  WHERE drain_message.tenant_id=$1 AND drain_message.aggregate_id=$2 \
                    AND drain_message.event_type='collection.drain.started' \
                    AND drain_message.routing_key='c10' \
                    AND drain_message.payload->>'reason'='CANCELLED') AS drain_outboxes, \
                (SELECT count(*) FROM command_dedupe report_receipt \
                  WHERE report_receipt.tenant_id=$1 AND report_receipt.scope=$8 \
                    AND report_receipt.idempotency_key=$9) AS report_receipts, \
                (SELECT count(*) FROM command_dedupe cancel_receipt \
                  WHERE cancel_receipt.tenant_id=$1 AND cancel_receipt.scope=$10 \
                    AND cancel_receipt.idempotency_key=$11) AS cancel_receipts \
           FROM runs run JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id \
            AND scope.run_id=run.id AND scope.owner_path_nodes='{}'::text[] \
            AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10' \
           JOIN collection_iterations member ON member.tenant_id=scope.tenant_id \
            AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
            AND member.iteration_index=0 \
           JOIN steps step ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
            AND step.node='reserve' AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(Uuid::parse_str(&receipt.lease.step_id).expect("step id"))
    .bind(Token::hash_of(&receipt.lease.lease_token))
    .bind(&receipt.worker_id)
    .bind(receipt.lease.attempt)
    .bind(receipt.lease.generation)
    .bind(report_scope(receipt))
    .bind(&receipt.completion.idempotency_key)
    .bind(operation_scope(receipt))
    .bind(CANCELLATION_KEY)
    .fetch_one(pool)
    .await
    .expect("first normal cancellation drain state");
    assert_eq!(row.get::<String, _>("run_status"), "CANCELLING");
    assert_eq!(
        row.get::<Option<String>, _>("stop_reason").as_deref(),
        Some("CANCELLED")
    );
    assert!(row.get::<Option<String>, _>("stop_code").is_none());
    assert_eq!(
        row.get::<i64, _>("run_version"),
        row.get::<i64, _>("head_seq")
    );
    assert_eq!(row.get::<String, _>("lifecycle"), "DRAINING");
    assert_eq!(
        row.get::<Option<String>, _>("drain_kind").as_deref(),
        Some("CANCELLED")
    );
    assert!(row.get::<Option<i32>, _>("drain_iteration").is_none());
    assert!(row.get::<Option<String>, _>("drain_failure_code").is_none());
    assert_eq!(row.get::<String, _>("member_lifecycle"), "ADMITTED");
    assert!(row.get::<bool, _>("slot_held"));
    assert_eq!(row.get::<String, _>("step_status"), "LEASED");
    assert!(row.get::<bool, _>("cancel_requested"));
    assert!(row.get::<bool, _>("same_step"));
    assert!(row.get::<bool, _>("same_token"));
    assert!(row.get::<bool, _>("same_worker"));
    assert!(row.get::<bool, _>("same_attempt"));
    assert!(row.get::<bool, _>("same_generation"));
    assert!(row.get::<bool, _>("live_lease"));
    assert_eq!(row.get::<i64, _>("stops"), 1);
    assert!(row.get::<bool, _>("stop_exact"));
    assert_eq!(row.get::<i64, _>("drains"), 1);
    assert_eq!(row.get::<i64, _>("cancellations"), 1);
    let stop_seq = row.get::<i64, _>("stop_seq");
    let drain_seq = row.get::<i64, _>("drain_seq");
    let cancel_seq = row.get::<i64, _>("cancel_seq");
    assert!(
        stop_seq < drain_seq && drain_seq < cancel_seq,
        "public stop, cancellation drain and lease cancellation order changed"
    );
    assert_eq!(
        row.get::<Option<i64>, _>("drain_sequence"),
        Some(cancel_seq)
    );
    assert_eq!(row.get::<i64, _>("run_version"), cancel_seq);
    assert_eq!(row.get::<i64, _>("premature_events"), 0);
    assert_eq!(row.get::<i64, _>("sagas"), 0);
    assert_eq!(row.get::<i64, _>("extra_effects"), 0);
    assert_eq!(row.get::<i64, _>("data_limit_audits"), 0);
    assert_eq!(row.get::<i64, _>("drain_outboxes"), 1);
    assert_eq!(row.get::<i64, _>("report_receipts"), 0);
    assert_eq!(row.get::<i64, _>("cancel_receipts"), 1);
    assert_eq!(outboxes(pool, receipt).await, outboxes_before + 1);
    assert_eq!(payloads(pool, receipt).await, payloads_before);
    assert_eq!(
        effective_totals(pool, receipt).await,
        receipt.preset.expected_totals()
    );
    assert!(
        output_reference_fingerprint(pool, receipt).await == output_fingerprint,
        "cancellation drain retained or rewired an output or blob"
    );
    assert!(
        source_witness(pool, receipt.tenant, receipt.run).await == receipt.source_witness,
        "cancellation drain changed the immutable old source"
    );
    cancellation_drain_fingerprint(pool, receipt).await
}

async fn assert_pending_compensation(
    pool: &PgPool,
    receipt: &Receipt,
    expected_cause: &str,
) -> String {
    let row = sqlx::query(
        "SELECT run.status,run.finished_at IS NULL AS unfinished,scope.lifecycle AS scope_lifecycle,member.lifecycle AS member_lifecycle, \
                saga.phase,saga.cause_code,saga.outcome,compensation.status AS compensation_status, \
                compensation.lease_expires_at>clock_timestamp() AS compensation_live, \
                (SELECT count(*) FROM scope_saga_entries entry WHERE entry.tenant_id=$1 \
                  AND entry.run_id=$2 AND entry.owner_path_nodes='{}'::text[] \
                  AND entry.owner_path_indices='{}'::integer[] AND entry.owner_node='c10' \
                  AND entry.iteration_index=0 AND entry.node='reserve' \
                  AND entry.kind='COMPENSATE') AS entries, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='collection.iteration_drained' AND event.node='c10') AS drains, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND (event.event_type='run.finished' OR (event.event_type='collection.finished' \
                    AND event.node='c10' AND event.instance_path_nodes='{}'::text[] \
                    AND event.instance_path_indices='{}'::integer[]))) AS terminal_events, \
                (SELECT count(*) FROM steps extra WHERE extra.tenant_id=$1 AND extra.run_id=$2 \
                  AND extra.node='reserve~r' AND extra.instance_path_nodes=ARRAY['c10']::text[] \
                  AND extra.instance_path_indices=ARRAY[0]::integer[]) AS reconciliations \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
            AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[] \
            AND scope.owner_node='c10' \
           JOIN collection_iterations member ON member.tenant_id=scope.tenant_id \
            AND member.run_id=scope.run_id AND member.scope_id=scope.scope_id \
            AND member.iteration_index=0 \
           JOIN scope_sagas saga ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
            AND saga.owner_path_nodes='{}'::text[] AND saga.owner_path_indices='{}'::integer[] \
            AND saga.owner_node='c10' \
           JOIN steps compensation ON compensation.tenant_id=run.tenant_id \
            AND compensation.run_id=run.id AND compensation.node='reserve~c' \
            AND compensation.instance_path_nodes=ARRAY['c10']::text[] \
            AND compensation.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("leased compensation remains an open obligation");
    assert_eq!(row.get::<String, _>("status"), "COMPENSATING");
    assert!(row.get::<bool, _>("unfinished"));
    assert_eq!(row.get::<String, _>("scope_lifecycle"), "DRAINING");
    assert_eq!(row.get::<String, _>("member_lifecycle"), "DRAINED");
    assert_eq!(row.get::<String, _>("phase"), "COMPENSATING");
    assert_eq!(row.get::<String, _>("cause_code"), expected_cause);
    assert!(row.get::<Option<String>, _>("outcome").is_none());
    assert_eq!(row.get::<String, _>("compensation_status"), "LEASED");
    assert!(row.get::<bool, _>("compensation_live"));
    assert_eq!(row.get::<i64, _>("entries"), 1);
    assert_eq!(row.get::<i64, _>("drains"), 1);
    assert_eq!(row.get::<i64, _>("terminal_events"), 0);
    assert_eq!(row.get::<i64, _>("reconciliations"), 0);
    let value: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'run',(SELECT jsonb_build_object('status',status,'version',run_version,'finished',finished_at) \
            FROM runs WHERE tenant_id=$1 AND id=$2), \
          'scope',(SELECT to_jsonb(scope) FROM iteration_scopes scope WHERE scope.tenant_id=$1 \
            AND scope.run_id=$2 AND scope.owner_node='c10'), \
          'member',(SELECT to_jsonb(member) FROM collection_iterations member \
            JOIN iteration_scopes scope USING(tenant_id,run_id,scope_id) \
            WHERE member.tenant_id=$1 AND member.run_id=$2 AND scope.owner_node='c10'), \
          'saga',(SELECT to_jsonb(saga) FROM scope_sagas saga WHERE saga.tenant_id=$1 \
            AND saga.run_id=$2 AND saga.owner_node='c10'), \
          'entries',(SELECT jsonb_agg(to_jsonb(entry)) FROM scope_saga_entries entry \
            WHERE entry.tenant_id=$1 AND entry.run_id=$2 AND entry.owner_node='c10'), \
          'compensation',(SELECT to_jsonb(step) FROM steps step WHERE step.tenant_id=$1 \
            AND step.run_id=$2 AND step.node='reserve~c' \
            AND step.instance_path_nodes=ARRAY['c10']::text[] \
            AND step.instance_path_indices=ARRAY[0]::integer[]))",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("pending compensation identity");
    sha(&value)
}

async fn under_limit_execution_footprint(pool: &PgPool, receipt: &Receipt) -> String {
    let audit_and_receipts: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'audits',(SELECT coalesce(jsonb_agg(to_jsonb(audit) ORDER BY audit.created_at,audit.id), \
            '[]'::jsonb) FROM audit_records audit WHERE audit.tenant_id=$1 \
              AND audit.resource_type='run' AND audit.resource_id=$2::text), \
          'receipts',(SELECT coalesce(jsonb_agg(to_jsonb(receipt_row) \
            ORDER BY receipt_row.scope,receipt_row.idempotency_key),'[]'::jsonb) \
            FROM command_dedupe receipt_row WHERE receipt_row.tenant_id=$1 \
              AND (receipt_row.scope=concat('run.operate:',$2::text) \
                OR receipt_row.scope LIKE concat('step:',$2::text,':%'))), \
          'payloadCount',(SELECT count(*) FROM payloads payload WHERE payload.tenant_id=$1))",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("safe execution audit and receipt footprint");
    let continuation_rows: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object( \
          'telemetry',(SELECT to_jsonb(telemetry) FROM telemetry_pending telemetry \
            WHERE telemetry.tenant_id=$1 AND telemetry.run_id=$2), \
          'sagas',(SELECT jsonb_agg(to_jsonb(saga) \
            ORDER BY saga.owner_path_nodes,saga.owner_path_indices,saga.owner_node) \
            FROM scope_sagas saga WHERE saga.tenant_id=$1 AND saga.run_id=$2), \
          'sagaEntries',(SELECT jsonb_agg(to_jsonb(entry) \
            ORDER BY entry.owner_path_nodes,entry.owner_path_indices,entry.owner_node, \
                     entry.iteration_index,entry.node) \
            FROM scope_saga_entries entry WHERE entry.tenant_id=$1 AND entry.run_id=$2))",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("exact telemetry and scope saga footprint");
    sha(&json!({
        "projection": projection_witness(pool, receipt.tenant, receipt.run).await,
        "outputs": output_reference_fingerprint(pool, receipt).await,
        "reservations": reservation_usage_fingerprint(pool, receipt).await,
        "auditAndReceipts": audit_and_receipts,
        "continuations": continuation_rows,
    }))
}

async fn assert_under_limit_full_overlay(api: &TestApi, receipt: &Receipt) {
    let before = under_limit_execution_footprint(&api.db.pool, receipt).await;
    let source_before = source_witness(&api.db.pool, receipt.tenant, receipt.run).await;
    let mut tx = api
        .executions
        .begin(receipt.tenant, receipt.run, Lock::Wait)
        .await
        .expect("open current production full run transaction")
        .expect("retained legacy run");
    assert_eq!(
        tx.logical_output_bytes()
            .await
            .expect("production effective output scalar"),
        4_000_020
    );
    let (root_aggregates, member_outputs, output_sum) = {
        let data = tx
            .data()
            .await
            .expect("bounded production full data overlay");
        let root_aggregates = data
            .root_outputs()
            .filter(|(node, output)| {
                node.as_str()
                    .strip_prefix('c')
                    .and_then(|suffix| suffix.parse::<u8>().ok())
                    .is_some_and(|index| index < 10)
                    && output.len() == 200_002
            })
            .count();
        let member_outputs = data
            .outputs
            .iter()
            .filter(|(instance, output)| {
                instance.path().depth() == 1
                    && instance.node().as_str() == "work"
                    && output.len() == 200_000
            })
            .count();
        let output_sum = data.outputs.values().map(Vec::len).sum::<usize>();
        (root_aggregates, member_outputs, output_sum)
    };
    assert_eq!(root_aggregates, 10);
    assert_eq!(member_outputs, 10);
    assert_eq!(output_sum, 4_000_020);
    tx.commit().await.expect("close read-only full overlay");
    assert!(
        source_witness(&api.db.pool, receipt.tenant, receipt.run).await == source_before,
        "full overlay changed the immutable source witness"
    );
    assert!(
        under_limit_execution_footprint(&api.db.pool, receipt).await == before,
        "full overlay backfilled or changed durable execution state"
    );
}

async fn fresh_scheduler(
    api: &TestApi,
    receipt: &Receipt,
) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    let url = if crate::support::as_runtime_role() {
        api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
    } else {
        api.db.url.clone()
    };
    let pool = connect(&PoolConfig::new(url))
        .await
        .expect("fresh scheduler pool");
    let store = PgRunStore::new(pool, DefinitionCache::new(Arc::new(JsonIrCodec), 8))
        .with_scoped_worker_protocol();
    let placement = Placement::new(receipt.scheduler_owner.clone());
    let round = Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    )
    .round()
    .await
    .expect("renew scheduler authority through fresh store");
    assert_eq!(round.replicas, 1);
    assert_eq!(round.held, usize::from(SHARDS));
    assert!(placement.owned().contains(Shard::of(receipt.run.as_uuid())));
    Scheduler::new(
        store,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement)
}

async fn reject_existing_outputs_while_under_limit(api: &TestApi, receipt: &Receipt) {
    let row = sqlx::query(
        "SELECT run.namespace_id,run.run_version,run.trace_id,run.status, \
                run.stop_reason,run.stop_code,scope.scope_id,scope.lifecycle, \
                scope.drain_kind,shard.owner,shard.generation \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
           JOIN scheduler_shards shard ON shard.shard=$3 \
          WHERE run.tenant_id=$1 AND run.id=$2 \
            AND scope.owner_path_nodes='{}'::text[] \
            AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10'",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(i16::from(Shard::of(receipt.run.as_uuid()).index()))
    .fetch_one(&api.db.pool)
    .await
    .expect("exact current collection and scheduler authority");
    assert_eq!(row.get::<Uuid, _>("namespace_id"), receipt.namespace);
    assert_eq!(row.get::<String, _>("status"), "RUNNING");
    assert!(row.get::<Option<String>, _>("stop_reason").is_none());
    assert!(row.get::<Option<String>, _>("stop_code").is_none());
    assert_eq!(row.get::<String, _>("lifecycle"), "OPEN");
    assert!(row.get::<Option<String>, _>("drain_kind").is_none());
    assert_eq!(row.get::<String, _>("owner"), receipt.scheduler_owner);
    let version = u64::try_from(row.get::<i64, _>("run_version")).expect("run version");
    let generation = u64::try_from(row.get::<i64, _>("generation")).expect("scheduler generation");
    let trace = row.get::<Option<String>, _>("trace_id");
    let evidence = CollectionDataLimitEvidence::existing_outputs();
    let occurred_at = time::OffsetDateTime::now_utc();
    let audit = AuditRecord::engine(
        receipt.tenant,
        AuditAction::RunDataLimitExceeded,
        ResourceType::Run,
        receipt.run.to_string(),
        evidence.required_audit_reason().as_str(),
        trace.clone(),
        occurred_at,
    );
    let command = CollectionDrain::for_data_limit(
        CollectionTransition::new(
            receipt.tenant,
            NamespaceId::new(receipt.namespace),
            receipt.run,
            version,
            ExecutionInstanceId::root(NodeId::parse("c10").expect("selected owner")),
            SchedulerAuthority::for_run(receipt.scheduler_owner.clone(), receipt.run, generation),
            Subject::engine(),
            trace,
            occurred_at,
            Some(audit),
        ),
        CollectionScopeId::new(row.get("scope_id")),
        evidence,
        Uuid::now_v7(),
    )
    .expect("checked ExistingOutputs command");
    let before = under_limit_execution_footprint(&api.db.pool, receipt).await;
    let store = PgCollectionStore::with_definition_cache(
        api.api_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    );
    assert!(matches!(
        store.start_collection_drain(&command).await,
        Err(PrimitiveStoreError::InvalidTransition)
    ));
    assert!(
        under_limit_execution_footprint(&api.db.pool, receipt).await == before,
        "rejected ExistingOutputs changed execution, audit, receipt or output state"
    );
}

async fn assert_under_limit_pre_report_controls(
    api: &TestApi,
    receipt: &Receipt,
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
) {
    assert_eq!(
        effective_totals(&api.db.pool, receipt).await,
        (2_000_000, 2_000_020, 4_000_020)
    );
    assert_under_limit_full_overlay(api, receipt).await;
    let candidates = api
        .executions
        .collection_candidates(&Window::everything(8))
        .await
        .expect("pre-report collection candidates");
    assert!(
        !candidates.iter().any(|candidate| {
            candidate.run_id == receipt.run && candidate.node.as_str() == "c10"
        })
    );
    let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("selected owner"));
    let before = under_limit_execution_footprint(&api.db.pool, receipt).await;
    assert_eq!(
        scheduler
            .advance_collection(receipt.tenant, receipt.run, &owner)
            .await
            .expect("normal pre-report collection advance"),
        CollectionAdvance::Waiting
    );
    let tick = scheduler.tick().await;
    assert_eq!(tick.failed, 0, "pre-report scheduler tick: {tick:?}");
    assert_eq!(
        tick.collections_advanced, 0,
        "pre-report scheduler tick: {tick:?}"
    );
    assert!(
        under_limit_execution_footprint(&api.db.pool, receipt).await == before,
        "pre-report scheduling changed execution or reservation state"
    );
    reject_existing_outputs_while_under_limit(api, receipt).await;
    assert_live_target(&api.db.pool, receipt, false).await;
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
    let verified = store.verify_run(receipt.tenant, receipt.run).await;
    assert!(verified.is_ok(), "cold journal projection replay failed");
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
    expected_status: &str,
) {
    for _ in 0..40 {
        let status: String =
            sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(receipt.tenant.as_uuid())
                .bind(receipt.run.as_uuid())
                .fetch_one(&api.db.pool)
                .await
                .expect("run status");
        if status == expected_status {
            return;
        }
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "final scheduler pass: {report:?}");
    }
    panic!("the run did not reach its real terminal outcome after compensation");
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReportDelivery {
    Direct,
    FrontierCas,
}

async fn run_report_before_drain_case(preset: FixturePreset) {
    run_report_before_drain_delivery(preset, ReportDelivery::Direct).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the separate immutable-old ordinary producer; see fixture README"]
async fn old_original_success_forces_frontier_cas_then_fresh_stopped_drain() {
    run_report_before_drain_delivery(FixturePreset::Ordinary, ReportDelivery::FrontierCas).await;
}

async fn run_report_before_drain_delivery(preset: FixturePreset, delivery: ReportDelivery) {
    let cas = delivery == ReportDelivery::FrontierCas;
    assert!(
        !cas || preset == FixturePreset::Ordinary,
        "CAS uses unchanged ordinary byte preset"
    );
    if cas {
        let _ = sagat_telemetry::Metrics::global();
    }
    let strong = preset == FixturePreset::UnderLimitIncrement || cas;
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
    let receipt = Arc::new(Receipt::parse_for(
        &std::fs::read(&receipt_path).expect("read private receipt"),
        preset,
    ));
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
    let mut options = current_options();
    if cas {
        options.statement_timeout = Duration::from_secs(30);
    }
    let api = start_on_db(db, options).await;
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
    assert_live_target(&platform.api.db.pool, &receipt, false).await;
    assert_eq!(
        effective_totals(&platform.api.db.pool, &receipt).await,
        preset.expected_totals()
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

    let pre_report_scheduler = if preset == FixturePreset::UnderLimitIncrement {
        let scheduler = fresh_scheduler(&platform.api, &receipt).await;
        assert_under_limit_pre_report_controls(&platform.api, &receipt, &scheduler).await;
        Some(scheduler)
    } else {
        None
    };
    let output_fingerprint_before_report =
        output_reference_fingerprint(&platform.api.db.pool, &receipt).await;
    let retained_output_references_before_report =
        retained_output_reference_fingerprint(&platform.api.db.pool, &receipt).await;
    let footprint_before_report =
        under_limit_execution_footprint(&platform.api.db.pool, &receipt).await;

    let reservations_before_report =
        reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await;
    let outboxes_before = outboxes(&platform.api.db.pool, &receipt).await;
    let payloads_before = payloads(&platform.api.db.pool, &receipt).await;
    let lookups_before = platform.api.authentication_lookups();
    let cas_rejections_before = if cas {
        Some(cas_primitive_rejections())
    } else {
        None
    };
    let accepted = if cas {
        submit_original_report_with_frontier_cas(&platform.api, receipt.clone(), &mut worker).await
    } else {
        let request = worker.req(receipt.completion.clone());
        worker
            .client
            .complete_step(request)
            .await
            .expect("submit exact immutable-old completion under current authentication")
            .into_inner()
    };
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert!(platform.api.authentication_lookups() > lookups_before);
    assert_successful_refusal(
        &platform.api,
        &receipt,
        outboxes_before,
        payloads_before,
        "OPEN",
        "FAILED",
        Some("RUN_DATA_LIMIT"),
    )
    .await;
    if strong {
        assert_under_limit_report_boundary(&platform.api.db.pool, &receipt).await;
        assert!(
            output_reference_fingerprint(&platform.api.db.pool, &receipt).await
                == output_fingerprint_before_report,
            "refused candidate changed an output, journal payload or blob reference"
        );
        assert_eq!(
            effective_totals(&platform.api.db.pool, &receipt).await,
            preset.expected_totals()
        );
    }
    let original_report_receipt =
        verified_report_receipt(&platform.api.db.pool, &receipt, &accepted.command_id).await;
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "ordinary report changed persisted worker authority"
    );
    let original_cas_stop = if cas {
        assert!(
            reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await
                == reservations_before_report,
            "committed report and rejected CAS preserve exact original reservations/run/tenant usage"
        );
        Some(cas_report_stop(&platform.api.db.pool, &receipt).await)
    } else {
        None
    };
    drop(pre_report_scheduler);

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

    if cas {
        assert!(
            reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await
                == reservations_before_report
                && cas_report_stop(&platform.api.db.pool, &receipt).await
                    == *original_cas_stop.as_ref().expect("original CAS stop"),
            "canonical report replay preserves original stop and capacity before any fresh scheduling"
        );
    }

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

    let scheduler = if strong {
        fresh_scheduler(&platform.api, &receipt).await
    } else {
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
        Scheduler::new(
            platform.api.executions.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings {
                discovery_rest: Duration::ZERO,
                ..SchedulerSettings::default()
            },
        )
        .with_placement(placement)
    };

    if cas {
        let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("target owner"));
        assert!(
            matches!(
                scheduler
                    .advance_collection(receipt.tenant, receipt.run, &owner)
                    .await,
                Ok(CollectionAdvance::DrainStarted)
            ),
            "fresh frontier selects stopped-for-limit drain"
        );
        assert!(
            cas_primitive_rejections()
                == cas_rejections_before.expect("captured rejection metric") + 1.0,
            "only the fresh committed drain increments primitive rejection counter"
        );
    } else {
        let drain_report = scheduler.tick().await;
        assert_eq!(
            drain_report.failed, 0,
            "normal durable discovery: {drain_report:?}"
        );
        assert_eq!(drain_report.collections_advanced, 1, "{drain_report:?}");
    }
    if cas {
        assert!(
            cas_report_stop(&platform.api.db.pool, &receipt).await
                == *original_cas_stop.as_ref().expect("original CAS stop"),
            "fresh drain preserves complete original report-created stop identity"
        );
        assert_cas_fresh_drain_boundary(&platform.api.db.pool, &receipt).await;
    }
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
    let stopped_limit_identity = if strong {
        let drain_evidence: (i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
              count(*) FILTER (WHERE reason='stopped_for_run_data_limit'), \
              count(*) FILTER (WHERE reason='existing_outputs'), \
              count(*) FILTER (WHERE reason='aggregate'), \
              (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 \
                AND event.run_id=$2 AND event.event_type='collection.drain_started' \
                AND event.node='c10' AND event.detail->>'reason'='run_data_limit'), \
              (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=$1 \
                AND message.aggregate_id=$2 AND message.event_type='collection.drain.started' \
                AND message.routing_key='c10' AND message.payload->>'reason'='FAILED') \
             FROM audit_records WHERE tenant_id=$1 AND resource_type='run' \
              AND resource_id=$2::text AND action='run.data_limit_exceeded'",
        )
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("exact stopped-limit drain evidence");
        assert_eq!(drain_evidence, (1, 0, 0, 1, 1));
        assert_eq!(
            effective_totals(&platform.api.db.pool, &receipt).await,
            preset.expected_totals()
        );
        Some(stopped_limit_drain_fingerprint(&platform.api.db.pool, &receipt).await)
    } else {
        None
    };
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
    assert!(
        compensation.target_instance == receipt.lease.instance,
        "compensation targets the exact original forward instance"
    );
    let compensation_instance = compensation
        .instance
        .as_ref()
        .expect("compensation identity");
    assert_eq!(compensation_instance.definition_node_id, "reserve~c");
    assert_eq!(compensation_instance.instance_path.len(), 1);
    assert_eq!(compensation_instance.instance_path[0].scope_node_id, "c10");
    assert_eq!(compensation_instance.instance_path[0].iteration_index, 0);
    if strong {
        let input: Value = serde_json::from_str(&compensation.input_json)
            .expect("ordinary compensation task context");
        assert_eq!(input["item"], json!(1));
        assert_eq!(input["index"], json!(0));
        assert!(
            input.pointer("/steps/reserve/output").is_none(),
            "refused forward output was synthesized in compensation input"
        );
        let pending =
            assert_pending_compensation(&platform.api.db.pool, &receipt, "RUN_DATA_LIMIT").await;
        let waiting = scheduler.tick().await;
        assert_eq!(waiting.failed, 0, "pending compensation tick: {waiting:?}");
        assert!(
            assert_pending_compensation(&platform.api.db.pool, &receipt, "RUN_DATA_LIMIT").await
                == pending,
            "normal scheduling finalized or replaced unfinished compensation"
        );
    }
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
    finish_after_compensation(&scheduler, &platform.api, &receipt, "COMPENSATED").await;

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
        preset.expected_totals()
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
    if strong {
        let compensation_output_absent: bool = sqlx::query_scalar(
            "SELECT output_digest IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 \
              AND node='reserve~c' AND instance_path_nodes=ARRAY['c10']::text[] \
              AND instance_path_indices=ARRAY[0]::integer[]",
        )
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("real compensation output reference");
        assert!(
            compensation_output_absent,
            "the real compensation retained an unexpected output"
        );
        assert!(
            retained_output_reference_fingerprint(&platform.api.db.pool, &receipt).await
                == retained_output_references_before_report,
            "report, drain or compensation changed retained output references"
        );
        let stopped_limit_identity = stopped_limit_identity
            .as_ref()
            .expect("captured stopped-limit drain identity");
        assert!(
            stopped_limit_drain_fingerprint(&platform.api.db.pool, &receipt).await
                == *stopped_limit_identity,
            "final compensation changed the original stop or drain identity"
        );
        assert!(
            stored_report_receipt(&platform.api.db.pool, &receipt).await == original_report_receipt,
            "final compensation changed the stored original report receipt"
        );
        let before_terminal_replay =
            under_limit_execution_footprint(&platform.api.db.pool, &receipt).await;
        let terminal_replay_request = worker.req(receipt.completion.clone());
        let terminal_replay = worker
            .client
            .complete_step(terminal_replay_request)
            .await
            .expect("replay original forward report after terminal compensation")
            .into_inner();
        assert!(terminal_replay.already_applied);
        assert!(
            terminal_replay.command_id == accepted.command_id
                && terminal_replay.step_status == "SUCCEEDED"
                && terminal_replay.run_status == "CANCELLING",
            "terminal replay changed the original stored ACK"
        );
        assert!(
            stored_report_receipt(&platform.api.db.pool, &receipt).await == original_report_receipt,
            "terminal replay changed the stored original report receipt"
        );
        assert!(
            under_limit_execution_footprint(&platform.api.db.pool, &receipt).await
                == before_terminal_replay,
            "terminal replay changed execution, audit, receipt or output state"
        );
        assert!(
            stopped_limit_drain_fingerprint(&platform.api.db.pool, &receipt).await
                == *stopped_limit_identity,
            "terminal replay changed the original stop or drain identity"
        );
        assert_eq!(
            effective_totals(&platform.api.db.pool, &receipt).await,
            preset.expected_totals()
        );
        assert!(
            footprint_before_report != before_terminal_replay,
            "report, drain and compensation did not create their durable transitions"
        );
    }
    fresh_verify(&platform.api, &receipt).await;
}

#[tokio::test]
#[ignore = "requires scripts/test-legacy-collection-inflight-report-v1.sh"]
async fn old_ordinary_success_before_drain_preserves_the_report_and_compensation() {
    run_report_before_drain_case(FixturePreset::Ordinary).await;
}

#[tokio::test]
#[ignore = "requires scripts/test-legacy-collection-inflight-report-v1.sh --case under-limit-increment-before-drain"]
async fn old_under_limit_increment_before_drain_preserves_the_report_and_compensation() {
    run_report_before_drain_case(FixturePreset::UnderLimitIncrement).await;
}

#[tokio::test]
#[ignore = "requires scripts/test-legacy-collection-inflight-report-v1.sh --case drain-before-report"]
async fn old_ordinary_success_after_drain_preserves_the_report_and_compensation() {
    run_drain_before_report_case(false).await;
}

#[tokio::test]
#[ignore = "requires scripts/test-legacy-collection-inflight-report-v1.sh --case live-work-frontier"]
async fn old_live_work_frontier_drains_before_waiting() {
    run_drain_before_report_case(true).await;
}

async fn live_frontier_inventory(pool: &PgPool, receipt: &Receipt) -> Value {
    let value:Value=sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'scope_id',s.scope_id,
      'progress',jsonb_build_array(s.next_admission_index,s.admitted_count,s.completed_count,s.active_count),
      'members',(SELECT jsonb_agg(to_jsonb(m) ORDER BY m.iteration_index) FROM collection_iterations m WHERE m.tenant_id=s.tenant_id AND m.run_id=s.run_id AND m.scope_id=s.scope_id),
      'identities',(SELECT jsonb_agg(jsonb_build_array(w.step_id,w.node,w.instance_path_nodes,w.instance_path_indices,w.kind) ORDER BY w.step_id) FROM steps w WHERE w.tenant_id=s.tenant_id AND w.run_id=s.run_id),
      'target_step_count',(SELECT count(*) FROM steps w WHERE w.tenant_id=s.tenant_id AND w.run_id=s.run_id AND w.instance_path_nodes[1]='c10'),
      'checkpoints',(SELECT count(*) FROM iteration_checkpoints c WHERE c.tenant_id=s.tenant_id AND c.run_id=s.run_id AND c.scope_id=s.scope_id),
      'done',(SELECT count(*) FROM steps w WHERE w.tenant_id=s.tenant_id AND w.run_id=s.run_id AND w.node='done'))
      FROM iteration_scopes s WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.owner_node='c10' AND cardinality(s.owner_path_nodes)=0"#)
        .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).fetch_one(pool).await.expect("original live-only frontier inventory");
    assert!(
        value["progress"] == json!([1, 1, 0, 1])
            && value["target_step_count"] == 1
            && value["checkpoints"] == 0
            && value["done"] == 0
            && value["members"].as_array().is_some_and(|m| m.len() == 1
                && m[0]["iteration_index"] == 0
                && m[0]["lifecycle"] == "ADMITTED"
                && m[0]["slot_held"] == true
                && m[0]["entry_instance_id"] == receipt.lease.step_id),
        "one exact held original member, no Return/checkpoint/End or free slot"
    );
    value
}

async fn run_drain_before_report_case(explicit_live_frontier: bool) {
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
        "q10c ordinary-drain-before-report API roles: producer={}, consumer={current_api_role}",
        receipt.producer_api_role
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "persisted worker authority changed during current API startup"
    );
    assert_live_target(&platform.api.db.pool, &receipt, false).await;
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
    let output_fingerprint = output_reference_fingerprint(&platform.api.db.pool, &receipt).await;
    assert_initial_drain_first_state(&platform.api.db.pool, &receipt, &output_fingerprint).await;

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

    let reservations_before = reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await;
    let frontier_inventory = if explicit_live_frontier {
        use sagat_application::scope_frontier::ScopeFrontierStore as _;
        use sagat_domain::execution::CollectionPhase;
        let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("live scope owner"));
        let frontier = match platform
            .api
            .executions
            .collection_frontier(receipt.tenant, receipt.run, &owner)
            .await
        {
            Ok(Some(value)) => value,
            _ => panic!("actual current live frontier"),
        };
        assert!(
            frontier.phase == CollectionPhase::Open
                && frontier.run.stop_reason.is_none()
                && frontier.run.stop_code.is_none()
                && frontier.run.effective_output_bytes == 4_200_020
                && frontier.item_count == 1
                && frontier.next_admission == 1
                && frontier.completed == 0
                && frontier.max_concurrency == 1
                && frontier.active.len() == 1,
            "genuine over-budget only-live-work frontier with no free admission slot"
        );
        let inventory = live_frontier_inventory(&platform.api.db.pool, &receipt).await;
        assert!(
            inventory["scope_id"] == frontier.scope_id.as_uuid().to_string(),
            "actual original frontier scope identity"
        );
        let stored_head: i64 =
            sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(receipt.tenant.as_uuid())
                .bind(receipt.run.as_uuid())
                .fetch_one(&platform.api.db.pool)
                .await
                .expect("original live frontier revision");
        assert!(
            frontier.run.version == u64::try_from(stored_head).expect("positive run revision"),
            "frontier revision matches the untouched original Run"
        );
        let member = &frontier.active[0];
        assert!(
            member.steps.len() == 1
                && member
                    .steps
                    .get(&member.entry)
                    .is_some_and(|step| step.status()
                        == sagat_domain::execution::StepStatus::Leased
                        && !step.cancel_requested()),
            "current frontier contains the original leased uncancelled reserve"
        );
        assert!(
            member.index == 0
                && member.first_failure().is_none()
                && member.pending_return(&frontier.run.definition).is_none()
                && member.entry.node().as_str() == "reserve"
                && member.entry.path().segments().len() == 1
                && member.entry.path().segments()[0].scope_node().as_str() == "c10"
                && member.entry.path().segments()[0].iteration_index() == 0,
            "live original member has no failure or pending Return"
        );
        drop(frontier);
        let result = scheduler
            .advance_collection(receipt.tenant, receipt.run, &owner)
            .await;
        assert!(
            matches!(result, Ok(CollectionAdvance::DrainStarted)),
            "first current public advance starts ExistingOutputs drain before Waiting"
        );
        Some(inventory)
    } else {
        let drain_report = scheduler.tick().await;
        assert_eq!(drain_report.failed, 0, "normal drain discovery failed");
        assert_eq!(
            drain_report.collections_advanced, 1,
            "normal discovery did not select the legacy c10 scope"
        );
        None
    };
    let frozen_stop_drain = assert_first_drain(
        &platform.api.db.pool,
        &receipt,
        outboxes_before,
        payloads_before,
        &output_fingerprint,
    )
    .await;
    if let Some(inventory) = frontier_inventory {
        assert!(
            live_frontier_inventory(&platform.api.db.pool, &receipt).await == inventory,
            "explicit drain preserves admission/member/identity inventories without checkpoint or successor"
        );
    }
    assert_live_target(&platform.api.db.pool, &receipt, true).await;
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "drain changed persisted worker authority"
    );
    fresh_verify(&platform.api, &receipt).await;

    assert!(
        reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await == reservations_before,
        "initial drain changed the live member reservations or run/tenant iteration usage"
    );
    let waiting = drain_wait_fingerprint(&platform.api.db.pool, &receipt).await;
    let wait_report = scheduler.tick().await;
    assert_eq!(wait_report.failed, 0, "live-member wait failed");
    assert_eq!(
        wait_report.collections_advanced, 0,
        "a live member must not count as collection progress"
    );
    assert!(
        wait_report.skipped >= 1,
        "the live draining scope was not observed waiting"
    );
    assert!(
        drain_wait_fingerprint(&platform.api.db.pool, &receipt).await == waiting,
        "a live cancellation request was incorrectly treated as a drained member"
    );
    assert_live_target(&platform.api.db.pool, &receipt, true).await;
    assert!(
        stop_drain_fingerprint(&platform.api.db.pool, &receipt).await == frozen_stop_drain,
        "the live-member wait changed the original stop or drain"
    );
    assert_eq!(
        outboxes(&platform.api.db.pool, &receipt).await,
        outboxes_before + 1
    );
    fresh_verify(&platform.api, &receipt).await;

    assert!(
        reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await == reservations_before,
        "live-member wait changed the original reservations or run/tenant iteration usage"
    );
    let lookups_before = platform.api.authentication_lookups();
    let request = worker.req(receipt.completion.clone());
    let accepted = worker
        .client
        .complete_step(request)
        .await
        .expect("submit the exact old success after current drain")
        .into_inner();
    assert!(!accepted.already_applied);
    assert_eq!(accepted.step_status, "SUCCEEDED");
    assert_eq!(accepted.run_status, "CANCELLING");
    assert!(platform.api.authentication_lookups() > lookups_before);
    assert_successful_refusal(
        &platform.api,
        &receipt,
        outboxes_before + 1,
        payloads_before,
        "DRAINING",
        "FAILED",
        Some("RUN_DATA_LIMIT"),
    )
    .await;
    assert!(
        stop_drain_fingerprint(&platform.api.db.pool, &receipt).await == frozen_stop_drain,
        "the genuine success replaced the committed stop or drain"
    );
    assert!(
        output_reference_fingerprint(&platform.api.db.pool, &receipt).await == output_fingerprint,
        "the refused success retained an output or blob reference"
    );
    let order: (i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT seq FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
            AND event_type='step.cancel_requested' AND node='reserve' \
            AND instance_path_nodes=ARRAY['c10']::text[] \
            AND instance_path_indices=ARRAY[0]::integer[]), \
          (SELECT seq FROM run_events WHERE tenant_id=$1 AND run_id=$2 \
            AND event_type='step.completed' AND node='reserve' \
            AND instance_path_nodes=ARRAY['c10']::text[] \
            AND instance_path_indices=ARRAY[0]::integer[])",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("drain-before-report journal order");
    assert!(
        order.0 < order.1,
        "the original success did not follow the cancellation request"
    );
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
    assert!(
        stop_drain_fingerprint(&platform.api.db.pool, &receipt).await == frozen_stop_drain,
        "exact replay changed the original stop or drain"
    );
    assert!(
        output_reference_fingerprint(&platform.api.db.pool, &receipt).await == output_fingerprint,
        "exact replay changed output or blob references"
    );
    assert_eq!(
        outboxes(&platform.api.db.pool, &receipt).await,
        outboxes_before + 1
    );
    assert_eq!(
        payloads(&platform.api.db.pool, &receipt).await,
        payloads_before
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "exact replay changed persisted worker authority"
    );
    fresh_verify(&platform.api, &receipt).await;

    let compensation = poll_compensation(&scheduler, &mut worker).await;
    assert_eq!(compensation.task_kind, "COMPENSATION");
    assert_eq!(compensation.activity_name, "line.release");
    assert_eq!(compensation.target_node, "reserve");
    assert_eq!(
        compensation.target_idempotency_key,
        receipt.lease.idempotency_key
    );
    assert!(
        compensation.target_instance == receipt.lease.instance,
        "compensation targets the exact original forward instance"
    );
    let compensation_instance = compensation
        .instance
        .as_ref()
        .expect("compensation identity");
    assert_eq!(compensation_instance.definition_node_id, "reserve~c");
    assert_eq!(compensation_instance.instance_path.len(), 1);
    assert_eq!(compensation_instance.instance_path[0].scope_node_id, "c10");
    assert_eq!(compensation_instance.instance_path[0].iteration_index, 0);

    let pending_compensation =
        assert_pending_compensation(&platform.api.db.pool, &receipt, "RUN_DATA_LIMIT").await;
    let pending_report = scheduler.tick().await;
    assert_eq!(pending_report.failed, 0, "leased compensation wait failed");
    assert!(
        assert_pending_compensation(&platform.api.db.pool, &receipt, "RUN_DATA_LIMIT").await
            == pending_compensation,
        "normal scheduling finalized or replaced an unfinished compensation"
    );
    assert!(
        stop_drain_fingerprint(&platform.api.db.pool, &receipt).await == frozen_stop_drain,
        "compensation planning changed the original stop or drain"
    );

    let compensation_ack = worker
        .complete(&compensation, &json!({"released": true}))
        .await
        .expect("complete the real engine compensation task");
    assert!(!compensation_ack.already_applied);
    assert_eq!(compensation_ack.step_status, "SUCCEEDED");
    finish_after_compensation(&scheduler, &platform.api, &receipt, "COMPENSATED").await;

    let final_state = sqlx::query(
        "SELECT run.status,run.result_digest IS NULL AS no_run_result, \
                scope.lifecycle,scope.drain_kind,scope.drain_failure_code, \
                owner.status AS owner_status,owner.failure_code AS owner_failure, \
                saga.phase,saga.outcome,forward.status AS forward_status, \
                forward.output_digest IS NULL AS no_forward_output, \
                compensation.status AS compensation_status, \
                telemetry.head_seq,run.run_version, \
                (SELECT count(*) FROM scope_saga_entries entry WHERE entry.tenant_id=$1 \
                  AND entry.run_id=$2 AND entry.owner_node='c10' \
                  AND entry.iteration_index=0 AND entry.node='reserve' \
                  AND entry.kind='COMPENSATE') AS saga_entries, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='collection.iteration_drained' AND event.node='c10') AS drained_events, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='step.completed' AND event.node='reserve~c' \
                  AND event.instance_path_nodes=ARRAY['c10']::text[] \
                  AND event.instance_path_indices=ARRAY[0]::integer[]) AS compensation_completions, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='collection.finished' AND event.node='c10' \
                  AND event.detail->>'outcome'='failed' \
                  AND event.detail->>'code'='RUN_DATA_LIMIT') AS collection_finishes, \
                (SELECT count(*) FROM run_events event WHERE event.tenant_id=$1 AND event.run_id=$2 \
                  AND event.event_type='run.finished' AND event.detail->>'status'='COMPENSATED') AS run_finishes, \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=$1 \
                  AND message.aggregate_id=$2 AND message.event_type='collection.finished' \
                  AND message.routing_key='c10' AND message.payload->>'status'='FAILED') AS collection_outboxes, \
                (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=$1 \
                  AND message.aggregate_id=$2 AND message.event_type='run.finished' \
                  AND message.payload->>'status'='COMPENSATED') AS run_outboxes, \
                (SELECT count(*) FROM steps final_step WHERE final_step.tenant_id=$1 \
                  AND final_step.run_id=$2 AND final_step.node='done' \
                  AND cardinality(final_step.instance_path_nodes)=0) AS end_steps, \
                (SELECT count(*) FROM steps reconciliation WHERE reconciliation.tenant_id=$1 \
                  AND reconciliation.run_id=$2 AND reconciliation.node='reserve~r' \
                  AND reconciliation.instance_path_nodes=ARRAY['c10']::text[] \
                  AND reconciliation.instance_path_indices=ARRAY[0]::integer[]) AS reconciliations \
           FROM runs run JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
            AND scope.owner_path_nodes='{}'::text[] AND scope.owner_path_indices='{}'::integer[] \
            AND scope.owner_node='c10' \
           JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
            AND owner.instance_path_nodes='{}'::text[] AND owner.instance_path_indices='{}'::integer[] \
            AND owner.node='c10' \
           JOIN scope_sagas saga ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
            AND saga.owner_path_nodes='{}'::text[] AND saga.owner_path_indices='{}'::integer[] \
            AND saga.owner_node='c10' \
           JOIN steps forward ON forward.tenant_id=run.tenant_id AND forward.run_id=run.id \
            AND forward.node='reserve' AND forward.instance_path_nodes=ARRAY['c10']::text[] \
            AND forward.instance_path_indices=ARRAY[0]::integer[] \
           JOIN steps compensation ON compensation.tenant_id=run.tenant_id \
            AND compensation.run_id=run.id AND compensation.node='reserve~c' \
            AND compensation.instance_path_nodes=ARRAY['c10']::text[] \
            AND compensation.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("final drain-first compensated state");
    assert_eq!(final_state.get::<String, _>("status"), "COMPENSATED");
    assert!(final_state.get::<bool, _>("no_run_result"));
    assert_eq!(final_state.get::<String, _>("lifecycle"), "FAILED");
    assert_eq!(
        final_state
            .get::<Option<String>, _>("drain_kind")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(
        final_state
            .get::<Option<String>, _>("drain_failure_code")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(final_state.get::<String, _>("owner_status"), "FAILED");
    assert_eq!(
        final_state
            .get::<Option<String>, _>("owner_failure")
            .as_deref(),
        Some("RUN_DATA_LIMIT")
    );
    assert_eq!(final_state.get::<String, _>("phase"), "DONE");
    assert_eq!(
        final_state.get::<Option<String>, _>("outcome").as_deref(),
        Some("COMPENSATED")
    );
    assert_eq!(final_state.get::<String, _>("forward_status"), "SUCCEEDED");
    assert!(final_state.get::<bool, _>("no_forward_output"));
    assert_eq!(
        final_state.get::<String, _>("compensation_status"),
        "SUCCEEDED"
    );
    assert_eq!(
        final_state.get::<i64, _>("head_seq"),
        final_state.get::<i64, _>("run_version")
    );
    assert_eq!(final_state.get::<i64, _>("saga_entries"), 1);
    assert_eq!(final_state.get::<i64, _>("drained_events"), 1);
    assert_eq!(final_state.get::<i64, _>("compensation_completions"), 1);
    assert_eq!(final_state.get::<i64, _>("collection_finishes"), 1);
    assert_eq!(final_state.get::<i64, _>("run_finishes"), 1);
    assert_eq!(final_state.get::<i64, _>("collection_outboxes"), 1);
    assert_eq!(final_state.get::<i64, _>("run_outboxes"), 1);
    assert_eq!(final_state.get::<i64, _>("end_steps"), 0);
    assert_eq!(final_state.get::<i64, _>("reconciliations"), 0);
    assert_eq!(
        effective_totals(&platform.api.db.pool, &receipt).await,
        (2_100_000, 2_100_020, 4_200_020)
    );
    assert!(
        source_witness(&platform.api.db.pool, receipt.tenant, receipt.run).await
            == receipt.source_witness,
        "closed source witness changed during drain-first compensation"
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "compensation changed persisted worker authority"
    );
    assert!(
        stop_drain_fingerprint(&platform.api.db.pool, &receipt).await == frozen_stop_drain,
        "final compensation changed the original stop or drain"
    );
    assert!(
        stored_report_receipt(&platform.api.db.pool, &receipt).await == original_report_receipt,
        "final scheduling changed the exact report receipt"
    );
    fresh_verify(&platform.api, &receipt).await;
}

#[tokio::test]
#[ignore = "requires scripts/test-legacy-collection-inflight-report-v1.sh"]
async fn old_ordinary_success_after_cancellation_drain_preserves_the_original_stop() {
    run_cancellation_before_report_case(FixturePreset::Ordinary).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires separate immutable-old under-limit producer; see fixture README"]
async fn old_under_limit_increment_after_cancellation_drain_preserves_the_original_stop() {
    run_cancellation_before_report_case(FixturePreset::UnderLimitIncrement).await;
}

async fn run_cancellation_before_report_case(preset: FixturePreset) {
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
    let receipt = Receipt::parse_for(
        &std::fs::read(&receipt_path).expect("read private receipt"),
        preset,
    );
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
        "q10c cancellation-before-report API roles: producer={}, consumer={current_api_role}, preset={}",
        receipt.producer_api_role,
        preset.format()
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "persisted worker authority changed during current API startup"
    );
    assert_live_target(&platform.api.db.pool, &receipt, false).await;
    assert_eq!(
        effective_totals(&platform.api.db.pool, &receipt).await,
        receipt.preset.expected_totals()
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
    let output_fingerprint = output_reference_fingerprint(&platform.api.db.pool, &receipt).await;
    let reservations_before = reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await;
    assert_initial_drain_first_state(&platform.api.db.pool, &receipt, &output_fingerprint).await;
    let retained_before =
        retained_output_reference_fingerprint(&platform.api.db.pool, &receipt).await;
    if preset == FixturePreset::UnderLimitIncrement {
        let (l, _, e) = effective_totals(&platform.api.db.pool, &receipt).await;
        let candidate =
            i64::try_from(receipt.completion.output_json.len()).expect("candidate byte length");
        assert!(
            candidate == 250_000
                && e <= 4_194_304
                && l + candidate <= 4_194_304
                && e + candidate > 4_194_304,
            "genuine retained total fits but exact original success increment exceeds effective budget"
        );
        assert_under_limit_full_overlay(&platform.api, &receipt).await;
    }

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

    let cancellation_absence: (i64, i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM run_events stop_event WHERE stop_event.tenant_id=$1 \
            AND stop_event.run_id=$2 AND stop_event.event_type='run.stop_requested'), \
          (SELECT count(*) FROM audit_records cancel_audit WHERE cancel_audit.tenant_id=$1 \
            AND cancel_audit.resource_type='run' AND cancel_audit.resource_id=$2::text \
            AND cancel_audit.action='run.cancel_requested'), \
          (SELECT count(*) FROM command_dedupe cancel_receipt \
            WHERE cancel_receipt.tenant_id=$1 AND cancel_receipt.scope=$3 \
              AND cancel_receipt.idempotency_key=$4)",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(operation_scope(&receipt))
    .bind(CANCELLATION_KEY)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("no public cancellation evidence before the command");
    assert_eq!(cancellation_absence, (0, 0, 0));
    let pre_cancel_version: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(receipt.tenant.as_uuid())
            .bind(receipt.run.as_uuid())
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("actual pre-cancellation journal head");
    let cancel_lookups_before = platform.api.authentication_lookups();
    let cancel_response = platform
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{}/operations", receipt.run),
            "alice",
            "acme",
        )
        .header("X-Octopus-Namespace", "payments")
        .header("Idempotency-Key", CANCELLATION_KEY)
        .json(&json!({
            "action": "CANCEL",
            "reason": CANCELLATION_REASON,
            "expectedRunVersion": pre_cancel_version,
        }))
        .send()
        .await
        .expect("public cancellation before legacy drain");
    assert_eq!(cancel_response.status(), StatusCode::ACCEPTED);
    let cancel_body = crate::support::api::json_of(cancel_response).await;
    assert!(
        cancel_body["accepted"].as_bool() == Some(true),
        "public cancellation was not accepted"
    );
    assert!(
        cancel_body["runStatus"].as_str() == Some("CANCELLING"),
        "public cancellation returned the wrong status"
    );
    assert!(
        cancel_body["runVersion"].as_i64() == Some(pre_cancel_version + 1),
        "public cancellation returned the wrong committed version"
    );
    let operation_id = expected_operation_id(&receipt);
    assert!(
        cancel_body["operationId"].as_str() == Some(operation_id.as_str()),
        "public cancellation returned a nondeterministic operation id"
    );
    assert!(
        platform.api.authentication_lookups() > cancel_lookups_before,
        "public cancellation did not authenticate a fresh current JWT"
    );
    let (operation_receipt, frozen_public_cancel) = assert_public_cancellation(
        &platform.api.db.pool,
        &receipt,
        pre_cancel_version,
        outboxes_before,
        payloads_before,
        &output_fingerprint,
    )
    .await;
    assert_live_target(&platform.api.db.pool, &receipt, false).await;
    assert!(
        reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await == reservations_before,
        "public cancellation changed live reservations or run/tenant iteration usage"
    );
    fresh_verify(&platform.api, &receipt).await;

    let drain_report = scheduler.tick().await;
    assert_eq!(drain_report.failed, 0, "normal cancellation drain failed");
    assert_eq!(
        drain_report.collections_advanced, 1,
        "normal discovery did not select the publicly cancelled c10 scope"
    );
    let frozen_cancellation_drain = assert_cancellation_first_drain(
        &platform.api.db.pool,
        &receipt,
        outboxes_before,
        payloads_before,
        &output_fingerprint,
    )
    .await;
    assert!(
        public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_public_cancel,
        "normal drain changed the public cancellation receipt, audit or stop"
    );
    assert_live_target(&platform.api.db.pool, &receipt, true).await;
    assert!(
        reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await == reservations_before,
        "cancellation drain changed live reservations or run/tenant iteration usage"
    );
    fresh_verify(&platform.api, &receipt).await;

    let waiting = drain_wait_fingerprint(&platform.api.db.pool, &receipt).await;
    let wait_report = scheduler.tick().await;
    assert_eq!(wait_report.failed, 0, "cancelled live-member wait failed");
    assert_eq!(
        wait_report.collections_advanced, 0,
        "a live cancelled member must not count as collection progress"
    );
    assert!(
        wait_report.skipped >= 1,
        "the cancelled live draining scope was not observed waiting"
    );
    assert!(
        drain_wait_fingerprint(&platform.api.db.pool, &receipt).await == waiting,
        "a live public cancellation was incorrectly treated as a drained member"
    );
    assert_live_target(&platform.api.db.pool, &receipt, true).await;
    assert!(
        public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_public_cancel,
        "live-member wait changed the public cancellation"
    );
    assert!(
        cancellation_drain_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_cancellation_drain,
        "live-member wait changed the cancellation drain"
    );
    assert!(
        reservation_usage_fingerprint(&platform.api.db.pool, &receipt).await == reservations_before,
        "live-member wait changed reservations or run/tenant iteration usage"
    );
    assert_eq!(
        outboxes(&platform.api.db.pool, &receipt).await,
        outboxes_before + 1
    );
    fresh_verify(&platform.api, &receipt).await;

    assert_cancellation_data_limit_audits(&platform.api.db.pool, &receipt, 0).await;
    let report_lookups_before = platform.api.authentication_lookups();
    let report_request = worker.req(receipt.completion.clone());
    let accepted = worker
        .client
        .complete_step(report_request)
        .await
        .expect("submit exact old success after public cancellation drain")
        .into_inner();
    assert!(!accepted.already_applied);
    assert!(
        accepted.step_status == "SUCCEEDED" && accepted.run_status == "CANCELLING",
        "the old completion returned the wrong accepted status"
    );
    assert!(platform.api.authentication_lookups() > report_lookups_before);
    assert_successful_refusal(
        &platform.api,
        &receipt,
        outboxes_before + 1,
        payloads_before,
        "DRAINING",
        "CANCELLED",
        None,
    )
    .await;
    assert!(
        public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_public_cancel,
        "the genuine success changed the public cancellation"
    );
    assert!(
        cancellation_drain_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_cancellation_drain,
        "the genuine success changed the cancellation drain"
    );
    assert!(
        output_reference_fingerprint(&platform.api.db.pool, &receipt).await == output_fingerprint,
        "the cancelled refused success retained an output or blob reference"
    );
    let order: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT stop_event.seq FROM run_events stop_event WHERE stop_event.tenant_id=$1 \
            AND stop_event.run_id=$2 AND stop_event.event_type='run.stop_requested'), \
          (SELECT drain_event.seq FROM run_events drain_event WHERE drain_event.tenant_id=$1 \
            AND drain_event.run_id=$2 AND drain_event.event_type='collection.drain_started' \
            AND drain_event.node='c10'), \
          (SELECT cancel_event.seq FROM run_events cancel_event WHERE cancel_event.tenant_id=$1 \
            AND cancel_event.run_id=$2 AND cancel_event.event_type='step.cancel_requested' \
            AND cancel_event.node='reserve' \
            AND cancel_event.instance_path_nodes=ARRAY['c10']::text[] \
            AND cancel_event.instance_path_indices=ARRAY[0]::integer[]), \
          (SELECT completion_event.seq FROM run_events completion_event \
            WHERE completion_event.tenant_id=$1 AND completion_event.run_id=$2 \
              AND completion_event.event_type='step.completed' \
              AND completion_event.node='reserve' \
              AND completion_event.instance_path_nodes=ARRAY['c10']::text[] \
              AND completion_event.instance_path_indices=ARRAY[0]::integer[])",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("public cancel, drain, lease cancellation and report order");
    assert!(
        order.0 < order.1 && order.1 < order.2 && order.2 < order.3,
        "public cancellation ordering changed"
    );
    assert_cancellation_data_limit_audits(&platform.api.db.pool, &receipt, 1).await;
    let original_report_receipt =
        verified_report_receipt(&platform.api.db.pool, &receipt, &accepted.command_id).await;
    fresh_verify(&platform.api, &receipt).await;

    let first_report_identity = report_identity(&platform.api.db.pool, &receipt).await;
    let replay_request = worker.req(receipt.completion.clone());
    let replay = worker
        .client
        .complete_step(replay_request)
        .await
        .expect("exact cancelled report receipt replay")
        .into_inner();
    assert!(replay.already_applied);
    assert!(
        replay.command_id == accepted.command_id
            && replay.step_status == "SUCCEEDED"
            && replay.run_status == "CANCELLING",
        "the exact completion replay changed its accepted identity"
    );
    assert_eq!(
        report_identity(&platform.api.db.pool, &receipt).await,
        first_report_identity
    );
    let replayed_report_receipt =
        verified_report_receipt(&platform.api.db.pool, &receipt, &replay.command_id).await;
    assert!(
        replayed_report_receipt == original_report_receipt,
        "cancelled report replay changed its exact receipt"
    );

    assert_cancellation_data_limit_audits(&platform.api.db.pool, &receipt, 1).await;
    let current_before_cancel_replay: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(receipt.tenant.as_uuid())
            .bind(receipt.run.as_uuid())
            .fetch_one(&platform.api.db.pool)
            .await
            .expect("current journal head before public operation replay");
    assert!(
        current_before_cancel_replay > pre_cancel_version + 1,
        "public operation replay must observe a newer current projection"
    );
    let public_replay_lookups = platform.api.authentication_lookups();
    let cancel_replay_response = platform
        .api
        .as_user(
            Method::POST,
            &format!("/runs/{}/operations", receipt.run),
            "alice",
            "acme",
        )
        .header("X-Octopus-Namespace", "payments")
        .header("Idempotency-Key", CANCELLATION_KEY)
        .json(&json!({
            "action": "CANCEL",
            "reason": CANCELLATION_REASON,
            "expectedRunVersion": pre_cancel_version,
        }))
        .send()
        .await
        .expect("replay public cancellation against current projection");
    assert_eq!(cancel_replay_response.status(), StatusCode::ACCEPTED);
    let cancel_replay_body = crate::support::api::json_of(cancel_replay_response).await;
    assert!(
        cancel_replay_body["accepted"].as_bool() == Some(true),
        "public cancellation replay was not accepted"
    );
    assert!(
        cancel_replay_body["operationId"].as_str() == Some(operation_id.as_str()),
        "public cancellation replay changed operation id"
    );
    assert!(
        cancel_replay_body["runStatus"].as_str() == Some("CANCELLING"),
        "public cancellation replay returned the wrong current status"
    );
    assert!(
        cancel_replay_body["runVersion"].as_i64() == Some(current_before_cancel_replay),
        "public cancellation replay returned a frozen acknowledgement"
    );
    assert!(platform.api.authentication_lookups() > public_replay_lookups);
    assert!(
        verified_operation_receipt(&platform.api.db.pool, &receipt, pre_cancel_version).await
            == operation_receipt,
        "public cancellation replay changed its exact receipt"
    );
    assert!(
        public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_public_cancel,
        "public cancellation replay changed the original public stop"
    );
    assert!(
        cancellation_drain_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_cancellation_drain,
        "public cancellation replay changed the original drain"
    );
    assert_eq!(
        report_identity(&platform.api.db.pool, &receipt).await,
        first_report_identity
    );
    assert!(
        stored_report_receipt(&platform.api.db.pool, &receipt).await == original_report_receipt,
        "public cancellation replay changed the report receipt"
    );
    assert!(
        output_reference_fingerprint(&platform.api.db.pool, &receipt).await == output_fingerprint,
        "public cancellation replay changed output or blob references"
    );
    assert_eq!(
        outboxes(&platform.api.db.pool, &receipt).await,
        outboxes_before + 1
    );
    assert_eq!(
        payloads(&platform.api.db.pool, &receipt).await,
        payloads_before
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "public cancellation replay changed persisted worker authority"
    );
    fresh_verify(&platform.api, &receipt).await;

    assert_cancellation_data_limit_audits(&platform.api.db.pool, &receipt, 1).await;
    let compensation = poll_compensation(&scheduler, &mut worker).await;
    assert!(
        compensation.task_kind == "COMPENSATION"
            && compensation.activity_name == "line.release"
            && compensation.target_node == "reserve"
            && compensation.target_idempotency_key == receipt.lease.idempotency_key,
        "the cancellation compensation changed its reviewed task or target identity"
    );
    assert!(
        compensation.target_instance == receipt.lease.instance,
        "cancel compensation targets the exact original forward instance"
    );
    let compensation_instance = compensation
        .instance
        .as_ref()
        .expect("cancellation compensation identity");
    assert!(
        compensation_instance.definition_node_id == "reserve~c"
            && compensation_instance.instance_path.len() == 1
            && compensation_instance.instance_path[0].scope_node_id == "c10"
            && compensation_instance.instance_path[0].iteration_index == 0,
        "the cancellation compensation changed its reviewed scoped instance"
    );

    let pending_compensation =
        assert_pending_compensation(&platform.api.db.pool, &receipt, "RUN_CANCELLED").await;
    let pending_report = scheduler.tick().await;
    assert_eq!(pending_report.failed, 0, "cancel compensation wait failed");
    assert!(
        assert_pending_compensation(&platform.api.db.pool, &receipt, "RUN_CANCELLED").await
            == pending_compensation,
        "normal scheduling finalized or replaced unfinished cancel compensation"
    );
    assert!(
        public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_public_cancel,
        "compensation planning changed the public cancellation"
    );
    assert!(
        cancellation_drain_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_cancellation_drain,
        "compensation planning changed the cancellation drain"
    );

    let compensation_ack = worker
        .complete(&compensation, &json!({"released": true}))
        .await
        .expect("complete real cancellation compensation task");
    assert!(
        !compensation_ack.already_applied && compensation_ack.step_status == "SUCCEEDED",
        "the real cancellation compensation did not commit one success"
    );
    finish_after_compensation(&scheduler, &platform.api, &receipt, "CANCELLED").await;

    let final_state = sqlx::query(
        "SELECT run.status AS run_status,run.stop_reason,run.stop_code, \
                run.result_digest IS NULL AS no_run_result,scope.lifecycle, \
                scope.drain_kind,scope.drain_failure_code, \
                owner.status AS owner_status,owner.failure_code AS owner_failure, \
                saga.phase,saga.cause_code,saga.outcome,forward.status AS forward_status, \
                forward.output_digest IS NULL AS no_forward_output, \
                compensation.status AS compensation_status, \
                telemetry.head_seq,run.run_version, \
                (SELECT count(*) FROM scope_saga_entries saga_entry \
                  WHERE saga_entry.tenant_id=$1 AND saga_entry.run_id=$2 \
                    AND saga_entry.owner_path_nodes='{}'::text[] \
                    AND saga_entry.owner_path_indices='{}'::integer[] \
                    AND saga_entry.owner_node='c10' AND saga_entry.iteration_index=0 \
                    AND saga_entry.node='reserve' AND saga_entry.kind='COMPENSATE') AS saga_entries, \
                (SELECT count(*) FROM run_events drain_event \
                  WHERE drain_event.tenant_id=$1 AND drain_event.run_id=$2 \
                    AND drain_event.event_type='collection.iteration_drained' \
                    AND drain_event.node='c10') AS drained_events, \
                (SELECT count(*) FROM run_events completion_event \
                  WHERE completion_event.tenant_id=$1 AND completion_event.run_id=$2 \
                    AND completion_event.event_type='step.completed' \
                    AND completion_event.node='reserve~c' \
                    AND completion_event.instance_path_nodes=ARRAY['c10']::text[] \
                    AND completion_event.instance_path_indices=ARRAY[0]::integer[]) AS compensation_completions, \
                (SELECT count(*) FROM run_events finish_event \
                  WHERE finish_event.tenant_id=$1 AND finish_event.run_id=$2 \
                    AND finish_event.event_type='collection.finished' \
                    AND finish_event.node='c10' \
                    AND finish_event.detail->>'outcome'='cancelled' \
                    AND finish_event.detail->>'code' IS NULL) AS collection_finishes, \
                (SELECT count(*) FROM run_events finish_event \
                  WHERE finish_event.tenant_id=$1 AND finish_event.run_id=$2 \
                    AND finish_event.event_type='run.finished' \
                    AND finish_event.detail->>'status'='CANCELLED') AS run_finishes, \
                (SELECT count(*) FROM event_outbox finish_message \
                  WHERE finish_message.tenant_id=$1 AND finish_message.aggregate_id=$2 \
                    AND finish_message.event_type='collection.finished' \
                    AND finish_message.routing_key='c10' \
                    AND finish_message.payload->>'status'='CANCELLED') AS collection_outboxes, \
                (SELECT count(*) FROM event_outbox finish_message \
                  WHERE finish_message.tenant_id=$1 AND finish_message.aggregate_id=$2 \
                    AND finish_message.event_type='run.finished' \
                    AND finish_message.payload->>'status'='CANCELLED') AS run_outboxes, \
                (SELECT count(*) FROM steps final_step \
                  WHERE final_step.tenant_id=$1 AND final_step.run_id=$2 \
                    AND final_step.node='done' \
                    AND cardinality(final_step.instance_path_nodes)=0) AS end_steps, \
                (SELECT count(*) FROM steps reconciliation \
                  WHERE reconciliation.tenant_id=$1 AND reconciliation.run_id=$2 \
                    AND reconciliation.node='reserve~r' \
                    AND reconciliation.instance_path_nodes=ARRAY['c10']::text[] \
                    AND reconciliation.instance_path_indices=ARRAY[0]::integer[]) AS reconciliations, \
                (SELECT count(*) FROM audit_records cancel_audit \
                  WHERE cancel_audit.tenant_id=$1 AND cancel_audit.resource_type='run' \
                    AND cancel_audit.resource_id=$2::text \
                    AND cancel_audit.action='run.cancel_requested' \
                    AND cancel_audit.actor='alice' \
                    AND cancel_audit.reason=$3) AS cancel_audits, \
                (SELECT count(*) FROM audit_records refusal_audit \
                  WHERE refusal_audit.tenant_id=$1 AND refusal_audit.resource_type='run' \
                    AND refusal_audit.resource_id=$2::text \
                    AND refusal_audit.action='run.data_limit_exceeded' \
                    AND refusal_audit.reason='successful scoped output refused by run data limit') AS refusal_audits, \
                (SELECT count(*) FROM audit_records drain_audit \
                  WHERE drain_audit.tenant_id=$1 AND drain_audit.resource_type='run' \
                    AND drain_audit.resource_id=$2::text \
                    AND drain_audit.action='run.data_limit_exceeded' \
                    AND drain_audit.reason IS DISTINCT FROM 'successful scoped output refused by run data limit') AS data_limit_drain_audits \
           FROM runs run JOIN telemetry_pending telemetry \
             ON telemetry.tenant_id=run.tenant_id AND telemetry.run_id=run.id \
           JOIN iteration_scopes scope ON scope.tenant_id=run.tenant_id \
            AND scope.run_id=run.id AND scope.owner_path_nodes='{}'::text[] \
            AND scope.owner_path_indices='{}'::integer[] AND scope.owner_node='c10' \
           JOIN steps owner ON owner.tenant_id=run.tenant_id AND owner.run_id=run.id \
            AND owner.instance_path_nodes='{}'::text[] \
            AND owner.instance_path_indices='{}'::integer[] AND owner.node='c10' \
           JOIN scope_sagas saga ON saga.tenant_id=run.tenant_id AND saga.run_id=run.id \
            AND saga.owner_path_nodes='{}'::text[] \
            AND saga.owner_path_indices='{}'::integer[] AND saga.owner_node='c10' \
           JOIN steps forward ON forward.tenant_id=run.tenant_id AND forward.run_id=run.id \
            AND forward.node='reserve' \
            AND forward.instance_path_nodes=ARRAY['c10']::text[] \
            AND forward.instance_path_indices=ARRAY[0]::integer[] \
           JOIN steps compensation ON compensation.tenant_id=run.tenant_id \
            AND compensation.run_id=run.id AND compensation.node='reserve~c' \
            AND compensation.instance_path_nodes=ARRAY['c10']::text[] \
            AND compensation.instance_path_indices=ARRAY[0]::integer[] \
          WHERE run.tenant_id=$1 AND run.id=$2",
    )
    .bind(receipt.tenant.as_uuid())
    .bind(receipt.run.as_uuid())
    .bind(CANCELLATION_REASON)
    .fetch_one(&platform.api.db.pool)
    .await
    .expect("final cancellation-first compensated state");
    assert_eq!(final_state.get::<String, _>("run_status"), "CANCELLED");
    assert_eq!(
        final_state
            .get::<Option<String>, _>("stop_reason")
            .as_deref(),
        Some("CANCELLED")
    );
    assert!(final_state.get::<Option<String>, _>("stop_code").is_none());
    assert!(final_state.get::<bool, _>("no_run_result"));
    assert_eq!(final_state.get::<String, _>("lifecycle"), "CANCELLED");
    assert_eq!(
        final_state
            .get::<Option<String>, _>("drain_kind")
            .as_deref(),
        Some("CANCELLED")
    );
    assert!(
        final_state
            .get::<Option<String>, _>("drain_failure_code")
            .is_none()
    );
    assert_eq!(final_state.get::<String, _>("owner_status"), "CANCELLED");
    assert!(
        final_state
            .get::<Option<String>, _>("owner_failure")
            .is_none()
    );
    assert_eq!(final_state.get::<String, _>("phase"), "DONE");
    assert_eq!(final_state.get::<String, _>("cause_code"), "RUN_CANCELLED");
    assert_eq!(
        final_state.get::<Option<String>, _>("outcome").as_deref(),
        Some("COMPENSATED")
    );
    assert_eq!(final_state.get::<String, _>("forward_status"), "SUCCEEDED");
    assert!(final_state.get::<bool, _>("no_forward_output"));
    assert_eq!(
        final_state.get::<String, _>("compensation_status"),
        "SUCCEEDED"
    );
    assert_eq!(
        final_state.get::<i64, _>("head_seq"),
        final_state.get::<i64, _>("run_version")
    );
    assert_eq!(final_state.get::<i64, _>("saga_entries"), 1);
    assert_eq!(final_state.get::<i64, _>("drained_events"), 1);
    assert_eq!(final_state.get::<i64, _>("compensation_completions"), 1);
    assert_eq!(final_state.get::<i64, _>("collection_finishes"), 1);
    assert_eq!(final_state.get::<i64, _>("run_finishes"), 1);
    assert_eq!(final_state.get::<i64, _>("collection_outboxes"), 1);
    assert_eq!(final_state.get::<i64, _>("run_outboxes"), 1);
    assert_eq!(final_state.get::<i64, _>("end_steps"), 0);
    assert_eq!(final_state.get::<i64, _>("reconciliations"), 0);
    assert_eq!(final_state.get::<i64, _>("cancel_audits"), 1);
    assert_eq!(final_state.get::<i64, _>("refusal_audits"), 1);
    assert_eq!(final_state.get::<i64, _>("data_limit_drain_audits"), 0);
    assert_eq!(
        effective_totals(&platform.api.db.pool, &receipt).await,
        receipt.preset.expected_totals()
    );
    assert!(
        source_witness(&platform.api.db.pool, receipt.tenant, receipt.run).await
            == receipt.source_witness,
        "closed source witness changed during cancellation-first compensation"
    );
    assert!(
        identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
        "cancellation compensation changed persisted worker authority"
    );
    assert!(
        public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_public_cancel,
        "final scheduling changed the original public cancellation"
    );
    assert!(
        cancellation_drain_fingerprint(&platform.api.db.pool, &receipt).await
            == frozen_cancellation_drain,
        "final scheduling changed the original cancellation drain"
    );
    assert!(
        verified_operation_receipt(&platform.api.db.pool, &receipt, pre_cancel_version).await
            == operation_receipt,
        "final scheduling changed the public operation receipt"
    );
    assert!(
        stored_report_receipt(&platform.api.db.pool, &receipt).await == original_report_receipt,
        "final scheduling changed the exact report receipt"
    );
    assert_cancellation_data_limit_audits(&platform.api.db.pool, &receipt, 1).await;
    fresh_verify(&platform.api, &receipt).await;
    if preset == FixturePreset::UnderLimitIncrement {
        let no_comp_output: bool = sqlx::query_scalar("SELECT output_digest IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='reserve~c' AND instance_path_nodes=ARRAY['c10']::text[] AND instance_path_indices=ARRAY[0]::integer[]")
            .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("real cancelled compensation output");
        assert!(
            no_comp_output
                && retained_output_reference_fingerprint(&platform.api.db.pool, &receipt).await
                    == retained_before,
            "cancelled success/refusal and real compensation retain no new output references"
        );
        let terminal = under_limit_execution_footprint(&platform.api.db.pool, &receipt).await;
        let request = worker.req(receipt.completion.clone());
        let replay = worker
            .client
            .complete_step(request)
            .await
            .expect("original report replay after terminal cancellation")
            .into_inner();
        assert!(
            replay.already_applied
                && replay.command_id == accepted.command_id
                && replay.step_status == "SUCCEEDED"
                && replay.run_status == "CANCELLING",
            "terminal CANCELLED replay returns original stored successful-report ACK"
        );
        assert!(
            under_limit_execution_footprint(&platform.api.db.pool, &receipt).await == terminal
                && stored_report_receipt(&platform.api.db.pool, &receipt).await
                    == original_report_receipt
                && public_cancellation_fingerprint(&platform.api.db.pool, &receipt).await
                    == frozen_public_cancel
                && cancellation_drain_fingerprint(&platform.api.db.pool, &receipt).await
                    == frozen_cancellation_drain
                && source_witness(&platform.api.db.pool, receipt.tenant, receipt.run).await
                    == receipt.source_witness
                && identity_witness(&platform.api.db.pool, &receipt).await == authority_before,
            "terminal report replay preserves complete execution, original stop/drain/receipt/source/authority"
        );
        assert_eq!(
            effective_totals(&platform.api.db.pool, &receipt).await,
            receipt.preset.expected_totals()
        );
        fresh_verify(&platform.api, &receipt).await;
        assert!(
            under_limit_execution_footprint(&platform.api.db.pool, &receipt).await == terminal,
            "cold verification after terminal original-ACK replay remains read-only"
        );
    }
}

fn cas_counter(name: &str, labels: &[(&str, &str)]) -> f64 {
    sagat_telemetry::Metrics::global()
        .render()
        .lines()
        .filter(|line| {
            line.starts_with(&format!("{name}{{"))
                && labels
                    .iter()
                    .all(|(key, value)| line.contains(&format!("{key}=\"{value}\"")))
        })
        .filter_map(|line| line.split_whitespace().last()?.parse::<f64>().ok())
        .sum()
}

fn cas_primitive_rejections() -> f64 {
    cas_counter(
        "workflow_primitive_rejections_total",
        &[("kind", "collection"), ("code", "RUN_DATA_LIMIT")],
    )
}

#[derive(Clone)]
struct CasBackend {
    pid: i32,
    started: String,
    role: String,
    database: String,
}

async fn cas_backend(connection: &mut sqlx::PgConnection) -> Result<CasBackend, &'static str> {
    let row=tokio::time::timeout(Duration::from_secs(3),sqlx::query(
        "SELECT pid,backend_start::text,usename::text,datname::text FROM pg_stat_activity WHERE pid=pg_backend_pid()")
        .fetch_one(connection)).await.map_err(|_|"backend identity deadline")?.map_err(|_|"backend identity query")?;
    Ok(CasBackend {
        pid: row.get("pid"),
        started: row.get("backend_start"),
        role: row.get("usename"),
        database: row.get("datname"),
    })
}

const CAS_DRAIN_LOCK: &str = "SELECT namespace_id,workflow_version_id,run_version,status,stop_reason,stop_code,paused FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE";

async fn cas_wait_report(
    observer: &mut sqlx::PgConnection,
    holder: &CasBackend,
    expected_role: &str,
    report: &tokio::task::JoinHandle<
        Result<sagat_api_grpc::pb::StepCommandResponse, tonic::Status>,
    >,
) -> Result<CasBackend, &'static str> {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let rows=sqlx::query("SELECT pid,backend_start::text,usename::text,datname::text FROM pg_stat_activity WHERE datname=$1 AND usename=$2 AND state='active' AND wait_event_type='Lock' AND query LIKE 'INSERT INTO telemetry_pending %' AND $3=ANY(pg_blocking_pids(pid))")
                .bind(&holder.database).bind(expected_role).bind(holder.pid).fetch_all(&mut *observer).await.map_err(|_|"report wait observation query")?;
            if rows.len()>1 {return Err("ambiguous report backend");}
            if let Some(row)=rows.first() {return Ok(CasBackend{pid:row.get("pid"),started:row.get("backend_start"),role:row.get("usename"),database:row.get("datname")});}
            if report.is_finished(){return Err("report ended before telemetry barrier");}
            tokio::task::yield_now().await;
        }
    }).await.map_err(|_|"report wait observation deadline")?
}

async fn cas_wait_native_drain(
    observer: &mut sqlx::PgConnection,
    holder: &CasBackend,
    report: &CasBackend,
    scheduler: &CasBackend,
) -> Result<(), &'static str> {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity s JOIN pg_stat_activity r ON r.pid=$2 WHERE s.pid=$1 AND s.backend_start::text=$3 AND r.backend_start::text=$4 AND s.datname=$5 AND r.datname=$5 AND s.usename=$6 AND r.usename=$6 AND s.state='active' AND r.state='active' AND s.wait_event_type='Lock' AND r.wait_event_type='Lock' AND s.query=$7 AND r.query LIKE 'INSERT INTO telemetry_pending %' AND r.pid=ANY(pg_blocking_pids(s.pid)) AND $8=ANY(pg_blocking_pids(r.pid)))")
                .bind(scheduler.pid).bind(report.pid).bind(&scheduler.started).bind(&report.started)
                .bind(&scheduler.database).bind(&scheduler.role).bind(CAS_DRAIN_LOCK).bind(holder.pid)
                .fetch_one(&mut *observer).await.map_err(|_|"native drain wait observation query")?;
            if blocked {return Ok(());}
            tokio::task::yield_now().await;
        }
    }).await.map_err(|_|"native drain wait observation deadline")?
}

async fn cas_idle(
    observer: &mut sqlx::PgConnection,
    backend: &CasBackend,
) -> Result<bool, &'static str> {
    let idle:bool=tokio::time::timeout(Duration::from_secs(3),sqlx::query_scalar(
        "SELECT NOT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND backend_start::text=$2 AND datname=$3 AND usename=$4 AND (xact_start IS NOT NULL OR state IS DISTINCT FROM 'idle'))")
        .bind(backend.pid).bind(&backend.started).bind(&backend.database).bind(&backend.role).fetch_one(observer))
        .await.map_err(|_|"backend idle deadline")?.map_err(|_|"backend idle query")?;
    Ok(idle)
}

async fn cas_terminate_owned(pool: &PgPool, backend: &CasBackend) -> bool {
    let result=tokio::time::timeout(Duration::from_secs(3),sqlx::query_scalar::<_,bool>(
        "SELECT coalesce(bool_and(pg_terminate_backend(pid)),true) FROM pg_stat_activity WHERE pid=$1 AND backend_start::text=$2 AND datname=$3 AND usename=$4 AND pid<>pg_backend_pid()")
        .bind(backend.pid).bind(&backend.started).bind(&backend.database).bind(&backend.role).fetch_one(pool)).await;
    matches!(result, Ok(Ok(true)))
}

async fn cas_wait_idle(observer: &mut sqlx::PgConnection, backend: &CasBackend) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if cas_idle(observer, backend).await? {
                    return Ok::<(), &'static str>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await,
        Ok(Ok(()))
    )
}

async fn cas_footprint_matches(
    pool: PgPool,
    receipt: Arc<Receipt>,
    expected: String,
) -> Result<(), &'static str> {
    let mut task =
        tokio::spawn(
            async move { under_limit_execution_footprint(&pool, &receipt).await == expected },
        );
    let result = tokio::time::timeout(Duration::from_secs(4), &mut task).await;
    match result {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err("committed footprint changed while report blocked"),
        Ok(Err(_)) => Err("footprint task failed"),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err("footprint task deadline")
        }
    }
}

async fn cas_join<T: Send + 'static>(handle: &mut tokio::task::JoinHandle<T>) -> Option<T> {
    match tokio::time::timeout(Duration::from_secs(10), &mut *handle).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(_)) => None,
        Err(_) => {
            handle.abort();
            let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
            None
        }
    }
}

async fn cas_close(pool: &PgPool) -> bool {
    tokio::time::timeout(Duration::from_secs(5), pool.close())
        .await
        .is_ok()
}

async fn submit_original_report_with_frontier_cas(
    api: &TestApi,
    receipt: Arc<Receipt>,
    worker: &mut Worker<'_>,
) -> sagat_api_grpc::pb::StepCommandResponse {
    use sagat_infra_postgres::BeginSafely as _;
    let role_url = if crate::support::as_runtime_role() {
        api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
    } else {
        api.db.url.clone()
    };
    let mut actor_config = PoolConfig::new(role_url);
    actor_config.max_connections = 1;
    actor_config.statement_timeout = Duration::from_secs(30);
    let scheduler_pool = tokio::time::timeout(Duration::from_secs(5), connect(&actor_config))
        .await
        .expect("CAS actor connection deadline")
        .expect("CAS actor pool");
    let mut scheduler_connection = scheduler_pool
        .acquire()
        .await
        .expect("CAS actor identity connection");
    let scheduler_backend = cas_backend(&mut scheduler_connection)
        .await
        .expect("CAS actor identity");
    assert!(
        scheduler_backend.role == receipt.producer_api_role
            && scheduler_backend.database == receipt.database_name,
        "dedicated scheduler uses actual selected API role and exact retained database"
    );
    drop(scheduler_connection);
    let store = PgRunStore::new(
        scheduler_pool.clone(),
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
    )
    .with_scoped_worker_protocol();
    let placement = Placement::new(receipt.scheduler_owner.clone());
    let keeper = Keeper::new(
        store.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(3_600),
        },
    );
    let round = keeper
        .round()
        .await
        .expect("normal pre-barrier Keeper renewal");
    assert!(
        round.replicas == 1
            && round.held == usize::from(SHARDS)
            && placement.owned().contains(Shard::of(receipt.run.as_uuid())),
        "actual original scheduler authority before the race"
    );
    let scheduler = Scheduler::new(
        store,
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);
    let mut inspection_config = PoolConfig::new(api.db.url.clone());
    inspection_config.max_connections = 2;
    inspection_config.statement_timeout = Duration::from_secs(3);
    let inspection_pool = tokio::time::timeout(Duration::from_secs(5), connect(&inspection_config))
        .await
        .expect("bounded owner inspection connection")
        .expect("owner inspection pool");
    // Pin owner cleanup C before any barrier; the other inspection connection serves witnesses.
    let mut cleanup_connection = inspection_pool
        .acquire()
        .await
        .expect("owner cleanup connection");
    let cleanup_backend = cas_backend(&mut cleanup_connection)
        .await
        .expect("owner cleanup identity");
    assert!(
        cleanup_backend.role != RUNTIME_ROLE && cleanup_backend.database == receipt.database_name,
        "owner-only holder cleanup visibility"
    );
    let mut holder_config = PoolConfig::new(api.db.url.clone());
    holder_config.max_connections = 1;
    holder_config.statement_timeout = Duration::from_secs(30);
    let holder_pool = tokio::time::timeout(Duration::from_secs(5), connect(&holder_config))
        .await
        .expect("holder connection deadline")
        .expect("holder pool");
    let mut warm = holder_pool
        .acquire()
        .await
        .expect("holder identity connection");
    let holder_backend = cas_backend(&mut warm).await.expect("holder identity");
    drop(warm);
    let mut observer = tokio::time::timeout(Duration::from_secs(5), api.api_pool.acquire())
        .await
        .expect("observer connection deadline")
        .expect("pinned observer connection");
    let observer_backend = cas_backend(&mut observer).await.expect("observer identity");
    assert!(
        observer_backend.role == receipt.producer_api_role
            && observer_backend.database == receipt.database_name
            && observer_backend.pid != holder_backend.pid
            && observer_backend.pid != scheduler_backend.pid
            && cleanup_backend.pid != holder_backend.pid
            && cleanup_backend.pid != scheduler_backend.pid,
        "independent pinned observer, owner holder/cleanup and scheduler sessions"
    );
    let baseline=sqlx::query("SELECT r.run_version,t.head_seq,(SELECT count(*) FROM chain_segments c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id) AS chains FROM runs r JOIN telemetry_pending t ON t.tenant_id=r.tenant_id AND t.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2")
        .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).fetch_one(&inspection_pool).await.expect("actual old revision/telemetry/unchained source");
    let v0 = baseline.get::<i64, _>("run_version");
    assert!(
        baseline.get::<i64, _>("head_seq") == v0 && baseline.get::<i64, _>("chains") == 0,
        "existing original telemetry row and no chain prefix"
    );
    let original_outputs = output_reference_fingerprint(&inspection_pool, &receipt).await;
    assert_initial_drain_first_state(&inspection_pool, &receipt, &original_outputs).await;
    assert_cas_initial_frontier(api, &inspection_pool, &receipt, v0).await;
    let before = under_limit_execution_footprint(&inspection_pool, &receipt).await;
    let refusal_before = cas_counter(
        "worker_scoped_output_refused_total",
        &[("reason", "run_data_limit")],
    );
    let rejections_before = cas_primitive_rejections();
    assert_live_target(&inspection_pool, &receipt, false).await;
    assert_eq!(
        effective_totals(&inspection_pool, &receipt).await,
        FixturePreset::Ordinary.expected_totals()
    );
    let mut holder = tokio::time::timeout(Duration::from_secs(5), holder_pool.begin_safely())
        .await
        .expect("cancel-safe holder begin deadline")
        .expect("cancel-safe holder begin");
    let mut report_task = None;
    let mut scheduler_task = None;
    let mut report_backend = None;
    let mut scheduler = Some(scheduler);
    // Only this proof is cancelled at90s; holder and actor handles live outside it for cleanup.
    let proof=tokio::time::timeout(Duration::from_secs(90),async {
        let actual_holder=cas_backend(&mut holder).await?;
        if actual_holder.pid!=holder_backend.pid || actual_holder.started!=holder_backend.started
            || actual_holder.role!=holder_backend.role || actual_holder.database!=holder_backend.database {
            return Err("holder connection identity changed before locking");
        }
        let existing=tokio::time::timeout(Duration::from_secs(5),sqlx::query_scalar::<_,i64>(
            "SELECT head_seq FROM telemetry_pending WHERE tenant_id=$1 AND run_id=$2 FOR UPDATE")
            .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).fetch_one(&mut *holder))
            .await.map_err(|_|"telemetry holder lock deadline")?.map_err(|_|"telemetry holder lock query")?;
        if existing!=v0 {return Err("telemetry changed before barrier");}
        let mut client=worker.client.clone();let request=worker.req(receipt.completion.clone());
        report_task=Some(tokio::spawn(async move{client.complete_step(request).await.map(tonic::Response::into_inner)}));
        let reporter=cas_wait_report(&mut observer,&holder_backend,&receipt.producer_api_role,
            report_task.as_ref().ok_or("missing report actor")?).await?;
        if reporter.pid==observer_backend.pid || reporter.pid==scheduler_backend.pid || reporter.pid==holder_backend.pid {
            return Err("report actor is not distinct");
        }
        report_backend=Some(reporter.clone());
        cas_footprint_matches(inspection_pool.clone(),receipt.clone(),before.clone()).await?;
        let actor=scheduler.take().ok_or("missing scheduler actor")?;
        let tenant=receipt.tenant;let run=receipt.run;
        let owner=ExecutionInstanceId::root(NodeId::parse("c10").map_err(|_|"fixed owner identity")?);
        scheduler_task=Some(tokio::spawn(async move{actor.advance_collection(tenant,run,&owner).await}));
        cas_wait_native_drain(&mut observer,&holder_backend,&reporter,&scheduler_backend).await?;
        cas_footprint_matches(inspection_pool.clone(),receipt.clone(),before.clone()).await?;
        cas_wait_native_drain(&mut observer,&holder_backend,&reporter,&scheduler_backend).await?;
        Ok::<(),&'static str>(())
    }).await.unwrap_or(Err("CAS proof budget exceeded"));
    // No assertions are surfaced while a row barrier remains held.
    let rolled_back = matches!(
        tokio::time::timeout(Duration::from_secs(5), holder.rollback()).await,
        Ok(Ok(()))
    );
    let mut cleanup_ok = rolled_back;
    let mut emergency_release_verified = true;
    if !rolled_back {
        let _ = tokio::time::timeout(Duration::from_secs(1), holder_pool.close()).await;
        cleanup_ok &= cas_terminate_owned(&inspection_pool, &holder_backend).await;
    }
    let holder_idle = cas_wait_idle(&mut cleanup_connection, &holder_backend).await;
    let mut holder_release_verified = holder_idle;
    cleanup_ok &= holder_idle;
    if !holder_idle {
        let terminated = cas_terminate_owned(&inspection_pool, &holder_backend).await;
        let released = cas_wait_idle(&mut cleanup_connection, &holder_backend).await;
        holder_release_verified = terminated && released;
        cleanup_ok = false;
    }
    if !holder_release_verified {
        // Never await normally blocked operations when holder release could not be proved.
        // Abort/reap their client tasks; the server-state checks below remain mandatory.
        if let Some(ref task) = report_task {
            task.abort();
        }
        if let Some(ref task) = scheduler_task {
            task.abort();
        }
    }
    let report_result = if let Some(ref mut task) = report_task {
        cas_join(task).await
    } else {
        None
    };
    let scheduler_result = if let Some(ref mut task) = scheduler_task {
        cas_join(task).await
    } else {
        None
    };
    if let Some(ref reporter) = report_backend
        && !cas_wait_idle(&mut observer, reporter).await
    {
        let terminated = cas_terminate_owned(&inspection_pool, reporter).await;
        let released = cas_wait_idle(&mut observer, reporter).await;
        emergency_release_verified &= terminated && released;
        cleanup_ok = false;
    }
    if !cas_wait_idle(&mut observer, &scheduler_backend).await {
        let terminated = cas_terminate_owned(&inspection_pool, &scheduler_backend).await;
        let released = cas_wait_idle(&mut observer, &scheduler_backend).await;
        emergency_release_verified &= terminated && released;
        cleanup_ok = false;
    }
    cleanup_ok &= emergency_release_verified;
    drop(observer); // Never close the API pool while its observer is pinned.
    drop(scheduler);
    drop(keeper);
    cleanup_ok &= cas_close(&scheduler_pool).await;
    cleanup_ok &= cas_close(&holder_pool).await;
    if proof.is_err()
        || report_result.as_ref().is_none_or(|result| result.is_err())
        || scheduler_result.is_none()
        || !cleanup_ok
    {
        // With an unidentified reporter, closing the whole owned API pool is the server cleanup proof.
        cleanup_ok &= tokio::time::timeout(Duration::from_secs(35), api.api_pool.close())
            .await
            .is_ok();
        cleanup_ok &= cas_wait_role_quiescent(
            &mut cleanup_connection,
            &receipt.database_name,
            &receipt.producer_api_role,
        )
        .await;
    }
    drop(cleanup_connection);
    cleanup_ok &= cas_close(&inspection_pool).await;
    assert!(cleanup_ok, "CAS actors/holder did not close cleanly");
    assert!(
        proof.is_ok(),
        "held telemetry/native frontier proof failed: {}",
        proof.err().unwrap_or("none")
    );
    let accepted = match report_result {
        Some(Ok(ack)) => ack,
        _ => panic!("original report did not deliver its accepted ACK"),
    };
    let current = match scheduler_result {
        Some(Err(sagat_scheduler::CollectionAdvanceError::Primitive(
            PrimitiveStoreError::RevisionConflict { current },
        ))) => current,
        _ => panic!("stale public frontier did not return exact native RevisionConflict"),
    };
    let v1: i64 = sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .fetch_one(&api.db.pool)
        .await
        .expect("independent post-report revision");
    assert!(
        v1 > v0
            && current == u64::try_from(v1).expect("positive committed revision")
            && accepted.command_id == format!("{}:{v1}", receipt.run),
        "native CAS current revision equals real committed report and ACK"
    );
    assert!(
        cas_counter(
            "worker_scoped_output_refused_total",
            &[("reason", "run_data_limit")]
        ) == refusal_before + 1.0
            && cas_primitive_rejections() == rejections_before,
        "report increments once; rejected stale drain emits no rejection metric"
    );
    println!(
        "observed_report_frontier_cas=true holder_pid={} report_pid={} scheduler_pid={} observer_pid={} original_revision={} committed_revision={}",
        holder_backend.pid,
        report_backend.expect("observed original report actor").pid,
        scheduler_backend.pid,
        observer_backend.pid,
        v0,
        v1
    );
    accepted
}

async fn assert_cas_initial_frontier(api: &TestApi, pool: &PgPool, receipt: &Receipt, head: i64) {
    use sagat_application::scope_frontier::ScopeFrontierStore as _;
    let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("selected owner"));
    let frontier = api
        .executions
        .collection_frontier(receipt.tenant, receipt.run, &owner)
        .await
        .expect("actual initial CAS frontier")
        .expect("selected CAS scope");
    let inventory = live_frontier_inventory(pool, receipt).await;
    assert!(
        frontier.phase == sagat_domain::execution::CollectionPhase::Open
            && frontier.run.version == u64::try_from(head).expect("positive original revision")
            && frontier.scope_id.as_uuid().to_string() == inventory["scope_id"]
            && frontier.run.stop_reason.is_none()
            && frontier.run.stop_code.is_none()
            && frontier.run.effective_output_bytes == 4_200_020
            && frontier.item_count == 1
            && frontier.next_admission == 1
            && frontier.completed == 0
            && frontier.max_concurrency == 1
            && frontier.active.len() == 1,
        "actual v0 OPEN/no-stop over-budget CAS source with no admission capacity"
    );
    let member = &frontier.active[0];
    assert!(
        member.index == 0
            && member.first_failure().is_none()
            && member.pending_return(&frontier.run.definition).is_none()
            && member.entry.node().as_str() == "reserve"
            && member.entry.path().segments().len() == 1
            && member.entry.path().segments()[0].scope_node().as_str() == "c10"
            && member.entry.path().segments()[0].iteration_index() == 0
            && member.steps.len() == 1
            && member.steps.get(&member.entry).is_some_and(|s| s.status()
                == sagat_domain::execution::StepStatus::Leased
                && !s.cancel_requested()),
        "one original live uncancelled member without failure or pending Return"
    );
}

async fn cas_report_stop(pool: &PgPool, receipt: &Receipt) -> String {
    let stop:Value=sqlx::query_scalar("SELECT jsonb_build_object('reason',r.stop_reason,'code',r.stop_code,'trace',r.trace_id,'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.event_type='run.stop_requested')) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2")
        .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).fetch_one(pool).await.expect("complete original report stop identity");
    assert!(
        stop["reason"] == "FAILED"
            && stop["code"] == "RUN_DATA_LIMIT"
            && stop["events"]
                .as_array()
                .is_some_and(|events| events.len() == 1
                    && events[0]["detail"] == json!({"reason":"FAILED","code":"RUN_DATA_LIMIT"})
                    && events[0]["trace_id"] == stop["trace"]),
        "exact single original report-created stop and original trace"
    );
    sha(&stop)
}

async fn assert_cas_fresh_drain_boundary(pool: &PgPool, receipt: &Receipt) {
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs r JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND s.owner_path_nodes='{}'::text[] JOIN run_events stop ON stop.tenant_id=r.tenant_id AND stop.run_id=r.id AND stop.event_type='run.stop_requested' JOIN run_events completed ON completed.tenant_id=r.tenant_id AND completed.run_id=r.id AND completed.event_type='step.completed' AND completed.node='reserve' AND completed.instance_path_nodes=ARRAY['c10']::text[] AND completed.instance_path_indices=ARRAY[0]::integer[] JOIN run_events drain ON drain.tenant_id=r.tenant_id AND drain.run_id=r.id AND drain.seq=s.drain_sequence AND drain.event_type='collection.drain_started' JOIN event_outbox hint ON hint.id=s.drain_outbox_id AND hint.tenant_id=r.tenant_id AND hint.aggregate_id=r.id JOIN audit_records audit ON audit.tenant_id=r.tenant_id AND audit.resource_id=r.id::text AND audit.action='run.data_limit_exceeded' AND audit.reason='stopped_for_run_data_limit' WHERE r.tenant_id=$1 AND r.id=$2 AND stop.seq<completed.seq AND completed.seq<drain.seq AND stop.trace_id IS NOT DISTINCT FROM r.trace_id AND completed.trace_id IS NOT DISTINCT FROM r.trace_id AND drain.trace_id IS NOT DISTINCT FROM r.trace_id AND audit.trace_id IS NOT DISTINCT FROM r.trace_id AND hint.payload->>'traceId' IS NOT DISTINCT FROM r.trace_id AND hint.event_type='collection.drain.started' AND hint.payload->>'reason'='FAILED')")
        .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).fetch_one(pool).await.expect("original stop then completion then fresh drain trace/order");
    assert!(
        valid,
        "fresh stopped-limit drain follows original completion and retains original trace in event/audit/hint"
    );
}

async fn cas_wait_role_quiescent(
    owner: &mut sqlx::PgConnection,
    database: &str,
    role: &str,
) -> bool {
    matches!(tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let quiet:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=$1 AND usename=$2 AND backend_type='client backend' AND pid<>pg_backend_pid() AND xact_start IS NOT NULL)")
                .bind(database).bind(role).fetch_one(&mut *owner).await.map_err(|_|"final owned-role server observation")?;
            if quiet {return Ok::<(),&'static str>(());}
            tokio::task::yield_now().await;
        }
    }).await,Ok(Ok(())))
}
