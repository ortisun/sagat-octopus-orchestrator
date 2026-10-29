//! PostgreSQL adapter of the scope frontier (G461, D520-D524).
//!
//! Every statement is bounded by the size of the change: the scope row, the iterations that hold a
//! slot (the `*_active_idx` partial indexes), and the steps and payloads of exactly those iterations,
//! found through the primary key of `steps`. Nothing here reads a row of an iteration that ended.

use std::collections::BTreeMap;

use sagat_application::error::StoreError;
use sagat_application::execution::{RunData, ScopeInput};
use sagat_application::primitives::CollectionScopeId;
use sagat_application::scope_frontier::{
    CollectionFrontier, LoopFrontier, ScopeFrontierStore, ScopeMember, ScopeRun,
};
use sagat_domain::blobs::BlobId;
use sagat_domain::execution::{
    CollectionPhase, ExecutionInstanceId, FailureCode, InstancePath, LoopPhase, RunId, StepState,
};
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::NodeId;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::collection_outputs;
use crate::execution::{
    PgRunStore, RUN_SNAPSHOT, instance_path, run_record, scoped_control_metadata_empty,
    scoped_event_metadata_valid, scoped_timer_metadata_valid, step_instance_path, step_state,
    validate_collection_entry_dispatch, validate_scoped_control_row,
    validate_scoped_event_identity,
};
use crate::identity::{col, corrupt, store_error};
use crate::pool::BeginSafely;

/// The columns of a step that `step_state` reads, plus the identity and the retained output.
const MEMBER_STEPS: &str = concat!(
    "SELECT s.node, s.instance_path_nodes, s.instance_path_indices, s.namespace_id, s.step_id, \
        s.status, s.attempt, s.generation, s.cancel_requested, s.routed, s.condition, \
        s.switch_case_id, s.timed_out, s.failure_code, s.next_attempt_at, s.waiting_kind, \
        s.due_at, s.correlation_key, s.event_type, s.event_since, s.child_run_id, s.approval_request_id, p.content AS output, \
        s.node_kind AS stored_node_kind, (",
    scoped_control_metadata_empty!(),
    ") AS control_metadata_empty, (",
    scoped_timer_metadata_valid!(),
    ") AS timer_metadata_valid, (",
    scoped_event_metadata_valid!(),
    ") AS event_metadata_valid \
     FROM unnest($4::int[]) AS member(last_index) \
     JOIN steps s ON s.tenant_id = $1 AND s.run_id = $2 \
      AND s.instance_path_nodes = $3 \
      AND s.instance_path_indices = array_append($5::int[], member.last_index) \
     LEFT JOIN payloads p ON p.tenant_id = s.tenant_id AND p.digest = s.output_digest"
);

/// A snapshot read: one consistent view, no row lock, nothing written.
async fn snapshot(store: &PgRunStore) -> Result<Transaction<'static, Postgres>, StoreError> {
    let mut tx = store.pool.begin_safely().await.map_err(store_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
    Ok(tx)
}

struct Header {
    run: ScopeRun,
    input_digest: Vec<u8>,
}

async fn header(
    store: &PgRunStore,
    conn: &mut PgConnection,
    tenant: TenantId,
    run: RunId,
) -> Result<Option<Header>, StoreError> {
    let Some(row) = sqlx::query(RUN_SNAPSHOT)
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let record = run_record(&row)?;
    let now: OffsetDateTime = col(&row, "db_now")?;
    let input_digest: Vec<u8> = col(&row, "input_digest")?;
    let stop_code = col::<Option<String>>(&row, "stop_code")?
        .map(|code| FailureCode::parse(&code).map_err(|_| corrupt("stop code")))
        .transpose()?;
    // The current-instance projection matches RunState, independently of the lifetime admission
    // and reservation budget. One keyed row is read, regardless of completed iteration count.
    let charged_instances: i64 = sqlx::query_scalar(
        "SELECT charged_instances FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let definition = store
        .definition(&mut *conn, tenant, record.info.version_id)
        .await?;
    // Keep E with the revision, stop and scope metadata from this repeatable-read snapshot. The
    // helper validates only bounded metadata and lengths; active-member payloads are loaded later.
    let effective_output_bytes = collection_outputs::inspect(&mut *conn, &record.info, &definition)
        .await?
        .total();
    Ok(Some(Header {
        run: ScopeRun {
            info: record.info,
            now,
            version: record.version,
            stop_reason: record.stop_reason,
            stop_code,
            effective_output_bytes,
            definition,
            charged_instances: usize::try_from(charged_instances)
                .map_err(|_| corrupt("instance count"))?,
        },
        input_digest,
    }))
}

/// The run input and the declared state; the outputs and scope inputs are added by the caller.
async fn base_data(
    conn: &mut PgConnection,
    tenant: TenantId,
    run: RunId,
    input_digest: &[u8],
) -> Result<RunData, StoreError> {
    let input_row = sqlx::query(
        "SELECT p.content, r.input_blob_id FROM runs r JOIN payloads p \
         ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest \
         WHERE r.tenant_id = $1 AND r.id = $2 AND p.digest = $3",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(input_digest)
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let state_row = sqlx::query(
        "SELECT p.content, s.state_revision FROM workflow_operation_states s JOIN payloads p \
         ON p.tenant_id=s.tenant_id AND p.digest=s.state_digest \
         WHERE s.tenant_id=$1 AND s.run_id=$2",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_optional(&mut *conn)
    .await
    .map_err(store_error)?;
    let state = state_row
        .as_ref()
        .map(|row| col::<Vec<u8>>(row, "content"))
        .transpose()?;
    let state_revision = state_row
        .as_ref()
        .map(|row| {
            u64::try_from(col::<i64>(row, "state_revision")?)
                .map_err(|_| corrupt("operation state revision"))
        })
        .transpose()?;
    Ok(RunData {
        run_id: Some(run),
        input: col(&input_row, "content")?,
        input_blob: col::<Option<Uuid>>(&input_row, "input_blob_id")?.map(BlobId::new),
        state,
        state_revision,
        outputs: BTreeMap::new(),
        scope_inputs: BTreeMap::new(),
    })
}

/// The steps (and their retained outputs) of the iterations whose last path index is in
/// `last_indices`, all under the same `base` path of an owner. The outputs go into `data`.
async fn member_steps(
    conn: &mut PgConnection,
    scope_run: &ScopeRun,
    nodes: &[String],
    base_indices: &[i32],
    last_indices: &[i32],
    data: &mut RunData,
) -> Result<BTreeMap<i32, BTreeMap<ExecutionInstanceId, StepState>>, StoreError> {
    let mut by_member: BTreeMap<i32, BTreeMap<ExecutionInstanceId, StepState>> = BTreeMap::new();
    if last_indices.is_empty() {
        return Ok(by_member);
    }
    let rows: Vec<PgRow> = sqlx::query(MEMBER_STEPS)
        .bind(scope_run.info.tenant_id.as_uuid())
        .bind(scope_run.info.run_id.as_uuid())
        .bind(nodes)
        .bind(last_indices)
        .bind(base_indices)
        .fetch_all(&mut *conn)
        .await
        .map_err(store_error)?;
    for row in &rows {
        let indices: Vec<i32> = col(row, "instance_path_indices")?;
        let last = *indices.last().ok_or_else(|| corrupt("scoped step path"))?;
        let node: String = col(row, "node")?;
        let instance = ExecutionInstanceId::in_path(
            step_instance_path(row)?,
            NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
        );
        if let Some(output) = col::<Option<Vec<u8>>>(row, "output")? {
            data.insert_output(instance.clone(), output);
        }
        let step = step_state(row)?;
        validate_scoped_control_row(
            scope_run.definition.node_for_instance(&instance),
            &step,
            row,
        )?;
        validate_scoped_event_identity(
            scope_run.definition.node_for_instance(&instance),
            scope_run.info.namespace_id,
            sagat_application::execution::instance_step_id(
                scope_run.info.version_id,
                scope_run.info.run_id,
                &instance,
            ),
            row,
        )?;
        by_member.entry(last).or_default().insert(instance, step);
    }
    Ok(by_member)
}

fn member_index(value: i32) -> Result<u32, StoreError> {
    u32::try_from(value).map_err(|_| corrupt("scope iteration index"))
}

/// Whether root work other than the scope's owner is still open (the root has an empty path).
async fn other_root_work(
    conn: &mut PgConnection,
    tenant: TenantId,
    run: RunId,
    owner: &ExecutionInstanceId,
) -> Result<bool, StoreError> {
    let (owner_nodes, owner_indices) = path_parts(owner.path());
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM steps \
         WHERE tenant_id = $1 AND run_id = $2 AND instance_path_nodes = '{}' \
           AND NOT (instance_path_nodes = $3 AND instance_path_indices = $4 AND node = $5) \
           AND status NOT IN ('SUCCEEDED', 'FAILED', 'CANCELLED', 'SKIPPED'))",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .bind(owner_nodes)
    .bind(owner_indices)
    .bind(owner.node().as_str())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)
}

fn path_parts(path: &InstancePath) -> (Vec<String>, Vec<i32>) {
    path.segments()
        .iter()
        .map(|segment| {
            (
                segment.scope_node().as_str().to_owned(),
                i32::try_from(segment.iteration_index()).unwrap_or(i32::MAX),
            )
        })
        .unzip()
}

fn owner_parts(owner: &ExecutionInstanceId) -> (Vec<String>, Vec<i32>, String) {
    let (nodes, indices) = path_parts(owner.path());
    (nodes, indices, owner.node().as_str().to_owned())
}

impl ScopeFrontierStore for PgRunStore {
    async fn collection_frontier(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<Option<CollectionFrontier>, StoreError> {
        let mut tx = snapshot(self).await?;
        let Some(header) = header(self, &mut tx, tenant_id, run_id).await? else {
            return Ok(None);
        };
        let (owner_nodes, owner_indices, owner_node) = owner_parts(owner);
        let Some(scope) = sqlx::query(
            "SELECT scope_id, lifecycle, item_count, next_admission_index, completed_count, \
                    active_count, max_concurrency \
             FROM iteration_scopes WHERE tenant_id = $1 AND run_id = $2 \
               AND owner_path_nodes = $3 AND owner_path_indices = $4 AND owner_node = $5",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(&owner_node)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let scope_id: Uuid = col(&scope, "scope_id")?;
        let phase = match col::<String>(&scope, "lifecycle")?.as_str() {
            "OPEN" => CollectionPhase::Open,
            "DRAINING" => CollectionPhase::Draining,
            "SUCCEEDED" => CollectionPhase::Succeeded,
            "FAILED" => CollectionPhase::Failed,
            "CANCELLED" => CollectionPhase::Cancelled,
            _ => return Err(corrupt("collection lifecycle")),
        };
        let item_count = member_index(col(&scope, "item_count")?)?;
        let next_admission = member_index(col(&scope, "next_admission_index")?)?;
        let completed = member_index(col(&scope, "completed_count")?)?;
        let active_count = member_index(col(&scope, "active_count")?)?;
        let max_concurrency = u16::try_from(col::<i32>(&scope, "max_concurrency")?)
            .map_err(|_| corrupt("collection concurrency limit"))?;

        let mut data = base_data(&mut tx, tenant_id, run_id, &header.input_digest).await?;
        let rows = sqlx::query(
            "SELECT i.iteration_index, i.entry_path_nodes, i.entry_path_indices, i.entry_node, \
                    p.content AS item, i.entry_dispatch,i.activity,i.outbox_id, \
                    admission.detail->>'entry_dispatch' AS journal_dispatch \
             FROM collection_iterations i \
             LEFT JOIN run_events admission ON admission.tenant_id=i.tenant_id AND admission.run_id=i.run_id \
              AND admission.seq=i.admitted_sequence AND admission.event_type='collection.iteration_admitted' \
             JOIN payloads p ON p.tenant_id = i.tenant_id AND p.digest = i.item_digest \
             WHERE i.tenant_id = $1 AND i.run_id = $2 AND i.scope_id = $3 AND i.slot_held \
             ORDER BY i.iteration_index",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(scope_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        // The counter the scope keeps must agree with the rows that hold a slot (D524).
        if u32::try_from(rows.len()).ok() != Some(active_count) {
            return Err(corrupt("collection active count"));
        }
        let mut entries = Vec::with_capacity(rows.len());
        let mut common: Option<(Vec<String>, Vec<i32>)> = None;
        let mut lasts = Vec::with_capacity(rows.len());
        for row in &rows {
            let index = member_index(col(row, "iteration_index")?)?;
            let nodes: Vec<String> = col(row, "entry_path_nodes")?;
            let mut indices: Vec<i32> = col(row, "entry_path_indices")?;
            let last = indices
                .pop()
                .ok_or_else(|| corrupt("collection entry path"))?;
            if last != i32::try_from(index).map_err(|_| corrupt("collection entry path"))? {
                return Err(corrupt("collection entry path"));
            }
            match &common {
                None => common = Some((nodes.clone(), indices.clone())),
                Some((n, i)) if *n == nodes && *i == indices => {}
                Some(_) => return Err(corrupt("collection entry path")),
            }
            let mut full = indices;
            full.push(last);
            let node: String = col(row, "entry_node")?;
            let entry = ExecutionInstanceId::in_path(
                instance_path(nodes, full, "collection entry path")?,
                NodeId::parse_stored(&node).map_err(|_| corrupt("collection entry node"))?,
            );
            validate_collection_entry_dispatch(&header.run.definition, &entry, row)?;
            data.scope_inputs.insert(
                entry.path().clone(),
                ScopeInput::Collection {
                    index,
                    item: col(row, "item")?,
                },
            );
            lasts.push(last);
            entries.push((index, entry));
        }
        let mut by_member = match &common {
            Some((nodes, base)) => {
                member_steps(&mut tx, &header.run, nodes, base, &lasts, &mut data).await?
            }
            None => BTreeMap::new(),
        };
        let active = entries
            .into_iter()
            .map(|(index, entry)| ScopeMember {
                index,
                steps: by_member
                    .remove(&i32::try_from(index).unwrap_or(i32::MAX))
                    .unwrap_or_default(),
                entry,
            })
            .collect();
        let other_root_work = if phase == CollectionPhase::Draining {
            other_root_work(&mut tx, tenant_id, run_id, owner).await?
        } else {
            false
        };
        Ok(Some(CollectionFrontier {
            run: header.run,
            scope_id: CollectionScopeId::new(scope_id),
            phase,
            item_count,
            next_admission,
            completed,
            max_concurrency,
            active,
            other_root_work,
            data,
        }))
    }

    async fn loop_frontier(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
        owner: &ExecutionInstanceId,
    ) -> Result<Option<LoopFrontier>, StoreError> {
        let mut tx = snapshot(self).await?;
        let Some(header) = header(self, &mut tx, tenant_id, run_id).await? else {
            return Ok(None);
        };
        let (owner_nodes, owner_indices, owner_node) = owner_parts(owner);
        let Some(scope) = sqlx::query(
            "SELECT owner_instance_id, lifecycle, deadline_at, next_iteration, active_iteration, \
                    delay_due_at, current_carry_digest \
             FROM loop_scopes WHERE tenant_id = $1 AND run_id = $2 \
               AND owner_path_nodes = $3 AND owner_path_indices = $4 AND owner_node = $5",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(&owner_nodes)
        .bind(&owner_indices)
        .bind(&owner_node)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let owner_id: Uuid = col(&scope, "owner_instance_id")?;
        let phase = match col::<String>(&scope, "lifecycle")?.as_str() {
            "READY" => LoopPhase::Ready,
            "RUNNING" => LoopPhase::Running,
            "DELAYING" => LoopPhase::Delaying,
            "SUCCEEDED" => LoopPhase::Succeeded,
            "FAILED" => LoopPhase::Failed,
            "DRAINING" => LoopPhase::Draining,
            "CANCELLED" => LoopPhase::Cancelled,
            _ => return Err(corrupt("loop lifecycle")),
        };
        let projected_active = col::<Option<i32>>(&scope, "active_iteration")?
            .map(member_index)
            .transpose()?;
        let mut data = base_data(&mut tx, tenant_id, run_id, &header.input_digest).await?;
        let rows = sqlx::query(
            "SELECT iteration_index, entry_path_nodes, entry_path_indices, entry_node \
             FROM loop_iterations \
             WHERE tenant_id = $1 AND run_id = $2 AND owner_instance_id = $3 AND slot_held \
             ORDER BY iteration_index",
        )
        .bind(tenant_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(owner_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;
        if rows.len() > 1 {
            return Err(corrupt("loop active iteration"));
        }
        let mut member = None;
        if let Some(row) = rows.first() {
            let index = member_index(col(row, "iteration_index")?)?;
            // The counter the scope keeps names the iteration that holds the slot (D524).
            if projected_active != Some(index) {
                return Err(corrupt("loop active iteration"));
            }
            let nodes: Vec<String> = col(row, "entry_path_nodes")?;
            let mut indices: Vec<i32> = col(row, "entry_path_indices")?;
            let last = indices.pop().ok_or_else(|| corrupt("loop entry path"))?;
            if last != i32::try_from(index).map_err(|_| corrupt("loop entry path"))? {
                return Err(corrupt("loop entry path"));
            }
            let base = indices.clone();
            let mut full = indices;
            full.push(last);
            let node: String = col(row, "entry_node")?;
            let entry = ExecutionInstanceId::in_path(
                instance_path(nodes.clone(), full, "loop entry path")?,
                NodeId::parse_stored(&node).map_err(|_| corrupt("loop entry node"))?,
            );
            if phase == LoopPhase::Running {
                let carry: Vec<u8> = sqlx::query_scalar(
                    "SELECT content FROM payloads WHERE tenant_id = $1 AND digest = $2",
                )
                .bind(tenant_id.as_uuid())
                .bind(col::<Vec<u8>>(&scope, "current_carry_digest")?)
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                data.scope_inputs.insert(
                    entry.path().clone(),
                    ScopeInput::Loop {
                        iteration: index,
                        carry,
                    },
                );
            }
            let mut by_member =
                member_steps(&mut tx, &header.run, &nodes, &base, &[last], &mut data).await?;
            member = Some(ScopeMember {
                index,
                steps: by_member.remove(&last).unwrap_or_default(),
                entry,
            });
        } else if projected_active.is_some() {
            return Err(corrupt("loop active iteration"));
        }
        let other_root_work = if phase == LoopPhase::Draining {
            other_root_work(&mut tx, tenant_id, run_id, owner).await?
        } else {
            false
        };
        Ok(Some(LoopFrontier {
            run: header.run,
            phase,
            deadline: col(&scope, "deadline_at")?,
            next_iteration: member_index(col(&scope, "next_iteration")?)?,
            delay_until: col(&scope, "delay_due_at")?,
            member,
            other_root_work,
            data,
        }))
    }

    async fn retained_output_bytes(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
    ) -> Result<u64, StoreError> {
        let mut tx = snapshot(self).await?;
        let header = header(self, &mut tx, tenant_id, run_id)
            .await?
            .ok_or(StoreError::MissingReference)?;
        Ok(header.run.effective_output_bytes)
    }
}
