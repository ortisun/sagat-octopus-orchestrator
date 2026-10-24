//! Genuine immutable-old reconciliation handoff. Attachment only; no current report is sent.
#![allow(clippy::expect_used, clippy::panic)]

use crate::support::api::{Options, start_on_db};
use crate::support::worker::Platform;
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, attach_existing_db};
use sagat_application::entropy::Token;
use sagat_application::execution::DefinitionCache;
use sagat_application::scope_frontier::ScopeFrontierStore as _;
use sagat_application::worker::WorkerSettings;
use sagat_domain::execution::ExecutionInstanceId;
use sagat_domain::workflow::NodeId;
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PgPool, PgRunStore, PoolConfig, connect, migrate};
use sha2::{Digest as _, Sha256};
use std::os::unix::fs::PermissionsExt as _;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use prost::Message as _;
use sagat_api_grpc::pb::{CompleteStepRequest, TaskLease};
use sagat_domain::execution::RunId;
use sagat_domain::identity::TenantId;
use serde_json::Value;
use uuid::Uuid;

use crate::support::worker::Worker;

const FORMAT: &str = "q10c-applied-handoff-v1";
const OLD_COMMIT: &str = "6c44a0e49670cf19660792cd1518d241e6c99b3b";
const ANSWER: &str = r#"{"effect":"applied","output":{"hold":7}}"#;

struct Handoff {
    format: String,
    source_commit: String,
    database_name: String,
    producer_api_role: String,
    tenant_id: String,
    other_tenant_id: String,
    namespace_id: String,
    namespace_slug: String,
    worker_subject: String,
    tenant_alias: String,
    worker_id: String,
    session_token: String,
    run_id: String,
    selected_owner: String,
    scheduler_owner: String,
    task_lease_b64: String,
    forward_task_lease_b64: String,
    completion_request_b64: String,
    producer_projection_witness_sha256: String,
    closed_source_witness_sha256: String,
    producer_authority_witness_sha256: String,
}

struct Receipt {
    handoff: Handoff,
    tenant: TenantId,
    other_tenant: TenantId,
    namespace: Uuid,
    run: RunId,
    forward: TaskLease,
    lookup: TaskLease,
    completion: CompleteStepRequest,
}

fn decode_canonical<T: prost::Message + Default>(encoded: &str) -> Result<T, &'static str> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "invalid private protobuf encoding")?;
    let value = T::decode(bytes.as_slice()).map_err(|_| "invalid private protobuf")?;
    if value.encode_to_vec() != bytes {
        return Err("noncanonical private protobuf");
    }
    Ok(value)
}

fn exact_instance(lease: &TaskLease, node: &str) -> bool {
    lease.instance.as_ref().is_some_and(|instance| {
        instance.definition_node_id == node
            && instance.instance_path.len() == 1
            && instance.instance_path[0].scope_node_id == "c10"
            && instance.instance_path[0].iteration_index == 0
    })
}

impl Receipt {
    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut value: Value =
            serde_json::from_slice(bytes).map_err(|_| "invalid private handoff JSON")?;
        let object = value
            .as_object_mut()
            .ok_or("invalid Applied handoff object")?;
        if object.len() != 21 {
            return Err("invalid exact Applied handoff fields");
        }
        let mut take = |name: &str| {
            object
                .remove(name)
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or("invalid Applied handoff string field")
        };
        let handoff = Handoff {
            format: take("format")?,
            source_commit: take("source_commit")?,
            database_name: take("database_name")?,
            producer_api_role: take("producer_api_role")?,
            tenant_id: take("tenant_id")?,
            other_tenant_id: take("other_tenant_id")?,
            namespace_id: take("namespace_id")?,
            namespace_slug: take("namespace_slug")?,
            worker_subject: take("worker_subject")?,
            tenant_alias: take("tenant_alias")?,
            worker_id: take("worker_id")?,
            session_token: take("session_token")?,
            run_id: take("run_id")?,
            selected_owner: take("selected_owner")?,
            scheduler_owner: take("scheduler_owner")?,
            task_lease_b64: take("task_lease_b64")?,
            forward_task_lease_b64: take("forward_task_lease_b64")?,
            completion_request_b64: take("completion_request_b64")?,
            producer_projection_witness_sha256: take("producer_projection_witness_sha256")?,
            closed_source_witness_sha256: take("closed_source_witness_sha256")?,
            producer_authority_witness_sha256: take("producer_authority_witness_sha256")?,
        };
        if !object.is_empty() {
            return Err("unknown Applied handoff field");
        }
        let owned_name = handoff.database_name.strip_prefix("t_").is_some_and(|s| {
            s.len() == 32
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        });
        if handoff.format != FORMAT
            || handoff.source_commit != OLD_COMMIT
            || !owned_name
            || handoff.producer_api_role.is_empty()
            || handoff.namespace_slug != "payments"
            || handoff.tenant_alias != "acme"
            || handoff.selected_owner != "c10"
            || handoff.worker_subject != "q10c-applied-handoff-worker"
            || handoff.scheduler_owner != "scope-compensation-q10c-applied-handoff-v1"
            || handoff.session_token.is_empty()
        {
            return Err("unexpected Applied handoff identity");
        }
        for digest in [
            &handoff.producer_projection_witness_sha256,
            &handoff.closed_source_witness_sha256,
            &handoff.producer_authority_witness_sha256,
        ] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err("invalid Applied witness digest");
            }
        }
        let tenant = handoff.tenant_id.parse().map_err(|_| "invalid tenant")?;
        let other_tenant = handoff
            .other_tenant_id
            .parse()
            .map_err(|_| "invalid other tenant")?;
        let namespace = handoff
            .namespace_id
            .parse()
            .map_err(|_| "invalid namespace")?;
        let run = handoff.run_id.parse().map_err(|_| "invalid run")?;
        Uuid::parse_str(&handoff.worker_id).map_err(|_| "invalid original worker")?;
        let forward: TaskLease = decode_canonical(&handoff.forward_task_lease_b64)?;
        let lookup: TaskLease = decode_canonical(&handoff.task_lease_b64)?;
        let completion: CompleteStepRequest = decode_canonical(&handoff.completion_request_b64)?;
        if tenant == other_tenant
            || forward.run_id != handoff.run_id
            || lookup.run_id != handoff.run_id
            || forward.task_id != format!("{}/reserve/{}", handoff.run_id, forward.generation)
            || lookup.task_id != format!("{}/reserve~r/{}", handoff.run_id, lookup.generation)
            || forward.task_kind != "FORWARD"
            || forward.activity_name != "line.reserve"
            || !exact_instance(&forward, "reserve")
            || !forward.tenant_alias.is_empty()
            || forward.target_instance.is_some()
            || !forward.target_node.is_empty()
            || !forward.target_idempotency_key.is_empty()
            || forward.idempotency_key.is_empty()
            || forward.lease_token.is_empty()
            || lookup.task_kind != "RECONCILIATION"
            || lookup.activity_name != "line.lookup"
            || !exact_instance(&lookup, "reserve~r")
            || !lookup.tenant_alias.is_empty()
            || lookup.target_instance != forward.instance
            || lookup.target_node != "reserve"
            || lookup.target_idempotency_key != forward.idempotency_key
            || lookup.idempotency_key.is_empty()
            || lookup.idempotency_key == forward.idempotency_key
            || lookup.lease_token.is_empty()
            || lookup.step_id == forward.step_id
            || lookup.task_id == forward.task_id
            || completion.output_json != ANSWER
        {
            return Err("Applied lookup does not bind its original forward effect");
        }
        let mut expected = Worker::complete_message(&lookup, ANSWER);
        expected.idempotency_key = format!("q10c-applied-handoff-{}", lookup.task_id);
        if expected.encode_to_vec() != completion.encode_to_vec() {
            return Err("Applied completion differs from exact original lookup request");
        }
        Ok(Self {
            handoff,
            tenant,
            other_tenant,
            namespace,
            run,
            forward,
            lookup,
            completion,
        })
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires separate immutable-old Applied producer; see decision evidence"]
async fn old_applied_lookup_attaches_without_changing_original_authority() {
    let path = std::env::var_os("SAGAT_Q10C_ORDINARY_PHASE_RECEIPT")
        .map(std::path::PathBuf::from)
        .expect("private phased receipt path");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("receipt metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let bytes = std::fs::read(path).expect("private handoff");
    // Fixture-contract RED: ordinary handoff cannot stand in for a genuine old lookup.
    let probe: Value = serde_json::from_slice(&bytes).expect("handoff object");
    let lookup: TaskLease =
        decode_canonical(probe["task_lease_b64"].as_str().expect("original task"))
            .expect("original task protobuf");
    if lookup.task_kind == "FORWARD" {
        eprintln!("observed_ordinary_forward_not_lookup=true");
    }
    assert!(
        lookup.task_kind == "RECONCILIATION",
        "genuine old lookup prerequisite, not ordinary FORWARD"
    );
    let receipt = Receipt::parse(&bytes).expect("strict Applied handoff");
    reject_handoff_mutations(&bytes);
    attach_and_verify(receipt).await;
}

fn reject_handoff_mutations(bytes: &[u8]) {
    let original: Value = serde_json::from_slice(bytes).expect("actual old handoff");
    let reject = |value: Value, label: &str| {
        let encoded = serde_json::to_vec(&value).expect("mutated handoff encoding");
        assert!(
            Receipt::parse(&encoded).is_err(),
            "accepted tampered handoff: {label}"
        );
    };
    let mut wrong = original.clone();
    wrong["format"] = Value::String("q10c-ordinary-before-drain-v1".into());
    reject(wrong, "wrong format");
    let mut wrong = original.clone();
    wrong["unexpected"] = Value::Bool(true);
    reject(wrong, "unknown field");
    let mut wrong = original.clone();
    let mut wire = base64::engine::general_purpose::STANDARD
        .decode(
            original["task_lease_b64"]
                .as_str()
                .expect("lookup encoding"),
        )
        .expect("actual lookup bytes");
    // Unknown field 1000, varint 1: prost discards it, so decoding would silently change bytes.
    wire.extend_from_slice(&[0xc0, 0x3e, 0x01]);
    wrong["task_lease_b64"] = Value::String(base64::engine::general_purpose::STANDARD.encode(wire));
    reject(wrong, "noncanonical protobuf");
    let mut wrong = original.clone();
    let mut request: CompleteStepRequest = decode_canonical(
        original["completion_request_b64"]
            .as_str()
            .expect("completion encoding"),
    )
    .expect("actual completion");
    request.task_id.push_str("-other");
    wrong["completion_request_b64"] =
        Value::String(base64::engine::general_purpose::STANDARD.encode(request.encode_to_vec()));
    reject(wrong, "request no longer binds original lookup");
    let mut wrong = original.clone();
    let mut lookup: TaskLease = decode_canonical(
        original["task_lease_b64"]
            .as_str()
            .expect("lookup encoding"),
    )
    .expect("actual lookup");
    lookup.target_instance = lookup.instance.clone();
    wrong["task_lease_b64"] =
        Value::String(base64::engine::general_purpose::STANDARD.encode(lookup.encode_to_vec()));
    reject(wrong, "lookup targets itself");
    let mut wrong = original.clone();
    let mut lookup: TaskLease = decode_canonical(
        original["task_lease_b64"]
            .as_str()
            .expect("lookup encoding"),
    )
    .expect("actual lookup");
    lookup.target_idempotency_key = lookup.idempotency_key.clone();
    wrong["task_lease_b64"] =
        Value::String(base64::engine::general_purpose::STANDARD.encode(lookup.encode_to_vec()));
    reject(wrong, "lookup key replaces original effect key");
}

#[test]
fn parser_refuses_coherently_readdressed_lookup_task() {
    use sagat_api_grpc::pb::{ExecutionInstanceIdentity, ScopePathSegment};
    use serde_json::json;

    // Synthetic wire data only: this test makes no database or authenticated-execution claim.
    let run = Uuid::from_u128(1).to_string();
    let forward = TaskLease {
        task_id: format!("{run}/reserve/1"),
        run_id: run.clone(),
        step_id: Uuid::from_u128(6).to_string(),
        attempt: 1,
        generation: 1,
        lease_token: "synthetic-forward-token".into(),
        activity_name: "line.reserve".into(),
        idempotency_key: "synthetic-forward-effect".into(),
        task_kind: "FORWARD".into(),
        instance: Some(ExecutionInstanceIdentity {
            definition_node_id: "reserve".into(),
            instance_path: vec![ScopePathSegment {
                scope_node_id: "c10".into(),
                iteration_index: 0,
            }],
        }),
        ..TaskLease::default()
    };
    let mut lookup = TaskLease {
        task_id: format!("{run}/reserve~r/1"),
        run_id: run.clone(),
        step_id: Uuid::from_u128(7).to_string(),
        attempt: 1,
        generation: 1,
        lease_token: "synthetic-lookup-token".into(),
        activity_name: "line.lookup".into(),
        idempotency_key: "synthetic-lookup-effect".into(),
        task_kind: "RECONCILIATION".into(),
        instance: Some(ExecutionInstanceIdentity {
            definition_node_id: "reserve~r".into(),
            instance_path: vec![ScopePathSegment {
                scope_node_id: "c10".into(),
                iteration_index: 0,
            }],
        }),
        target_instance: forward.instance.clone(),
        target_node: "reserve".into(),
        target_idempotency_key: forward.idempotency_key.clone(),
        ..TaskLease::default()
    };
    let encode = |bytes: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut completion = Worker::complete_message(&lookup, ANSWER);
    completion.idempotency_key = format!("q10c-applied-handoff-{}", lookup.task_id);
    let mut value = json!({
        "format": FORMAT, "source_commit": OLD_COMMIT, "database_name": format!("t_{}", "0".repeat(32)),
        "producer_api_role": "postgres", "tenant_id": Uuid::from_u128(2).to_string(),
        "other_tenant_id": Uuid::from_u128(3).to_string(), "namespace_id": Uuid::from_u128(4).to_string(),
        "namespace_slug": "payments", "worker_subject": "q10c-applied-handoff-worker", "tenant_alias": "acme",
        "worker_id": Uuid::from_u128(5).to_string(), "session_token": "synthetic-parser-only",
        "run_id": run, "selected_owner": "c10", "scheduler_owner": "scope-compensation-q10c-applied-handoff-v1",
        "task_lease_b64": encode(lookup.encode_to_vec()), "forward_task_lease_b64": encode(forward.encode_to_vec()),
        "completion_request_b64": encode(completion.encode_to_vec()), "producer_projection_witness_sha256": "0".repeat(64),
        "closed_source_witness_sha256": "0".repeat(64), "producer_authority_witness_sha256": "0".repeat(64)
    });
    assert!(
        Receipt::parse(&serde_json::to_vec(&value).expect("synthetic wire control")).is_ok(),
        "valid synthetic wire control"
    );
    let original_value = value.clone();
    lookup.task_id = format!("{}/reserve~r/2", lookup.run_id);
    completion = Worker::complete_message(&lookup, ANSWER);
    completion.idempotency_key = format!("q10c-applied-handoff-{}", lookup.task_id);
    value["task_lease_b64"] = Value::String(encode(lookup.encode_to_vec()));
    value["completion_request_b64"] = Value::String(encode(completion.encode_to_vec()));
    assert!(
        Receipt::parse(&serde_json::to_vec(&value).expect("coherent address mutation")).is_err(),
        "coherently altered lookup/request task ID must be bound to actual generation"
    );
    let mut forward = forward;
    forward.task_id.push_str("-other");
    let mut value = original_value;
    value["forward_task_lease_b64"] = Value::String(encode(forward.encode_to_vec()));
    assert!(
        Receipt::parse(&serde_json::to_vec(&value).expect("forward address mutation")).is_err(),
        "forward task ID must bind its original address/fence too"
    );
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
        r#"SELECT jsonb_build_object(
 'tenants',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM tenants t WHERE t.id=ANY($1::uuid[])),
 'memberships',(SELECT jsonb_agg(to_jsonb(m) ORDER BY m.subject) FROM memberships m WHERE m.tenant_id=$2 AND m.subject=$3),
 'bindings',(SELECT jsonb_agg(to_jsonb(b) ORDER BY b.pattern) FROM worker_bindings b WHERE b.tenant_id=$2 AND b.namespace_id=$4 AND b.subject=$3),
 'bindingPools',(SELECT jsonb_agg(to_jsonb(p) ORDER BY p.pool) FROM worker_binding_pools p WHERE p.tenant_id=$2 AND p.namespace_id=$4 AND p.subject=$3),
 'session',(SELECT to_jsonb(w) FROM worker_sessions w WHERE w.tenant_id=$2 AND w.id=$5))"#,
    )
    .bind(vec![receipt.tenant.as_uuid(), receipt.other_tenant.as_uuid()])
    .bind(receipt.tenant.as_uuid())
    .bind(&receipt.handoff.worker_subject)
    .bind(receipt.namespace)
    .bind(Uuid::parse_str(&receipt.handoff.worker_id).expect("worker id"))
    .fetch_one(pool)
    .await
    .expect("durable worker authority witness");
    sha(&value)
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

const APPLIED_STATE_SQL: &str = r#"SELECT jsonb_build_object(
 'run', r.status='RUNNING' AND r.stop_reason IS NULL AND r.stop_code IS NULL AND r.result_digest IS NULL,
 'scope', s.lifecycle='OPEN' AND s.item_count=1 AND s.next_admission_index=1 AND s.admitted_count=1 AND s.completed_count=0 AND s.active_count=1 AND s.result_digest IS NULL AND s.drain_kind IS NULL AND s.drain_iteration IS NULL AND s.drain_sequence IS NULL,
 'member', m.lifecycle='ADMITTED' AND m.slot_held AND m.reserved_instances>=1 AND m.remaining_reservations>=1 AND m.result_digest IS NULL,
 'forward', f.step_id=$4 AND f.status='WAITING' AND f.waiting_kind='RECONCILIATION' AND f.failure_code='OUTCOME_UNKNOWN' AND f.attempt=$6 AND f.generation=$7 AND NOT f.cancel_requested AND f.output_digest IS NULL AND f.reported_lease_token_hash=$8,
 'lookup', q.step_id=$5 AND q.status='LEASED' AND q.attempt=$9 AND q.generation=$10 AND q.lease_token_hash=$11 AND q.leased_by=$12 AND q.lease_expires_at>clock_timestamp() AND NOT q.cancel_requested AND q.output_digest IS NULL AND q.reported_lease_token_hash IS NULL,
 'session', w.token_hash=$13 AND w.expires_at>clock_timestamp() AND w.namespace_id=$3 AND w.subject='q10c-applied-handoff-worker' AND w.accepted_protocol_minor=5,
 'only_forward_lookup', (SELECT count(*)=2 FROM steps x WHERE x.tenant_id=$1 AND x.run_id=$2 AND x.instance_path_nodes=ARRAY['c10']::text[] AND x.instance_path_indices=ARRAY[0]::integer[]),
 'no_done', NOT EXISTS(SELECT 1 FROM steps x WHERE x.tenant_id=$1 AND x.run_id=$2 AND x.node='done'),
 'request', (SELECT count(*)=1 AND bool_and(e.detail->>'cause'='OUTCOME_UNKNOWN' AND (e.detail->>'attempt')::int=$6 AND (e.detail->>'generation')::bigint=$7) FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.node='reserve' AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[] AND e.event_type='reconciliation.requested'),
 'no_outcome', NOT EXISTS(SELECT 1 FROM run_events e WHERE e.tenant_id=$1 AND e.run_id=$2 AND e.instance_path_nodes=ARRAY['c10']::text[] AND e.instance_path_indices=ARRAY[0]::integer[] AND e.event_type IN ('reconciliation.concluded','step.completed','step.failed','step.cancelled')),
 'no_checkpoint', NOT EXISTS(SELECT 1 FROM iteration_checkpoints k WHERE k.tenant_id=$1 AND k.run_id=$2 AND k.scope_id=s.scope_id),
 'no_saga', NOT EXISTS(SELECT 1 FROM scope_sagas z WHERE z.tenant_id=$1 AND z.run_id=$2),
 'no_certificate', NOT EXISTS(SELECT 1 FROM leaf_collection_success_certifications z WHERE z.tenant_id=$1 AND z.run_id=$2),
 'no_lookup_receipt', NOT EXISTS(SELECT 1 FROM command_dedupe z WHERE z.tenant_id=$1 AND z.scope LIKE concat('step:',$2::text,':%') AND z.idempotency_key=$14)
)
FROM runs r
JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND s.owner_path_nodes='{}'::text[] AND s.owner_path_indices='{}'::integer[]
JOIN collection_iterations m ON m.tenant_id=s.tenant_id AND m.run_id=s.run_id AND m.scope_id=s.scope_id AND m.iteration_index=0
JOIN steps f ON f.tenant_id=r.tenant_id AND f.run_id=r.id AND f.namespace_id=$3 AND f.node='reserve' AND f.instance_path_nodes=ARRAY['c10']::text[] AND f.instance_path_indices=ARRAY[0]::integer[]
JOIN steps q ON q.tenant_id=r.tenant_id AND q.run_id=r.id AND q.namespace_id=$3 AND q.node='reserve~r' AND q.instance_path_nodes=ARRAY['c10']::text[] AND q.instance_path_indices=ARRAY[0]::integer[]
JOIN worker_sessions w ON w.tenant_id=r.tenant_id AND w.id::text=q.leased_by
WHERE r.tenant_id=$1 AND r.id=$2
"#;

async fn assert_applied_state(pool: &PgPool, receipt: &Receipt) {
    let state: Value = sqlx::query_scalar(APPLIED_STATE_SQL)
        .bind(receipt.tenant.as_uuid())
        .bind(receipt.run.as_uuid())
        .bind(receipt.namespace)
        .bind(Uuid::parse_str(&receipt.forward.step_id).expect("forward step"))
        .bind(Uuid::parse_str(&receipt.lookup.step_id).expect("lookup step"))
        .bind(receipt.forward.attempt)
        .bind(receipt.forward.generation)
        .bind(Token::hash_of(&receipt.forward.lease_token))
        .bind(receipt.lookup.attempt)
        .bind(receipt.lookup.generation)
        .bind(Token::hash_of(&receipt.lookup.lease_token))
        .bind(&receipt.handoff.worker_id)
        .bind(Token::hash_of(&receipt.handoff.session_token))
        .bind(&receipt.completion.idempotency_key)
        .fetch_one(pool)
        .await
        .expect("original Applied-ready state");
    for (name, valid) in state.as_object().expect("boolean state proofs") {
        assert!(
            valid.as_bool() == Some(true),
            "current Applied-ready invariant: {name}"
        );
    }
    assert_eq!(
        effective_totals(pool, receipt).await,
        (2_100_000, 2_100_020, 4_200_020)
    );
}

async fn side_effect_witness(pool: &PgPool, receipt: &Receipt) -> String {
    let value: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'audits',(SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY a.created_at,a.id),'[]'::jsonb) FROM audit_records a WHERE a.tenant_id=$1),
      'receipts',(SELECT coalesce(jsonb_agg(to_jsonb(d) ORDER BY d.scope,d.idempotency_key),'[]'::jsonb) FROM command_dedupe d WHERE d.tenant_id=$1 AND (d.scope=concat('run.operate:',$2::text) OR d.scope LIKE concat('step:',$2::text,':%'))),
      'telemetry',(SELECT to_jsonb(t) FROM telemetry_pending t WHERE t.tenant_id=$1 AND t.run_id=$2),
      'sagas',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.owner_path_nodes,t.owner_path_indices,t.owner_node) FROM scope_sagas t WHERE t.tenant_id=$1 AND t.run_id=$2),
      'entries',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.owner_path_nodes,t.owner_path_indices,t.owner_node,t.iteration_index,t.node) FROM scope_saga_entries t WHERE t.tenant_id=$1 AND t.run_id=$2),
      'tenantRetained',(SELECT to_jsonb(t) FROM tenant_retained_usage t WHERE t.tenant_id=$1),
      'tenantIteration',(SELECT to_jsonb(t) FROM tenant_iteration_usage t WHERE t.tenant_id=$1),
      'iterationPolicy',(SELECT to_jsonb(t) FROM tenant_collection_policies t WHERE t.tenant_id=$1),
      'quotaCeilings',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.resource) FROM quota_ceilings t WHERE t.tenant_id=$1),
      'runActiveCounts',(SELECT jsonb_agg(to_jsonb(t) ORDER BY t.slot) FROM run_active_counts t WHERE t.tenant_id=$1 AND t.namespace_id=$3),
      'payloads',(SELECT count(*) FROM payloads p WHERE p.tenant_id=$1))"#)
        .bind(receipt.tenant.as_uuid()).bind(receipt.run.as_uuid()).bind(receipt.namespace).fetch_one(pool).await.expect("complete read-only side-effect witness");
    sha(&value)
}

fn process_counters() -> Vec<String> {
    let mut lines: Vec<_> = sagat_telemetry::Metrics::global()
        .render()
        .lines()
        .filter(|line| {
            [
                "reconciliations_total{",
                "worker_scoped_output_refused_total{",
                "workflow_primitive_rejections_total{",
            ]
            .iter()
            .any(|name| line.starts_with(name))
        })
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

async fn assert_unchanged(pool: &PgPool, receipt: &Receipt, side_effects: &str) {
    assert!(
        projection_witness(pool, receipt.tenant, receipt.run).await
            == receipt.handoff.producer_projection_witness_sha256,
        "complete original projection changed"
    );
    assert!(
        source_witness(pool, receipt.tenant, receipt.run).await
            == receipt.handoff.closed_source_witness_sha256,
        "complete closed source changed"
    );
    assert!(
        identity_witness(pool, receipt).await == receipt.handoff.producer_authority_witness_sha256,
        "original authority or exact persisted expiry changed"
    );
    assert!(
        side_effect_witness(pool, receipt).await == side_effects,
        "attachment/verification changed audit/receipt/telemetry/usage/saga footprint"
    );
    assert_applied_state(pool, receipt).await;
}

async fn attach_and_verify(receipt: Receipt) {
    let counters = process_counters();
    let admin = std::env::var("SAGAT_TEST_DATABASE_URL").expect("repository-owned admin URL");
    let db = attach_existing_db(&admin, &receipt.handoff.database_name).await;
    let actual_database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&db.pool)
        .await
        .expect("same retained database");
    assert!(actual_database == receipt.handoff.database_name);
    let side_effects = side_effect_witness(&db.pool, &receipt).await;
    assert_unchanged(&db.pool, &receipt, &side_effects).await;
    migrate(&db.pool)
        .await
        .expect("owner applies current migrations");
    assert_unchanged(&db.pool, &receipt, &side_effects).await;
    let api = start_on_db(
        db,
        Options {
            runtime_role: crate::support::as_runtime_role(),
            worker: WorkerSettings {
                lease_ttl: Duration::from_secs(3600),
                session_ttl: Duration::from_secs(3600),
                ..WorkerSettings::default()
            },
            ..Options::default()
        },
    )
    .await;
    let platform = Platform {
        api,
        tenant: receipt.tenant,
        other_tenant: receipt.other_tenant,
    };
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.api_pool)
        .await
        .expect("actual current API role");
    assert!(role == receipt.handoff.producer_api_role);
    if crate::support::as_runtime_role() {
        assert_eq!(role, RUNTIME_ROLE);
    } else {
        assert_ne!(role, RUNTIME_ROLE);
    }
    eprintln!(
        "q10c applied-handoff API roles: producer={}, consumer={role}",
        receipt.handoff.producer_api_role
    );
    assert_unchanged(&platform.api.db.pool, &receipt, &side_effects).await;
    let lookups_before = platform.api.authentication_lookups();
    let mut worker = platform
        .connect(
            &receipt.handoff.worker_subject,
            &receipt.handoff.tenant_alias,
        )
        .await;
    worker.worker_id = receipt.handoff.worker_id.clone();
    worker.session = receipt.handoff.session_token.clone();
    let signed = worker.req(receipt.completion.clone());
    assert!(
        signed.get_ref().encode_to_vec() == receipt.completion.encode_to_vec(),
        "fresh signed transport retains original request bytes"
    );
    // No RPC is sent: current acceptance/authentication belongs to the next bounded slice.
    assert_eq!(platform.api.authentication_lookups(), lookups_before);
    let url = if crate::support::as_runtime_role() {
        platform.api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
    } else {
        platform.api.db.url.clone()
    };
    let pool = connect(&PoolConfig::new(url))
        .await
        .expect("fresh role-correct read-only store pool");
    let store = PgRunStore::new(pool.clone(), DefinitionCache::new(Arc::new(JsonIrCodec), 8))
        .with_scoped_worker_protocol();
    let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("original owner"));
    let frontier = store
        .collection_frontier(receipt.tenant, receipt.run, &owner)
        .await
        .expect("native read-only frontier")
        .expect("original scope");
    assert_eq!(frontier.run.effective_output_bytes, 4_200_020);
    assert!(frontier.run.stop_reason.is_none() && frontier.run.stop_code.is_none());
    assert!(
        store.verify_run(receipt.tenant, receipt.run).await.is_ok(),
        "fresh cold journal projection replay"
    );
    drop(store);
    pool.close().await;
    assert_unchanged(&platform.api.db.pool, &receipt, &side_effects).await;
    assert_eq!(
        process_counters(),
        counters,
        "read-only attachment emits no reconciliation/refusal/primitive rejection"
    );
}
