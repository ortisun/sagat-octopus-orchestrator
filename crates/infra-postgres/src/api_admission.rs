//! Canonical namespace lookups for durable public API admission.

use sagat_application::api_admission::ApiAdmissionStore;
use sagat_application::error::StoreError;
use sagat_domain::audit::AuditRecord;
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::{ApprovalRequestId, RunId};
use sagat_domain::identity::{NamespaceId, Slug, TenantId};
use sagat_domain::versioning::RequestId;
use sqlx::Row;

use crate::execution::PgRunStore;
use crate::identity::{DEFAULT_DENIAL_WINDOW, insert_denial_once, store_error};

impl ApiAdmissionStore for PgRunStore {
    async fn record_approval_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        metrics::counter!("security_denials_total", "action" => audit.action().as_str())
            .increment(1);
        match insert_denial_once(self.pool(), audit, DEFAULT_DENIAL_WINDOW).await {
            Ok(()) => {
                metrics::counter!("audit_denials_total", "outcome" => "accepted").increment(1);
                Ok(())
            }
            Err(error) => {
                metrics::counter!("audit_denials_total", "outcome" => "failed").increment(1);
                Err(error)
            }
        }
    }

    async fn namespace_id_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> Result<Option<NamespaceId>, StoreError> {
        optional_namespace(
            sqlx::query(
                "SELECT id AS namespace_id FROM namespaces WHERE tenant_id = $1 AND slug = $2",
            )
            .bind(tenant.as_uuid())
            .bind(slug.as_str())
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?,
        )
    }

    async fn workflow_namespace(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
    ) -> Result<Option<NamespaceId>, StoreError> {
        optional_namespace(
            sqlx::query("SELECT namespace_id FROM workflows WHERE tenant_id = $1 AND id = $2")
                .bind(tenant.as_uuid())
                .bind(workflow.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
        )
    }

    async fn workflow_exists_outside(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
    ) -> Result<bool, StoreError> {
        exists_outside(self, "workflows", tenant, workflow.as_uuid()).await
    }

    async fn run_namespace(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> Result<Option<NamespaceId>, StoreError> {
        optional_namespace(
            sqlx::query("SELECT namespace_id FROM runs WHERE tenant_id = $1 AND id = $2")
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .fetch_optional(self.pool())
                .await
                .map_err(store_error)?,
        )
    }

    async fn run_exists_outside(&self, tenant: TenantId, run: RunId) -> Result<bool, StoreError> {
        exists_outside(self, "runs", tenant, run.as_uuid()).await
    }

    async fn publication_request_namespace(
        &self,
        tenant: TenantId,
        request: RequestId,
    ) -> Result<Option<NamespaceId>, StoreError> {
        optional_namespace(
            sqlx::query(
                "SELECT namespace_id FROM publication_requests WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(request.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?,
        )
    }

    async fn publication_request_exists_outside(
        &self,
        tenant: TenantId,
        request: RequestId,
    ) -> Result<bool, StoreError> {
        exists_outside(self, "publication_requests", tenant, request.as_uuid()).await
    }

    async fn approval_request_namespace(
        &self,
        tenant: TenantId,
        request: ApprovalRequestId,
    ) -> Result<Option<NamespaceId>, StoreError> {
        optional_namespace(
            sqlx::query(
                "SELECT namespace_id FROM approval_requests WHERE tenant_id = $1 AND request_id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(request.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?,
        )
    }

    async fn approval_request_exists_outside(
        &self,
        tenant: TenantId,
        request: ApprovalRequestId,
    ) -> Result<bool, StoreError> {
        exists_outside(self, "approval_requests", tenant, request.as_uuid()).await
    }
}

fn optional_namespace(
    row: Option<sqlx::postgres::PgRow>,
) -> Result<Option<NamespaceId>, StoreError> {
    row.map(|row| {
        Ok(NamespaceId::new(
            row.try_get("namespace_id").map_err(store_error)?,
        ))
    })
    .transpose()
}

async fn exists_outside(
    store: &PgRunStore,
    table: &'static str,
    tenant: TenantId,
    id: uuid::Uuid,
) -> Result<bool, StoreError> {
    let query = match table {
        "workflows" => "SELECT EXISTS (SELECT 1 FROM workflows WHERE id = $1 AND tenant_id <> $2)",
        "runs" => "SELECT EXISTS (SELECT 1 FROM runs WHERE id = $1 AND tenant_id <> $2)",
        "publication_requests" => {
            "SELECT EXISTS (SELECT 1 FROM publication_requests WHERE id = $1 AND tenant_id <> $2)"
        }
        "approval_requests" => {
            "SELECT EXISTS (SELECT 1 FROM approval_requests WHERE request_id = $1 AND tenant_id <> $2)"
        }
        _ => {
            return Err(StoreError::Internal {
                detail: "invalid admission target".to_owned(),
            });
        }
    };
    sqlx::query_scalar(query)
        .bind(id)
        .bind(tenant.as_uuid())
        .fetch_one(store.pool())
        .await
        .map_err(store_error)
}
