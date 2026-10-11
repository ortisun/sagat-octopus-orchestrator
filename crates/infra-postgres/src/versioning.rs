//! PostgreSQL implementation of the publishing port: policies, requests, the history of the active
//! pointer (FR-011-002..005).
//!
//! The rules of a request's life are the domain's (`PublicationRequest`): this module loads a
//! request under the lock of its row, applies one transition, and writes the result and its audit
//! record in the same transaction. Every statement carries the tenant.

use sagat_application::definitions::PublishContext;
use sagat_application::dispatch::TriggerRegistration;
use sagat_application::error::StoreError;
use sagat_application::publishing::{
    ActivationRecord, Decided, NewActivation, Opened, PromotedFrom, PublishingStore, StoredPolicy,
    Switched,
};
use sagat_domain::audit::AuditRecord;
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{Namespace, NamespaceId, Subject, TenantId};
use sagat_domain::versioning::{
    ActivationId, ActivationKind, ApprovalPolicy, Decision, PublicationRequest, RequestId,
    RequestKind, RequestState, Verdict,
};
use sagat_domain::workflow::Digest;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::definitions::{PgDefinitionStore, bare_workflow, register_triggers};
use crate::identity::{col, corrupt, insert_audit, namespace_from_row, store_error};
use crate::pool::BeginSafely;

const REQUEST_COLUMNS: &str = "id, tenant_id, namespace_id, workflow_id, kind, draft_revision, \
     digest, release_notes, promoted_from_version_id, requested_by, requested_at, \
     required_approvals, state, published_version_id, published_by, published_at";

fn subject(text: &str) -> Result<Subject, StoreError> {
    Subject::parse(text).map_err(|_| corrupt("subject"))
}

fn request_from_row(
    row: &PgRow,
    decisions: Vec<Decision>,
) -> Result<PublicationRequest, StoreError> {
    let kind: String = col(row, "kind")?;
    let state: String = col(row, "state")?;
    let digest: String = col(row, "digest")?;
    let revision: i64 = col(row, "draft_revision")?;
    let required: i16 = col(row, "required_approvals")?;
    let published = match col::<Option<Uuid>>(row, "published_version_id")? {
        None => None,
        Some(version) => {
            let by: String = col(row, "published_by")?;
            let at: OffsetDateTime = col(row, "published_at")?;
            Some((WorkflowVersionId::new(version), subject(&by)?, at))
        }
    };
    let requested_by: String = col(row, "requested_by")?;
    Ok(PublicationRequest::restore(
        RequestId::new(col::<Uuid>(row, "id")?),
        TenantId::new(col::<Uuid>(row, "tenant_id")?),
        NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
        RequestKind::parse(&kind).ok_or_else(|| corrupt("request kind"))?,
        u64::try_from(revision).map_err(|_| corrupt("draft revision"))?,
        Digest::from_hex(&digest).ok_or_else(|| corrupt("digest"))?,
        col(row, "release_notes")?,
        col::<Option<Uuid>>(row, "promoted_from_version_id")?.map(WorkflowVersionId::new),
        subject(&requested_by)?,
        col(row, "requested_at")?,
        u8::try_from(required).map_err(|_| corrupt("required approvals"))?,
        decisions,
        RequestState::parse(&state).ok_or_else(|| corrupt("request state"))?,
        published,
    ))
}

async fn decisions_of(
    conn: &mut PgConnection,
    tenant: TenantId,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, Decision)>, StoreError> {
    let rows = sqlx::query(
        "SELECT request_id, approver, verdict, reason, decided_at FROM publication_decisions \
         WHERE tenant_id = $1 AND request_id = ANY($2) ORDER BY decided_at, approver",
    )
    .bind(tenant.as_uuid())
    .bind(ids)
    .fetch_all(conn)
    .await
    .map_err(store_error)?;
    rows.iter()
        .map(|row| {
            let verdict: String = col(row, "verdict")?;
            let approver: String = col(row, "approver")?;
            Ok((
                col::<Uuid>(row, "request_id")?,
                Decision {
                    by: subject(&approver)?,
                    verdict: match verdict.as_str() {
                        "APPROVED" => Verdict::Approve,
                        "REJECTED" => Verdict::Reject,
                        _ => return Err(corrupt("verdict")),
                    },
                    reason: col(row, "reason")?,
                    at: col(row, "decided_at")?,
                },
            ))
        })
        .collect()
}

async fn load_request(
    conn: &mut PgConnection,
    tenant: TenantId,
    id: RequestId,
    lock: bool,
) -> Result<Option<PublicationRequest>, StoreError> {
    let sql = if lock {
        format!(
            "SELECT {REQUEST_COLUMNS} FROM publication_requests \
             WHERE tenant_id = $1 AND id = $2 FOR UPDATE"
        )
    } else {
        format!(
            "SELECT {REQUEST_COLUMNS} FROM publication_requests WHERE tenant_id = $1 AND id = $2"
        )
    };
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&mut *conn)
        .await
        .map_err(store_error)?
    else {
        return Ok(None);
    };
    let decisions = decisions_of(conn, tenant, &[id.as_uuid()])
        .await?
        .into_iter()
        .map(|(_, d)| d)
        .collect();
    request_from_row(&row, decisions).map(Some)
}

/// Writes the state a transition left a request in.
async fn save_state(
    conn: &mut PgConnection,
    request: &PublicationRequest,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE publication_requests SET state = $3, updated_at = $4 \
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(request.tenant_id().as_uuid())
    .bind(request.id().as_uuid())
    .bind(request.state().as_str())
    .bind(now)
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

impl PublishingStore for PgDefinitionStore {
    async fn namespace_by_id(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<Option<Namespace>, StoreError> {
        sqlx::query(
            "SELECT id, tenant_id, workspace_id, slug, environment, created_at \
             FROM namespaces WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        .map(|row| namespace_from_row(&row))
        .transpose()
    }

    async fn namespace_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM namespaces WHERE id = $2 AND tenant_id <> $1)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_one(self.pool())
        .await
        .map_err(store_error)
    }

    async fn version_scripts(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> Result<Vec<String>, StoreError> {
        sqlx::query_scalar(
            "SELECT digest FROM workflow_version_scripts \
             WHERE tenant_id = $1 AND workflow_version_id = $2 ORDER BY digest",
        )
        .bind(tenant.as_uuid())
        .bind(version.as_uuid())
        .fetch_all(self.pool())
        .await
        .map_err(store_error)
    }

    async fn request_outside(&self, tenant: TenantId, id: RequestId) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM publication_requests WHERE id = $2 AND tenant_id <> $1)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_one(self.pool())
        .await
        .map_err(store_error)
    }

    async fn stored_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Option<StoredPolicy>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT required_approvals, set_by, set_at FROM publication_policies \
             WHERE tenant_id = $1 AND namespace_id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let required: i16 = col(&row, "required_approvals")?;
        Ok(Some(StoredPolicy {
            required: ApprovalPolicy::new(u8::try_from(required).map_err(|_| corrupt("policy"))?)
                .map_err(|_| corrupt("policy"))?,
            set_by: col(&row, "set_by")?,
            set_at: col(&row, "set_at")?,
        }))
    }

    async fn set_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        policy: ApprovalPolicy,
        by: &str,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        sqlx::query(
            "INSERT INTO publication_policies (tenant_id, namespace_id, required_approvals, set_by, set_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (tenant_id, namespace_id) DO UPDATE \
               SET required_approvals = EXCLUDED.required_approvals, set_by = EXCLUDED.set_by, \
                   set_at = EXCLUDED.set_at",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(i16::from(policy.required()))
        .bind(by)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn open_request(
        &self,
        request: &PublicationRequest,
        audit: &AuditRecord,
    ) -> Result<Opened, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let inserted = sqlx::query(
            "INSERT INTO publication_requests \
             (id, tenant_id, namespace_id, workflow_id, kind, draft_revision, digest, release_notes, \
              promoted_from_version_id, requested_by, requested_at, required_approvals, state, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $11) \
             ON CONFLICT (tenant_id, workflow_id) WHERE state IN ('PENDING', 'APPROVED') DO NOTHING",
        )
        .bind(request.id().as_uuid())
        .bind(request.tenant_id().as_uuid())
        .bind(request.namespace_id().as_uuid())
        .bind(request.workflow_id().as_uuid())
        .bind(request.kind().as_str())
        .bind(i64::try_from(request.draft_revision()).map_err(|_| corrupt("draft revision"))?)
        .bind(request.digest().to_hex())
        .bind(request.release_notes())
        .bind(request.promoted_from().map(WorkflowVersionId::as_uuid))
        .bind(request.requested_by().as_str())
        .bind(request.requested_at())
        .bind(i16::from(request.required()))
        .bind(request.state().as_str())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if inserted == 0 {
            let open: Uuid = sqlx::query_scalar(
                "SELECT id FROM publication_requests \
                 WHERE tenant_id = $1 AND workflow_id = $2 AND state IN ('PENDING', 'APPROVED')",
            )
            .bind(request.tenant_id().as_uuid())
            .bind(request.workflow_id().as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            return Ok(Opened::AlreadyOpen(RequestId::new(open)));
        }
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Opened::Done)
    }

    async fn request(
        &self,
        tenant: TenantId,
        id: RequestId,
    ) -> Result<Option<PublicationRequest>, StoreError> {
        let mut conn = self.pool().acquire().await.map_err(store_error)?;
        load_request(&mut conn, tenant, id, false).await
    }

    async fn requests(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        state: Option<RequestState>,
        before: Option<RequestId>,
        limit: u32,
    ) -> Result<Vec<PublicationRequest>, StoreError> {
        let mut conn = self.pool().acquire().await.map_err(store_error)?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {REQUEST_COLUMNS} FROM publication_requests \
             WHERE tenant_id = $1 AND workflow_id = $2 \
               AND ($3::text IS NULL OR state = $3) AND ($4::uuid IS NULL OR id < $4) \
             ORDER BY id DESC LIMIT $5"
        )))
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .bind(state.map(RequestState::as_str))
        .bind(before.map(RequestId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(&mut *conn)
        .await
        .map_err(store_error)?;
        let ids: Vec<Uuid> = rows
            .iter()
            .map(|r| col::<Uuid>(r, "id"))
            .collect::<Result<_, _>>()?;
        let decisions = decisions_of(&mut conn, tenant, &ids).await?;
        rows.iter()
            .map(|row| {
                let id: Uuid = col(row, "id")?;
                let mine = decisions
                    .iter()
                    .filter(|(request, _)| *request == id)
                    .map(|(_, d)| d.clone())
                    .collect();
                request_from_row(row, mine)
            })
            .collect()
    }

    async fn decide(
        &self,
        tenant: TenantId,
        id: RequestId,
        by: &Subject,
        verdict: Verdict,
        reason: &str,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<Decided, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let Some(mut request) = load_request(&mut tx, tenant, id, true).await? else {
            return Ok(Decided::NotFound);
        };
        let outcome = match verdict {
            Verdict::Approve => request.approve(by, reason, now).map(|_| ()),
            Verdict::Reject => request.reject(by, reason, now),
        };
        if let Err(refused) = outcome {
            return Ok(Decided::Refused(refused));
        }
        sqlx::query(
            "INSERT INTO publication_decisions (tenant_id, request_id, approver, verdict, reason, decided_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .bind(by.as_str())
        .bind(verdict.as_str())
        .bind(reason)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        save_state(&mut tx, &request, now).await?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Decided::Done(Box::new(request)))
    }

    async fn withdraw(
        &self,
        tenant: TenantId,
        id: RequestId,
        by: &Subject,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<Decided, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        let Some(mut request) = load_request(&mut tx, tenant, id, true).await? else {
            return Ok(Decided::NotFound);
        };
        if let Err(refused) = request.withdraw(by) {
            return Ok(Decided::Refused(refused));
        }
        save_state(&mut tx, &request, now).await?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Decided::Done(Box::new(request)))
    }

    async fn activations(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        before: Option<ActivationId>,
        limit: u32,
    ) -> Result<Vec<ActivationRecord>, StoreError> {
        sqlx::query(
            "SELECT a.id, a.workflow_id, a.version_id, v.version_number, pv.version_number AS previous_number, \
                    a.kind, a.reason, a.actor, a.request_id, a.created_at, \
                    sv.version_number AS source_number, sw.id AS source_workflow, sw.namespace_id AS source_namespace \
             FROM workflow_activations a \
             JOIN workflow_versions v ON v.tenant_id = a.tenant_id AND v.id = a.version_id \
             LEFT JOIN workflow_versions pv ON pv.tenant_id = a.tenant_id AND pv.id = a.previous_version_id \
             LEFT JOIN workflow_versions sv ON sv.tenant_id = a.tenant_id AND sv.id = a.promoted_from_version_id \
             LEFT JOIN workflows sw ON sw.tenant_id = a.tenant_id AND sw.id = sv.workflow_id \
             WHERE a.tenant_id = $1 AND a.workflow_id = $2 \
               AND ($3::uuid IS NULL OR (a.created_at, a.id) < ( \
                    SELECT c.created_at, c.id FROM workflow_activations c \
                    WHERE c.id = $3 AND c.tenant_id = $1 AND c.workflow_id = $2)) \
             ORDER BY a.created_at DESC, a.id DESC LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .bind(before.map(ActivationId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?
        .iter()
        .map(activation_from_row)
        .collect()
    }

    async fn switch(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        to: u32,
        expected: Option<u32>,
        change: &NewActivation,
        triggers: &[TriggerRegistration],
        audit: &AuditRecord,
    ) -> Result<Switched, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        // The row lock serialises everything that moves the pointer of this workflow.
        let Some(locked) = sqlx::query(
            "SELECT id, tenant_id, namespace_id, name, draft_revision, active_version_id, \
                    created_at, updated_at \
             FROM workflows WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            return Ok(Switched::NoWorkflow);
        };
        let record = bare_workflow(&locked)?;
        let active: Option<(Uuid, i32)> = match record.active_version() {
            None => None,
            Some(id) => {
                let number: i32 = sqlx::query_scalar(
                    "SELECT version_number FROM workflow_versions WHERE tenant_id = $1 AND id = $2",
                )
                .bind(tenant.as_uuid())
                .bind(id.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                Some((id.as_uuid(), number))
            }
        };
        let current = active
            .map(|(_, n)| u32::try_from(n))
            .transpose()
            .map_err(|_| corrupt("version number"))?;
        if expected.is_some() && expected != current {
            return Ok(Switched::Stale { current });
        }
        let Some(target) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM workflow_versions \
             WHERE tenant_id = $1 AND workflow_id = $2 AND version_number = $3",
        )
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .bind(i64::from(to))
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?
        else {
            return Ok(Switched::NoVersion);
        };
        if current == Some(to) {
            return Ok(Switched::Unchanged);
        }
        sqlx::query(
            "INSERT INTO workflow_activations \
             (id, tenant_id, workflow_id, version_id, previous_version_id, kind, reason, actor, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(change.id.as_uuid())
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .bind(target)
        .bind(active.map(|(id, _)| id))
        .bind(change.kind.as_str())
        .bind(&change.reason)
        .bind(&change.actor)
        .bind(change.at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        sqlx::query(
            "UPDATE workflows SET active_version_id = $3, updated_at = $4 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .bind(target)
        .bind(change.at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        register_triggers(&mut tx, &record, WorkflowVersionId::new(target), triggers).await?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        let latest = self
            .activations(tenant, workflow, None, 1)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| corrupt("activation"))?;
        Ok(Switched::Done(Box::new(latest)))
    }
}

fn activation_from_row(row: &PgRow) -> Result<ActivationRecord, StoreError> {
    let kind: String = col(row, "kind")?;
    let number: i32 = col(row, "version_number")?;
    let previous: Option<i32> = col(row, "previous_number")?;
    let promoted = match col::<Option<i32>>(row, "source_number")? {
        None => None,
        Some(source) => Some(PromotedFrom {
            namespace_id: NamespaceId::new(col::<Uuid>(row, "source_namespace")?),
            workflow_id: WorkflowId::new(col::<Uuid>(row, "source_workflow")?),
            version: u32::try_from(source).map_err(|_| corrupt("version number"))?,
        }),
    };
    Ok(ActivationRecord {
        id: ActivationId::new(col::<Uuid>(row, "id")?),
        workflow_id: WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
        version_id: WorkflowVersionId::new(col::<Uuid>(row, "version_id")?),
        version: u32::try_from(number).map_err(|_| corrupt("version number"))?,
        previous: previous
            .map(u32::try_from)
            .transpose()
            .map_err(|_| corrupt("version number"))?,
        kind: kind
            .parse::<ActivationKind>()
            .map_err(|()| corrupt("activation kind"))?,
        reason: col(row, "reason")?,
        actor: col(row, "actor")?,
        request: col::<Option<Uuid>>(row, "request_id")?.map(RequestId::new),
        promoted_from: promoted,
        at: col(row, "created_at")?,
    })
}

/// A publication as the transaction that makes it sees it.
#[derive(Clone, Copy)]
pub(crate) struct Published<'a> {
    pub tenant: TenantId,
    pub workflow: WorkflowId,
    /// The version the pointer had before.
    pub previous_active: Option<WorkflowVersionId>,
    pub version: WorkflowVersionId,
    /// False when the document was a version that existed.
    pub created: bool,
    pub context: &'a PublishContext,
    pub actor: &'a str,
    pub now: OffsetDateTime,
}

/// What a publication records beyond the version: the change of the pointer, the artifacts the
/// version pins, and that a request was published. Called inside the publishing transaction, with
/// the workflow row locked.
pub(crate) async fn record_publication(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    published: &Published<'_>,
) -> Result<(), StoreError> {
    let Published {
        tenant,
        workflow,
        previous_active,
        version,
        created,
        context,
        actor,
        now,
    } = *published;
    if created {
        for digest in &context.scripts {
            sqlx::query(
                "INSERT INTO workflow_version_scripts (tenant_id, workflow_version_id, digest) \
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(tenant.as_uuid())
            .bind(version.as_uuid())
            .bind(digest.to_string())
            .execute(&mut **tx)
            .await
            .map_err(store_error)?;
        }
    }
    if let Some(request) = context.request {
        // The request must be approved, with what the policy in force asks for now.
        let row = sqlx::query(
            "SELECT r.state, \
                    (SELECT count(*) FROM publication_decisions d \
                     WHERE d.request_id = r.id AND d.verdict = 'APPROVED') AS approvals, \
                    coalesce(p.required_approvals, CASE n.environment WHEN 'prod' THEN 1 ELSE 0 END)::smallint AS needed \
             FROM publication_requests r \
             JOIN namespaces n ON n.tenant_id = r.tenant_id AND n.id = r.namespace_id \
             LEFT JOIN publication_policies p ON p.tenant_id = r.tenant_id AND p.namespace_id = r.namespace_id \
             WHERE r.tenant_id = $1 AND r.id = $2 AND r.workflow_id = $3 FOR UPDATE OF r",
        )
        .bind(tenant.as_uuid())
        .bind(request.as_uuid())
        .bind(workflow.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(store_error)?
        .ok_or(StoreError::MissingReference)?;
        let state: String = col(&row, "state")?;
        let approvals: i64 = col(&row, "approvals")?;
        let needed: i16 = col(&row, "needed")?;
        if state != "APPROVED" || approvals < i64::from(needed) {
            return Err(StoreError::Conflict(
                sagat_application::error::ConflictKind::RequestNotApproved,
            ));
        }
        sqlx::query(
            "UPDATE publication_requests \
             SET state = 'PUBLISHED', published_version_id = $3, published_by = $4, \
                 published_at = $5, updated_at = $5 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(request.as_uuid())
        .bind(version.as_uuid())
        .bind(actor)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;
    }
    if previous_active != Some(version) {
        let kind = if context.promoted_from.is_some() {
            ActivationKind::Promote
        } else {
            ActivationKind::Publish
        };
        sqlx::query(
            "INSERT INTO workflow_activations \
             (id, tenant_id, workflow_id, version_id, previous_version_id, kind, reason, actor, \
              request_id, promoted_from_version_id, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(ActivationId::generate().as_uuid())
        .bind(tenant.as_uuid())
        .bind(workflow.as_uuid())
        .bind(version.as_uuid())
        .bind(previous_active.map(WorkflowVersionId::as_uuid))
        .bind(kind.as_str())
        .bind(context.reason.as_deref())
        .bind(actor)
        .bind(context.request.map(RequestId::as_uuid))
        .bind(context.promoted_from.map(WorkflowVersionId::as_uuid))
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}

/// The policy in force for a namespace: what an administrator set, or the default of its
/// environment. `None` for a namespace that is not in the tenant.
pub(crate) async fn effective_policy(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<Option<ApprovalPolicy>, StoreError> {
    let Some(row) = sqlx::query(
        "SELECT n.environment, p.required_approvals FROM namespaces n \
         LEFT JOIN publication_policies p ON p.tenant_id = n.tenant_id AND p.namespace_id = n.id \
         WHERE n.tenant_id = $1 AND n.id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(store_error)?
    else {
        return Ok(None);
    };
    let required: Option<i16> = row.try_get("required_approvals").map_err(store_error)?;
    if let Some(required) = required {
        return ApprovalPolicy::new(u8::try_from(required).map_err(|_| corrupt("policy"))?)
            .map(Some)
            .map_err(|_| corrupt("policy"));
    }
    let environment: String = col(&row, "environment")?;
    Ok(Some(ApprovalPolicy::default_for(
        environment
            .parse::<sagat_domain::identity::Environment>()
            .map_err(|_| corrupt("environment"))?,
    )))
}

/// The run store serves publishing too, so one value backs every service of the control plane.
impl PublishingStore for crate::execution::PgRunStore {
    async fn namespace_by_id(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<Option<Namespace>, StoreError> {
        self.definitions.namespace_by_id(tenant, id).await
    }

    async fn namespace_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<bool, StoreError> {
        self.definitions.namespace_outside(tenant, id).await
    }

    async fn version_scripts(
        &self,
        tenant: TenantId,
        version: WorkflowVersionId,
    ) -> Result<Vec<String>, StoreError> {
        self.definitions.version_scripts(tenant, version).await
    }

    async fn request_outside(&self, tenant: TenantId, id: RequestId) -> Result<bool, StoreError> {
        self.definitions.request_outside(tenant, id).await
    }

    async fn stored_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Option<StoredPolicy>, StoreError> {
        self.definitions.stored_policy(tenant, namespace).await
    }

    async fn set_policy(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        policy: ApprovalPolicy,
        by: &str,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        self.definitions
            .set_policy(tenant, namespace, policy, by, now, audit)
            .await
    }

    async fn open_request(
        &self,
        request: &PublicationRequest,
        audit: &AuditRecord,
    ) -> Result<Opened, StoreError> {
        self.definitions.open_request(request, audit).await
    }

    async fn request(
        &self,
        tenant: TenantId,
        id: RequestId,
    ) -> Result<Option<PublicationRequest>, StoreError> {
        self.definitions.request(tenant, id).await
    }

    async fn requests(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        state: Option<RequestState>,
        before: Option<RequestId>,
        limit: u32,
    ) -> Result<Vec<PublicationRequest>, StoreError> {
        self.definitions
            .requests(tenant, workflow, state, before, limit)
            .await
    }

    async fn decide(
        &self,
        tenant: TenantId,
        id: RequestId,
        by: &Subject,
        verdict: Verdict,
        reason: &str,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<Decided, StoreError> {
        self.definitions
            .decide(tenant, id, by, verdict, reason, now, audit)
            .await
    }

    async fn withdraw(
        &self,
        tenant: TenantId,
        id: RequestId,
        by: &Subject,
        now: OffsetDateTime,
        audit: &AuditRecord,
    ) -> Result<Decided, StoreError> {
        self.definitions.withdraw(tenant, id, by, now, audit).await
    }

    async fn activations(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        before: Option<ActivationId>,
        limit: u32,
    ) -> Result<Vec<ActivationRecord>, StoreError> {
        self.definitions
            .activations(tenant, workflow, before, limit)
            .await
    }

    async fn switch(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
        to: u32,
        expected: Option<u32>,
        change: &NewActivation,
        triggers: &[TriggerRegistration],
        audit: &AuditRecord,
    ) -> Result<Switched, StoreError> {
        self.definitions
            .switch(tenant, workflow, to, expected, change, triggers, audit)
            .await
    }
}
