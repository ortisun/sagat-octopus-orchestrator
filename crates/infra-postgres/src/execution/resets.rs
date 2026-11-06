//! Reset to a point (feature 018, US06, `FR-018-035`..`FR-018-037`): reading the journal of a source run as a
//! reset sees it, and writing its prefix under the run a reset created.
//!
//! The prefix is written by the same `persist` as every journal, so the steps and the sagas it projects are
//! the ones the platform projects; what it does not do is what the first time did. A row of the prefix is the
//! row the source stored (type, node, attempt, detail, who, when, which payload), marked `restored`.

use std::collections::BTreeSet;
use std::sync::Arc;

use sagat_application::execution::{Actor, Effects, RunTx as _};
use sagat_application::runs::{ResetSource, RestoredPrefix, SourceEvent};
use sagat_domain::execution::{AddressedRunEvent, RunEvent};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::{
    NodeId, PgRunTx, StoreError, TenantId, Uuid, col, corrupt, decode, event_instance_path,
    store_error,
};
use sagat_domain::execution::RunId;

fn internal(detail: &str) -> StoreError {
    StoreError::Internal {
        detail: detail.to_owned(),
    }
}

/// The canonical encoding of one row, which the digest of a prefix is taken over: the sequence, the type, the
/// node, the attempt, the detail (its keys in the order of the JSON value) and the digest of the payload,
/// each framed so that no two rows encode alike.
fn canonical_row(
    sequence: u64,
    event_type: &str,
    node: Option<&str>,
    attempt: Option<i32>,
    detail: &Value,
    payload: Option<&[u8]>,
) -> Result<Vec<u8>, StoreError> {
    let mut out = Vec::new();
    out.extend(sequence.to_be_bytes());
    let framed = |out: &mut Vec<u8>, bytes: &[u8]| {
        out.extend((bytes.len() as u64).to_be_bytes());
        out.extend(bytes);
    };
    framed(&mut out, event_type.as_bytes());
    match node {
        Some(node) => {
            out.push(1);
            framed(&mut out, node.as_bytes());
        }
        None => out.push(0),
    }
    match attempt {
        Some(attempt) => {
            out.push(1);
            out.extend(attempt.to_be_bytes());
        }
        None => out.push(0),
    }
    framed(
        &mut out,
        &serde_json::to_vec(detail).map_err(|_| corrupt("journal detail"))?,
    );
    match payload {
        Some(payload) => {
            out.push(1);
            framed(&mut out, payload);
        }
        None => out.push(0),
    }
    Ok(out)
}

/// The journal of `run`, every row that decodes in order and the type of the first that does not.
pub(super) async fn read_source(
    pool: &PgPool,
    tenant: TenantId,
    run: RunId,
) -> Result<Option<ResetSource>, StoreError> {
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM runs WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_optional(pool)
            .await
            .map_err(store_error)?;
    if exists.is_none() {
        return Ok(None);
    }
    let rows = sqlx::query(
        "SELECT seq, event_type, node, attempt, detail, payload_digest, actor, trace_id, occurred_at, \
                instance_path_nodes, instance_path_indices \
         FROM run_events WHERE tenant_id = $1 AND run_id = $2 ORDER BY seq",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    let mut source = ResetSource::default();
    for (index, row) in rows.iter().enumerate() {
        let seq: i64 = col(row, "seq")?;
        if u64::try_from(seq).ok() != u64::try_from(index + 1).ok() {
            return Err(corrupt("a gap in the journal"));
        }
        let event_type: String = col(row, "event_type")?;
        let node: Option<String> = col(row, "node")?;
        let attempt: Option<i32> = col(row, "attempt")?;
        let detail: Value = col(row, "detail")?;
        let payload: Option<Vec<u8>> = col(row, "payload_digest")?;
        let Some(event) = decode(&event_type, node.as_deref(), &detail) else {
            source.unknown_event_type = Some(event_type);
            break;
        };
        let addressed = AddressedRunEvent::in_path(event_instance_path(row)?, event)
            .map_err(|_| corrupt("journal instance path"))?;
        let sequence = u64::try_from(seq).map_err(|_| corrupt("sequence"))?;
        let canonical = canonical_row(
            sequence,
            &event_type,
            node.as_deref(),
            attempt,
            &detail,
            payload.as_deref(),
        )?;
        source.events.push(SourceEvent {
            sequence,
            event: addressed,
            event_type,
            node,
            attempt,
            detail,
            actor: col(row, "actor")?,
            trace_id: col(row, "trace_id")?,
            occurred_at: col(row, "occurred_at")?,
            payload_digest: payload,
            canonical,
        });
    }
    Ok(Some(source))
}

/// The detail of a restored row: the one the source stored, marked `restored`, and for the first row, where
/// the history came from (`reset_of`).
pub(super) fn restored_detail(
    row: &SourceEvent,
    prefix: &RestoredPrefix,
    offset: usize,
) -> Result<Value, StoreError> {
    let mut detail = row.detail.clone();
    let object = detail
        .as_object_mut()
        .ok_or_else(|| corrupt("a journal detail that is not an object"))?;
    object.insert("restored".into(), json!(true));
    if offset == 0 {
        object.insert(
            "reset_of".into(),
            json!({
                "source_run_id": prefix.source.to_string(),
                "source_sequence": prefix.sequence(),
            }),
        );
    }
    Ok(detail)
}

/// Writes the prefix under the run of `tx` (see [`sagat_application::execution::RunTx::restore_prefix`]).
pub(super) async fn restore(tx: &mut PgRunTx, prefix: &RestoredPrefix) -> Result<(), StoreError> {
    if tx.persisted != 0 || tx.state.version() != 0 {
        return Err(internal("a prefix is restored into an empty run"));
    }
    if prefix.events.is_empty() {
        return Err(internal(
            "a reset restores at least the start of its source",
        ));
    }
    if prefix.events.iter().any(|row| !row.event.path().is_root()) {
        return Err(internal(
            "this version restores only the events of the root of a run",
        ));
    }
    for row in &prefix.events {
        tx.state
            .apply_addressed(&row.event)
            .map_err(|error| StoreError::Internal {
                detail: format!("the restored prefix does not replay: {error}"),
            })?;
    }
    // The outputs the prefix names, which the steps and the expressions of the run read: the bytes are the
    // source's, content-addressed, so writing them again is the same row.
    let mut outputs: Vec<(NodeId, Vec<u8>)> = Vec::new();
    let mut named = BTreeSet::new();
    for row in &prefix.events {
        let (Some(digest), Some(node)) = (&row.payload_digest, row.event.event().node()) else {
            continue;
        };
        if !named.insert(node.clone()) {
            continue;
        }
        let bytes: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2")
                .bind(tx.info.tenant_id.as_uuid())
                .bind(digest)
                .fetch_optional(&mut *tx.tx)
                .await
                .map_err(store_error)?;
        outputs.push((node.clone(), bytes.ok_or(StoreError::MissingReference)?));
    }
    let mut effects = Effects::by(Actor::System("reset"), None);
    effects.outputs = outputs;
    effects.restored = Some(Arc::new(prefix.clone()));
    let events: Vec<RunEvent> = prefix
        .events
        .iter()
        .map(|row| row.event.event().clone())
        .collect();
    tx.persist(&events, effects).await?;
    restore_declared_state(tx, prefix).await
}

/// The declared state of the run is the one the source had at the point: the last Update of the prefix
/// replaced it, and the run was created with the initial one.
async fn restore_declared_state(
    tx: &mut PgRunTx,
    prefix: &RestoredPrefix,
) -> Result<(), StoreError> {
    let last = prefix
        .events
        .iter()
        .rev()
        .find_map(|row| match row.event.event() {
            RunEvent::StateUpdated {
                after_revision,
                state_digest,
                ..
            } => Some((row.sequence, *after_revision, *state_digest)),
            _ => None,
        });
    let Some((sequence, revision, digest)) = last else {
        return Ok(());
    };
    let updated = sqlx::query(
        "UPDATE workflow_operation_states \
         SET state_digest = $4, \
             state_size_bytes = (SELECT octet_length(content) FROM payloads \
                                  WHERE tenant_id = $1 AND digest = $4), \
             state_revision = $5, last_changed_run_version = $6, updated_at = $7 \
         WHERE tenant_id = $1 AND run_id = $2 AND namespace_id = $3",
    )
    .bind(tx.info.tenant_id.as_uuid())
    .bind(tx.info.run_id.as_uuid())
    .bind(tx.info.namespace_id.as_uuid())
    .bind(digest.as_bytes().as_slice())
    .bind(i64::try_from(revision).map_err(|_| corrupt("state revision"))?)
    .bind(i64::try_from(sequence).map_err(|_| corrupt("sequence"))?)
    .bind(tx.now)
    .execute(&mut *tx.tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if updated != 1 {
        return Err(corrupt("the declared state of a reset run"));
    }
    Ok(())
}
