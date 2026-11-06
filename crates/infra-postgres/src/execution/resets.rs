//! Reset to a point (feature 018, US06, `FR-018-035`..`FR-018-037`): reading the journal of a source run as a
//! reset sees it, and writing its prefix under the run a reset created.
//!
//! The prefix is written by the same `persist` as every journal, so the steps and the sagas it projects are
//! the ones the platform projects; what it does not do is what the first time did. A row of the prefix is the
//! row the source stored (type, node, attempt, detail, who, when, which payload), marked `restored`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use sagat_application::execution::{Actor, Effects, RunTx as _};
use sagat_application::runs::{
    RerunBusinessId, ResetRecord, ResetRegistration, ResetSource, ResetState, RestoredPrefix,
    Retained, SourceEvent, report_from_json, report_json,
};
use sagat_domain::execution::{AddressedRunEvent, ResetId, ResetReapply, RunEvent};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgRow;

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

/// Writes the entry of the register for the run of `tx` (see
/// [`sagat_application::execution::RunTx::register_reset`]): a reset of an ended source is recorded as
/// created, in the transaction that creates its run.
pub(super) async fn register(
    tx: &mut PgRunTx,
    registration: &ResetRegistration<'_>,
) -> Result<(), StoreError> {
    // the name of the policy is written by the statement: a string literal in Rust that reads like a command
    // is audited as one
    let dropped = matches!(registration.business, RerunBusinessId::Drop);
    sqlx::query(
        "INSERT INTO run_resets \
         (tenant_id, namespace_id, id, source_run_id, source_sequence, prefix_digest, reapply, \
          business_id_policy, source_policy, state, new_run_id, actor, reason, created_at, completed_at, \
          reapply_report) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, CASE WHEN $8 THEN 'DROP' ELSE 'KEEP' END, \
                 'REQUIRE_TERMINAL', 'CREATED', $9, $10, $11, $12, $12, $13)",
    )
    .bind(tx.info.tenant_id.as_uuid())
    .bind(tx.info.namespace_id.as_uuid())
    .bind(registration.id.as_uuid())
    .bind(registration.source.as_uuid())
    .bind(i64::try_from(registration.sequence).map_err(|_| corrupt("sequence"))?)
    .bind(registration.prefix_digest.as_bytes().as_slice())
    .bind(registration.reapply.as_str())
    .bind(dropped)
    .bind(tx.info.run_id.as_uuid())
    .bind(registration.actor)
    .bind(registration.reason)
    .bind(tx.now)
    .bind(report_json(registration.report))
    .execute(&mut *tx.tx)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn record(row: &PgRow) -> Result<ResetRecord, StoreError> {
    let state: String = col(row, "state")?;
    let reapply: String = col(row, "reapply")?;
    let sequence: i64 = col(row, "source_sequence")?;
    let new_run: Option<Uuid> = col(row, "new_run_id")?;
    let report: Value = col(row, "reapply_report")?;
    Ok(ResetRecord {
        id: ResetId::new(col(row, "id")?),
        source: RunId::new(col(row, "source_run_id")?),
        sequence: u64::try_from(sequence).map_err(|_| corrupt("reset sequence"))?,
        state: ResetState::parse(&state).ok_or_else(|| corrupt("reset state"))?,
        failure_reason: col(row, "failure_reason")?,
        new_run: new_run.map(RunId::new),
        reapply: ResetReapply::parse(&reapply).ok_or_else(|| corrupt("reset reapply"))?,
        reapply_report: report_from_json(&report),
        created_at: col(row, "created_at")?,
    })
}

/// The resets made from `source`, newest first (the ids are time-ordered), at most 200.
pub(super) async fn list(
    pool: &PgPool,
    tenant: TenantId,
    source: RunId,
) -> Result<Vec<ResetRecord>, StoreError> {
    sqlx::query(
        "SELECT id, source_run_id, source_sequence, state, failure_reason, new_run_id, reapply, reapply_report, \
                created_at \
         FROM run_resets WHERE tenant_id = $1 AND source_run_id = $2 ORDER BY id DESC LIMIT 200",
    )
    .bind(tenant.as_uuid())
    .bind(source.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(store_error)?
    .iter()
    .map(record)
    .collect()
}

/// One reset made from `source`.
pub(super) async fn get(
    pool: &PgPool,
    tenant: TenantId,
    source: RunId,
    reset: ResetId,
) -> Result<Option<ResetRecord>, StoreError> {
    sqlx::query(
        "SELECT id, source_run_id, source_sequence, state, failure_reason, new_run_id, reapply, reapply_report, \
                created_at \
         FROM run_resets WHERE tenant_id = $1 AND source_run_id = $2 AND id = $3",
    )
    .bind(tenant.as_uuid())
    .bind(source.as_uuid())
    .bind(reset.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(store_error)?
    .as_ref()
    .map(record)
    .transpose()
}

/// The reset that created `run`.
pub(super) async fn made(
    pool: &PgPool,
    tenant: TenantId,
    run: RunId,
) -> Result<Option<ResetRecord>, StoreError> {
    sqlx::query(
        "SELECT id, source_run_id, source_sequence, state, failure_reason, new_run_id, reapply, reapply_report, \
                created_at \
         FROM run_resets WHERE tenant_id = $1 AND new_run_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(store_error)?
    .as_ref()
    .map(record)
    .transpose()
}

/// What the tenant keeps under each digest, in the order asked, in one statement.
pub(super) async fn payload_contents(
    pool: &PgPool,
    tenant: TenantId,
    digests: &[Vec<u8>],
    read: bool,
) -> Result<Vec<Option<Retained>>, StoreError> {
    if digests.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT digest, octet_length(content) AS size, CASE WHEN $3 THEN content END AS content \
         FROM payloads WHERE tenant_id = $1 AND digest = ANY($2)",
    )
    .bind(tenant.as_uuid())
    .bind(digests)
    .bind(read)
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    let mut kept: HashMap<Vec<u8>, Retained> = HashMap::new();
    for row in &rows {
        kept.insert(col(row, "digest")?, retained(row)?);
    }
    Ok(digests
        .iter()
        .map(|digest| kept.get(digest).cloned())
        .collect())
}

fn retained(row: &PgRow) -> Result<Retained, StoreError> {
    let size: i32 = col(row, "size")?;
    Ok(Retained {
        size: u64::try_from(size).map_err(|_| corrupt("payload size"))?,
        content: col(row, "content")?,
    })
}

/// The most resets one row can be traced through to the run that first recorded it. A chain of resets of resets has
/// no bound of its own; this is the bound of the walk, and a row beyond it is not retained for a reset.
const MAX_RESET_HOPS: i32 = 64;

/// A run of the chain that made another: its id and how far it restored the journal of the one before (`None` for a
/// run no reset made).
type Link = (Uuid, Option<i64>);

/// The run whose receipt holds what the row `sequence` kept: the first of the chain that did not restore it from the
/// one before. `None` when the chain ends (at the bound, or at a run that is not there) while the row is still a
/// restored one.
fn owner_of(chain: &[Link], sequence: i64) -> Option<Uuid> {
    for (id, restored_through) in chain {
        match restored_through {
            Some(through) if sequence <= *through => {}
            _ => return Some(*id),
        }
    }
    None
}

/// The runs a run was made from, the run itself first: one statement, however many rows are asked of it.
async fn chain_of(pool: &PgPool, tenant: TenantId, run: RunId) -> Result<Vec<Link>, StoreError> {
    let rows: Vec<(Uuid, Option<i64>)> = sqlx::query_as(
        "WITH RECURSIVE chain(id, restored_through, rerun_of, depth) AS ( \
             SELECT id, reset_sequence, rerun_of, 0 FROM runs WHERE tenant_id = $1 AND id = $2 \
           UNION ALL \
             SELECT r.id, r.reset_sequence, r.rerun_of, c.depth + 1 \
             FROM chain c JOIN runs r ON r.tenant_id = $1 AND r.id = c.rerun_of \
             WHERE c.restored_through IS NOT NULL AND c.depth < $3) \
         SELECT id, restored_through FROM chain ORDER BY depth",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(MAX_RESET_HOPS)
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    Ok(rows)
}

/// The canonical arguments that the Updates recorded by the rows `sequences` of `run` kept, in the order asked, if
/// they are still kept: the chain of resets is read once, the owner of each row is found in memory, and the receipts
/// are read in one statement.
pub(super) async fn retained_arguments(
    pool: &PgPool,
    tenant: TenantId,
    run: RunId,
    sequences: &[u64],
    read: bool,
) -> Result<Vec<Option<Retained>>, StoreError> {
    if sequences.is_empty() {
        return Ok(Vec::new());
    }
    let chain = chain_of(pool, tenant, run).await?;
    let wanted: Vec<Option<(Uuid, i64)>> = sequences
        .iter()
        .map(|sequence| {
            let sequence = i64::try_from(*sequence).ok()?;
            owner_of(&chain, sequence).map(|owner| (owner, sequence))
        })
        .collect();
    let (owners, versions): (Vec<Uuid>, Vec<i64>) = wanted.iter().flatten().copied().unzip();
    if owners.is_empty() {
        return Ok(vec![None; sequences.len()]);
    }
    let rows = sqlx::query(
        "SELECT r.resolved_run_id AS owner, r.run_version AS version, \
                octet_length(p.content) AS size, CASE WHEN $4 THEN p.content END AS content \
         FROM operation_update_receipts r \
         JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.arguments_digest \
         JOIN unnest($2::uuid[], $3::bigint[]) AS asked(owner, version) \
           ON asked.owner = r.resolved_run_id AND asked.version = r.run_version \
         WHERE r.tenant_id = $1 AND r.status = 'APPLIED'",
    )
    .bind(tenant.as_uuid())
    .bind(&owners)
    .bind(&versions)
    .bind(read)
    .fetch_all(pool)
    .await
    .map_err(store_error)?;
    let mut kept: HashMap<(Uuid, i64), Retained> = HashMap::new();
    for row in &rows {
        kept.insert((col(row, "owner")?, col(row, "version")?), retained(row)?);
    }
    Ok(wanted
        .iter()
        .map(|asked| asked.and_then(|key| kept.get(&key).cloned()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_row_belongs_to_the_first_run_of_the_chain_that_did_not_restore_it() {
        // 3 was reset from 2 at 10, 2 from 1 at 6, 1 was made by no reset
        let chain = [(id(3), Some(10)), (id(2), Some(6)), (id(1), None)];
        // restored through both: the first run's
        assert_eq!(owner_of(&chain, 4), Some(id(1)));
        assert_eq!(owner_of(&chain, 6), Some(id(1)));
        // restored by the last reset only
        assert_eq!(owner_of(&chain, 7), Some(id(2)));
        assert_eq!(owner_of(&chain, 10), Some(id(2)));
        // its own
        assert_eq!(owner_of(&chain, 11), Some(id(3)));
        // a run that no reset made owns what it has
        assert_eq!(owner_of(&[(id(1), None)], 1), Some(id(1)));
    }

    #[test]
    fn a_chain_that_ends_while_the_row_is_still_restored_owns_nothing() {
        // the walk stopped at the bound, or the run it was made from is not there
        let cut = [(id(3), Some(10)), (id(2), Some(6))];
        assert_eq!(owner_of(&cut, 5), None);
        assert_eq!(owner_of(&cut, 8), Some(id(2)));
        assert_eq!(owner_of(&[], 1), None);
    }

    #[test]
    fn the_bound_of_the_walk_is_the_one_the_runbook_says() {
        assert_eq!(MAX_RESET_HOPS, 64);
    }
}
