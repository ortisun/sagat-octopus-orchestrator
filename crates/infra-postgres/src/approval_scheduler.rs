//! PostgreSQL candidate discovery and progress delegation for scheduler-driven approvals.

use sagat_application::approval_progress::{
    ApprovalAddress, ApprovalDecisionAttempt, ApprovalExpiryAttempt, ApprovalListItem,
    ApprovalOpening, ApprovalPageSize, ApprovalProgressError, ApprovalProgressStore,
};
use sagat_application::approval_scheduler::{ApprovalSchedulerStore, DueApproval};
use sagat_application::error::StoreError;
use sagat_application::maintenance::{StepRef, Window};
use sagat_domain::execution::{
    ApprovalDecisionResolution, ApprovalRequest, ApprovalRequestId, ApprovalTransition,
    InstancePath, RunId,
};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::NodeId;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::approval_progress::PgApprovalProgressStore;
use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};

fn window_params(window: &Window) -> (i64, Vec<i16>, Vec<Uuid>) {
    (
        i64::from(window.limit),
        window.shards.indexes(),
        window
            .exclude
            .iter()
            .map(|tenant| tenant.as_uuid())
            .collect(),
    )
}

fn step_ref(row: &PgRow) -> Result<StepRef, StoreError> {
    let nodes: Vec<String> = col(row, "instance_path_nodes")?;
    let indices: Vec<i32> = col(row, "instance_path_indices")?;
    if nodes.len() != indices.len() {
        return Err(corrupt("approval candidate instance path"));
    }
    let mut path = InstancePath::root();
    for (node, index) in nodes.into_iter().zip(indices) {
        path = path
            .child(
                NodeId::parse_stored(&node)
                    .map_err(|_| corrupt("approval candidate scope node"))?,
                u32::try_from(index).map_err(|_| corrupt("approval candidate scope index"))?,
            )
            .map_err(|_| corrupt("approval candidate instance path"))?;
    }
    Ok(StepRef {
        tenant_id: TenantId::new(col(row, "tenant_id")?),
        run_id: RunId::new(col(row, "run_id")?),
        node: NodeId::parse_stored(&col::<String>(row, "node")?)
            .map_err(|_| corrupt("approval candidate node"))?,
        instance_path: path,
    })
}

impl ApprovalProgressStore for PgRunStore {
    async fn open(
        &self,
        opening: &ApprovalOpening,
    ) -> Result<ApprovalRequest, ApprovalProgressError> {
        PgApprovalProgressStore::new(self.pool().clone())
            .open(opening)
            .await
    }

    async fn decide(
        &self,
        attempt: &ApprovalDecisionAttempt,
    ) -> Result<ApprovalDecisionResolution, ApprovalProgressError> {
        PgApprovalProgressStore::new(self.pool().clone())
            .decide(attempt)
            .await
    }

    async fn expire(
        &self,
        attempt: &ApprovalExpiryAttempt,
    ) -> Result<ApprovalTransition, ApprovalProgressError> {
        PgApprovalProgressStore::new(self.pool().clone())
            .expire(attempt)
            .await
    }

    async fn load(
        &self,
        address: ApprovalAddress,
    ) -> Result<ApprovalRequest, ApprovalProgressError> {
        PgApprovalProgressStore::new(self.pool().clone())
            .load(address)
            .await
    }

    async fn list_for_run(
        &self,
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        run_id: RunId,
        after_opened_sequence: Option<u64>,
        limit: ApprovalPageSize,
    ) -> Result<Vec<ApprovalListItem>, ApprovalProgressError> {
        PgApprovalProgressStore::new(self.pool().clone())
            .list_for_run(
                tenant_id,
                namespace_id,
                run_id,
                after_opened_sequence,
                limit,
            )
            .await
    }
}

impl ApprovalSchedulerStore for PgRunStore {
    async fn approval_candidates(&self, window: &Window) -> Result<Vec<StepRef>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT candidate.tenant_id,candidate.run_id,candidate.node, \
                    candidate.instance_path_nodes,candidate.instance_path_indices \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
               SELECT step.tenant_id,step.run_id,step.node,step.instance_path_nodes, \
                      step.instance_path_indices,step.updated_at \
               FROM steps step JOIN runs run \
                 ON run.tenant_id=step.tenant_id AND run.id=step.run_id \
               WHERE run_shard(step.run_id)=sh.s AND run.status='RUNNING' \
                 AND step.status='PENDING' AND step.tenant_id <> ALL($3::uuid[]) \
               ORDER BY step.updated_at LIMIT $1 \
             ) candidate ORDER BY candidate.updated_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(step_ref)
        .collect()
    }

    async fn due_approvals(&self, window: &Window) -> Result<Vec<DueApproval>, StoreError> {
        let (limit, shards, exclude) = window_params(window);
        sqlx::query(
            "SELECT candidate.tenant_id,candidate.namespace_id,candidate.request_id, \
                    candidate.run_id,candidate.trace_id,candidate.as_of \
             FROM unnest($2::smallint[]) AS sh(s) \
             CROSS JOIN LATERAL ( \
               SELECT timer.tenant_id,request.namespace_id,timer.request_id,request.run_id, \
                      run.trace_id,now() AS as_of,timer.due_at \
               FROM approval_timers timer \
               JOIN approval_requests request \
                 ON request.tenant_id=timer.tenant_id AND request.request_id=timer.request_id \
               JOIN runs run ON run.tenant_id=request.tenant_id AND run.id=request.run_id \
               WHERE run_shard(request.run_id)=sh.s AND run.status='WAITING' \
                 AND timer.state='PENDING' AND request.status='PENDING' \
                 AND timer.due_at <= now() AND timer.tenant_id <> ALL($3::uuid[]) \
               ORDER BY timer.due_at LIMIT $1 \
             ) candidate ORDER BY candidate.due_at LIMIT $1",
        )
        .bind(limit)
        .bind(shards)
        .bind(exclude)
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let tenant_id = TenantId::new(col(row, "tenant_id")?);
            let request_id = ApprovalRequestId::new(col(row, "request_id")?);
            Ok(DueApproval::new(
                ApprovalAddress::new(
                    tenant_id,
                    NamespaceId::new(col(row, "namespace_id")?),
                    request_id,
                ),
                RunId::new(col(row, "run_id")?),
                col(row, "trace_id")?,
                col(row, "as_of")?,
            ))
        })
        .collect()
    }
}
