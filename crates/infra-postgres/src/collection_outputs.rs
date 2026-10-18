//! Verified legacy collection outputs used by the full-Run expression view and byte guard.
//!
//! Old collection finishers retained the aggregate on the successful root scope without retaining
//! the same logical output on its owner step. This reader recognizes only that exact historical
//! shape. It never writes a projection, follows a continuation, or broadens scoped worker data.

use std::collections::BTreeMap;

use sagat_application::{
    error::StoreError,
    execution::{RunInfo, instance_step_id},
};
use sagat_domain::{
    execution::{CollectionOutcome, ExecutionInstanceId, RunEvent},
    workflow::{Digest, MAX_NODES, NodeId, NodeKind, WorkflowDefinition},
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    event_codec,
    identity::{col, corrupt, store_error},
};

const SUCCESSFUL_ROOT_COLLECTIONS: &str = r#"
SELECT scope.owner_node, scope.item_count, scope.next_admission_index,
       scope.admitted_count, scope.completed_count, scope.active_count,
       (scope.kind = 'COLLECTION') AS collection_kind,
       (scope.drain_kind IS NULL AND scope.drain_iteration IS NULL
        AND scope.drain_failure_code IS NULL AND scope.drain_sequence IS NULL
        AND scope.drain_outbox_id IS NULL) AS no_drain,
       scope.result_digest, scope.finish_sequence, scope.finish_outbox_id,
       step.step_id, step.namespace_id AS step_namespace_id,
       (step.kind = 'FORWARD') AS forward_step,
       (step.node_kind IN ('forEach', 'legacyControl')) AS foreach_step,
       (step.status = 'SUCCEEDED') AS succeeded_step, step.routed,
       (step.waiting_kind IS NULL AND step.lease_token_hash IS NULL
        AND step.lease_expires_at IS NULL AND step.attempt_deadline_at IS NULL
        AND step.leased_by IS NULL AND step.next_attempt_at IS NULL) AS no_owner_work,
       step.output_digest,
       payload.digest IS NOT NULL AS payload_present,
       octet_length(payload.content)::bigint AS payload_bytes
FROM runs run
JOIN iteration_scopes scope
  ON scope.tenant_id = run.tenant_id AND scope.run_id = run.id
LEFT JOIN steps step
  ON step.tenant_id = scope.tenant_id AND step.run_id = scope.run_id
 AND step.instance_path_nodes = '{}'::text[]
 AND step.instance_path_indices = '{}'::integer[]
 AND step.node = scope.owner_node
LEFT JOIN payloads payload
  ON payload.tenant_id = scope.tenant_id AND payload.digest = scope.result_digest
WHERE run.tenant_id = $1 AND run.id = $2 AND run.namespace_id = $3
  AND run.workflow_version_id = $4
  AND scope.namespace_id = $3
  AND scope.owner_path_nodes = '{}'::text[]
  AND scope.owner_path_indices = '{}'::integer[]
  AND scope.lifecycle = 'SUCCEEDED'
ORDER BY scope.owner_node
LIMIT 10001
"#;

const JOURNAL_METADATA: &str = r#"
WITH source AS (
 SELECT seq, event_type, node, instance_id, instance_path_nodes, instance_path_indices,
        CASE WHEN event_type IN ('collection.finished', 'step.routed')
             THEN detail ELSE '{}'::jsonb END AS d
 FROM run_events
 WHERE tenant_id = $1 AND run_id = $2 AND seq = ANY($3::bigint[])
)
SELECT seq,
       CASE WHEN octet_length(event_type) <= 64 THEN event_type END AS event_type,
       CASE WHEN octet_length(node) <= 64 THEN node END AS node,
       instance_id, instance_path_nodes, instance_path_indices,
       CASE WHEN jsonb_typeof(d->'outcome') = 'string'
                  AND octet_length(d->>'outcome') <= 16
            THEN d->>'outcome' END AS outcome,
       CASE WHEN jsonb_typeof(d->'result_digest') = 'string'
                  AND octet_length(d->>'result_digest') <= 64
            THEN d->>'result_digest' END AS result_digest,
       CASE WHEN jsonb_typeof(d->'activated') = 'array'
            THEN jsonb_array_length(d->'activated') END AS activated_count,
       CASE WHEN jsonb_typeof(d#>'{activated,0}') = 'string'
                  AND octet_length(d#>>'{activated,0}') <= 64
            THEN d#>>'{activated,0}' END AS activated_node
FROM source
ORDER BY seq
"#;

#[derive(Debug)]
struct CollectionOwner {
    instance: ExecutionInstanceId,
    digest: Digest,
    bytes: u64,
    missing_output: bool,
}

/// One metadata-only snapshot of the stored ledger and verified successful collection owners.
///
/// Keeping the candidate identities alongside their scalar lengths lets callers enforce the Run
/// limit before `hydrate_missing` transfers aggregate bodies.
#[derive(Debug)]
pub(crate) struct EffectiveOutputBytes {
    ledger: u64,
    collections: Vec<CollectionOwner>,
    total: u64,
}

impl EffectiveOutputBytes {
    pub(crate) const fn ledger(&self) -> u64 {
        self.ledger
    }

    pub(crate) fn legacy_bytes(&self) -> Result<u64, StoreError> {
        self.total
            .checked_sub(self.ledger)
            .ok_or_else(|| corrupt("effective output byte total"))
    }

    pub(crate) const fn total(&self) -> u64 {
        self.total
    }
}

#[derive(Debug)]
struct Candidate {
    owner: NodeId,
    successor: NodeId,
    digest: Digest,
    finish_sequence: u64,
    output_present: bool,
    payload_bytes: u64,
}

#[derive(Debug)]
struct JournalEvidence {
    instance_id: Option<Uuid>,
    path_nodes: Option<Vec<String>>,
    path_indices: Option<Vec<i32>>,
    event: Option<RunEvent>,
}

/// Reads and validates the effective logical output total without transferring aggregate bodies.
pub(crate) async fn inspect(
    conn: &mut PgConnection,
    info: &RunInfo,
    definition: &WorkflowDefinition,
) -> Result<EffectiveOutputBytes, StoreError> {
    // A scope namespace is not part of the scope-to-Run foreign key. Detect a contradictory root
    // success before the qualified candidate query can hide it and before touching payload rows.
    let contradictory_namespace: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM runs run \
           JOIN iteration_scopes scope \
             ON scope.tenant_id = run.tenant_id AND scope.run_id = run.id \
           WHERE run.tenant_id = $1 AND run.id = $2 AND run.namespace_id = $3 \
             AND run.workflow_version_id = $4 \
             AND scope.owner_path_nodes = '{}'::text[] \
             AND scope.owner_path_indices = '{}'::integer[] \
             AND scope.lifecycle = 'SUCCEEDED' \
             AND scope.namespace_id <> run.namespace_id)",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(info.run_id.as_uuid())
    .bind(info.namespace_id.as_uuid())
    .bind(info.version_id.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    if contradictory_namespace {
        return Err(corrupt("successful collection namespace"));
    }

    let ledger: Option<i64> = sqlx::query_scalar(
        "SELECT usage.logical_output_bytes FROM runs run \
         LEFT JOIN run_instance_usage usage \
           ON usage.tenant_id = run.tenant_id AND usage.run_id = run.id \
         WHERE run.tenant_id = $1 AND run.id = $2 AND run.namespace_id = $3 \
           AND run.workflow_version_id = $4",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(info.run_id.as_uuid())
    .bind(info.namespace_id.as_uuid())
    .bind(info.version_id.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?
    .flatten();
    let ledger = u64::try_from(ledger.ok_or_else(|| corrupt("run output byte ledger"))?)
        .map_err(|_| corrupt("run output byte ledger"))?;

    let rows = sqlx::query(SUCCESSFUL_ROOT_COLLECTIONS)
        .bind(info.tenant_id.as_uuid())
        .bind(info.run_id.as_uuid())
        .bind(info.namespace_id.as_uuid())
        .bind(info.version_id.as_uuid())
        .fetch_all(&mut *conn)
        .await
        .map_err(store_error)?;
    if rows.len() > MAX_NODES {
        return Err(corrupt("root collection output count"));
    }

    let mut candidates = Vec::with_capacity(rows.len());
    for row in &rows {
        let owner = NodeId::parse_stored(&col::<String>(row, "owner_node")?)
            .map_err(|_| corrupt("collection output owner"))?;
        let declared = definition
            .node(&owner)
            .ok_or_else(|| corrupt("collection output owner"))?;
        if !matches!(declared.kind(), NodeKind::ForEach { .. }) {
            return Err(corrupt("collection output owner"));
        }
        let successor = declared
            .next()
            .cloned()
            .ok_or_else(|| corrupt("collection output successor"))?;
        let instance = ExecutionInstanceId::root(owner.clone());

        let item_count: i32 = col(row, "item_count")?;
        if item_count < 0
            || col::<i32>(row, "next_admission_index")? != item_count
            || col::<i32>(row, "admitted_count")? != item_count
            || col::<i32>(row, "completed_count")? != item_count
            || col::<i32>(row, "active_count")? != 0
            || !col::<bool>(row, "collection_kind")?
            || !col::<bool>(row, "no_drain")?
        {
            return Err(corrupt("successful collection output metadata"));
        }
        let finish_sequence = u64::try_from(
            col::<Option<i64>>(row, "finish_sequence")?
                .ok_or_else(|| corrupt("collection finish sequence"))?,
        )
        .map_err(|_| corrupt("collection finish sequence"))?;
        if finish_sequence == 0
            || col::<Option<Uuid>>(row, "finish_outbox_id")?.is_none()
            || col::<Option<Uuid>>(row, "step_id")?
                != Some(instance_step_id(info.version_id, info.run_id, &instance))
            || col::<Option<Uuid>>(row, "step_namespace_id")? != Some(info.namespace_id.as_uuid())
            || col::<Option<bool>>(row, "forward_step")? != Some(true)
            || col::<Option<bool>>(row, "foreach_step")? != Some(true)
            || col::<Option<bool>>(row, "succeeded_step")? != Some(true)
            || col::<Option<bool>>(row, "routed")? != Some(true)
            || col::<Option<bool>>(row, "no_owner_work")? != Some(true)
        {
            return Err(corrupt("successful collection output owner"));
        }

        let digest = stored_digest(
            col::<Option<Vec<u8>>>(row, "result_digest")?
                .ok_or_else(|| corrupt("collection output digest"))?,
            "collection output digest",
        )?;
        if !col::<bool>(row, "payload_present")? {
            return Err(corrupt("collection output payload"));
        }
        let payload_bytes = u64::try_from(
            col::<Option<i64>>(row, "payload_bytes")?
                .ok_or_else(|| corrupt("collection output payload length"))?,
        )
        .map_err(|_| corrupt("collection output payload length"))?;
        let output_present = match col::<Option<Vec<u8>>>(row, "output_digest")? {
            Some(output) => {
                if stored_digest(output, "collection owner output digest")? != digest {
                    return Err(corrupt("collection owner output digest"));
                }
                true
            }
            None => false,
        };
        candidates.push(Candidate {
            owner,
            successor,
            digest,
            finish_sequence,
            output_present,
            payload_bytes,
        });
    }
    verify_journal(conn, info, &candidates).await?;

    let mut collections = Vec::with_capacity(candidates.len());
    let mut total = ledger;
    for candidate in candidates {
        if !candidate.output_present {
            total = total
                .checked_add(candidate.payload_bytes)
                .ok_or_else(|| corrupt("effective output byte total"))?;
        }
        collections.push(CollectionOwner {
            instance: ExecutionInstanceId::root(candidate.owner),
            digest: candidate.digest,
            bytes: candidate.payload_bytes,
            missing_output: !candidate.output_present,
        });
    }
    Ok(EffectiveOutputBytes {
        ledger,
        collections,
        total,
    })
}

async fn verify_journal(
    conn: &mut PgConnection,
    info: &RunInfo,
    candidates: &[Candidate],
) -> Result<(), StoreError> {
    let mut sequences = Vec::with_capacity(candidates.len().saturating_mul(2));
    for candidate in candidates {
        sequences.push(
            i64::try_from(candidate.finish_sequence)
                .map_err(|_| corrupt("collection finish sequence"))?,
        );
        sequences.push(
            i64::try_from(
                candidate
                    .finish_sequence
                    .checked_add(1)
                    .ok_or_else(|| corrupt("collection route sequence"))?,
            )
            .map_err(|_| corrupt("collection route sequence"))?,
        );
    }
    if sequences.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query(JOURNAL_METADATA)
        .bind(info.tenant_id.as_uuid())
        .bind(info.run_id.as_uuid())
        .bind(sequences)
        .fetch_all(&mut *conn)
        .await
        .map_err(store_error)?;
    if rows.len() != candidates.len().saturating_mul(2) {
        return Err(corrupt("collection output journal evidence"));
    }

    let mut evidence = BTreeMap::new();
    for row in &rows {
        let sequence = u64::try_from(col::<i64>(row, "seq")?)
            .map_err(|_| corrupt("collection output journal sequence"))?;
        let event_type: Option<String> = col(row, "event_type")?;
        let node: Option<String> = col(row, "node")?;
        let detail = match event_type.as_deref() {
            Some("collection.finished") => json!({
                "outcome": col::<Option<String>>(row, "outcome")?,
                "result_digest": col::<Option<String>>(row, "result_digest")?,
            }),
            Some("step.routed") => {
                let activated = if col::<Option<i32>>(row, "activated_count")? == Some(1) {
                    json!([col::<Option<String>>(row, "activated_node")?])
                } else {
                    Value::Null
                };
                json!({"activated": activated})
            }
            _ => Value::Null,
        };
        let record = JournalEvidence {
            instance_id: col(row, "instance_id")?,
            path_nodes: col(row, "instance_path_nodes")?,
            path_indices: col(row, "instance_path_indices")?,
            event: event_type
                .as_deref()
                .and_then(|name| event_codec::decode(name, node.as_deref(), &detail)),
        };
        if evidence.insert(sequence, record).is_some() {
            return Err(corrupt("collection output journal evidence"));
        }
    }

    for candidate in candidates {
        let owner = ExecutionInstanceId::root(candidate.owner.clone());
        let expected_id = Some(instance_step_id(info.version_id, info.run_id, &owner));
        let finish = evidence
            .get(&candidate.finish_sequence)
            .ok_or_else(|| corrupt("collection finish journal evidence"))?;
        if finish.instance_id != expected_id
            || !finish.path_nodes.as_ref().is_some_and(Vec::is_empty)
            || !finish.path_indices.as_ref().is_some_and(Vec::is_empty)
            || finish.event.as_ref()
                != Some(&RunEvent::CollectionFinished {
                    node: candidate.owner.clone(),
                    outcome: CollectionOutcome::Succeeded,
                    result_digest: Some(candidate.digest),
                })
        {
            return Err(corrupt("collection finish journal evidence"));
        }
        let route_sequence = candidate
            .finish_sequence
            .checked_add(1)
            .ok_or_else(|| corrupt("collection route sequence"))?;
        let route = evidence
            .get(&route_sequence)
            .ok_or_else(|| corrupt("collection route journal evidence"))?;
        if route.instance_id != expected_id
            || !route.path_nodes.as_ref().is_some_and(Vec::is_empty)
            || !route.path_indices.as_ref().is_some_and(Vec::is_empty)
            || route.event.as_ref()
                != Some(&RunEvent::StepRouted {
                    node: candidate.owner.clone(),
                    activated: vec![candidate.successor.clone()],
                })
        {
            return Err(corrupt("collection route journal evidence"));
        }
    }
    Ok(())
}

/// Verifies successful aggregate bodies and returns only those absent from their owner projections.
pub(crate) async fn hydrate_missing(
    conn: &mut PgConnection,
    info: &RunInfo,
    inspected: &EffectiveOutputBytes,
) -> Result<Vec<(ExecutionInstanceId, Vec<u8>)>, StoreError> {
    if inspected.collections.is_empty() {
        return Ok(Vec::new());
    }
    let owners = inspected
        .collections
        .iter()
        .map(|collection| collection.instance.node().as_str().to_owned())
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        "SELECT scope.owner_node, scope.result_digest, payload.content \
         FROM runs run \
         JOIN iteration_scopes scope \
           ON scope.tenant_id = run.tenant_id AND scope.run_id = run.id \
         LEFT JOIN payloads payload \
           ON payload.tenant_id = scope.tenant_id AND payload.digest = scope.result_digest \
         WHERE run.tenant_id = $1 AND run.id = $2 AND run.namespace_id = $3 \
           AND run.workflow_version_id = $4 AND scope.namespace_id = $3 \
           AND scope.owner_path_nodes = '{}'::text[] \
           AND scope.owner_path_indices = '{}'::integer[] \
           AND scope.lifecycle = 'SUCCEEDED' AND scope.owner_node = ANY($5::text[]) \
         ORDER BY scope.owner_node",
    )
    .bind(info.tenant_id.as_uuid())
    .bind(info.run_id.as_uuid())
    .bind(info.namespace_id.as_uuid())
    .bind(info.version_id.as_uuid())
    .bind(owners)
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    if rows.len() != inspected.collections.len() {
        return Err(corrupt("collection output payload"));
    }

    let mut bodies = BTreeMap::new();
    for row in &rows {
        let owner = NodeId::parse_stored(&col::<String>(row, "owner_node")?)
            .map_err(|_| corrupt("collection output owner"))?;
        let digest = stored_digest(
            col::<Option<Vec<u8>>>(row, "result_digest")?
                .ok_or_else(|| corrupt("collection output digest"))?,
            "collection output digest",
        )?;
        let content = col::<Option<Vec<u8>>>(row, "content")?
            .ok_or_else(|| corrupt("collection output payload"))?;
        if bodies.insert(owner, (digest, content)).is_some() {
            return Err(corrupt("collection output payload"));
        }
    }

    let mut outputs = Vec::new();
    for collection in &inspected.collections {
        let (digest, content) = bodies
            .remove(collection.instance.node())
            .ok_or_else(|| corrupt("collection output payload"))?;
        let actual = Digest::new(Sha256::digest(&content).into());
        if digest != collection.digest
            || actual != collection.digest
            || u64::try_from(content.len())
                .map_err(|_| corrupt("collection output payload length"))?
                != collection.bytes
            || serde_json::from_slice::<Value>(&content).is_err()
        {
            return Err(corrupt("collection output payload"));
        }
        if collection.missing_output {
            outputs.push((collection.instance.clone(), content));
        }
    }
    if !bodies.is_empty() {
        return Err(corrupt("collection output payload"));
    }
    Ok(outputs)
}

fn stored_digest(bytes: Vec<u8>, what: &'static str) -> Result<Digest, StoreError> {
    <[u8; 32]>::try_from(bytes)
        .map(Digest::new)
        .map_err(|_| corrupt(what))
}
