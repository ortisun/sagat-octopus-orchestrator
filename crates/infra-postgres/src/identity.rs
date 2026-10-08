//! `IdentityStore` implementation over PostgreSQL (T001-010, T001-019).

use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::identity::IdentityStore;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{
    Environment, Membership, Namespace, NamespaceId, Role, Slug, Subject, Tenant, TenantId,
    Workspace, WorkspaceId,
};
use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgRow;
use sqlx::{PgConnection, PgPool, Row};
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Identical cross-tenant denials from one actor are recorded at most once per this window.
pub const DEFAULT_DENIAL_WINDOW: Duration = Duration::from_secs(300);

/// Most denial events being written at the same time; beyond this they are dropped (and logged).
const MAX_PENDING_DENIALS: usize = 64;

/// PostgreSQL-backed identity store.
#[derive(Debug, Clone)]
pub struct PgIdentityStore {
    pool: PgPool,
    denial_window: Duration,
    denial_slots: Arc<Semaphore>,
}

impl PgIdentityStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            denial_window: DEFAULT_DENIAL_WINDOW,
            denial_slots: Arc::new(Semaphore::new(MAX_PENDING_DENIALS)),
        }
    }

    /// Overrides the deduplication window of denial events (tests use a short one).
    #[must_use]
    pub fn with_denial_window(mut self, window: Duration) -> Self {
        self.denial_window = window;
        self
    }

    /// Creates a tenant and its first administrator atomically.
    ///
    /// Provisioning is a platform operation (operators and the development seed), deliberately
    /// not part of the tenant-facing [`IdentityStore`] port.
    ///
    /// # Errors
    /// [`StoreError::Conflict`] when the tenant slug is taken.
    pub async fn provision_tenant(
        &self,
        tenant: &Tenant,
        first_admin: &Membership,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        sqlx::query("INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, $3)")
            .bind(tenant.id().as_uuid())
            .bind(tenant.slug().as_str())
            .bind(tenant.created_at())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
        insert_membership(&mut tx, first_admin).await?;
        // Creating a tenant and its first administrator is the most privileged thing this system
        // does, so it leaves a trace, atomically with the change itself.
        insert_audit(
            &mut tx,
            &AuditRecord::provisioning(
                tenant.id(),
                AuditAction::TenantProvisioned,
                ResourceType::Tenant,
                tenant.id().to_string(),
                "tenant provisioned by an operator or the development seed",
                tenant.created_at(),
            ),
        )
        .await?;
        insert_audit(
            &mut tx,
            &membership_granted(first_admin, "first administrator of a new tenant"),
        )
        .await?;
        tx.commit().await.map_err(store_error)
    }

    /// Adds or replaces nothing: inserts a membership, failing if the subject already has one.
    ///
    /// # Errors
    /// [`StoreError::Internal`] on a duplicate `(tenant, subject)`; callers reconcile explicitly.
    pub async fn insert_membership(&self, membership: &Membership) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        insert_membership(&mut tx, membership).await?;
        insert_audit(
            &mut tx,
            &membership_granted(membership, "membership added by an operator"),
        )
        .await?;
        tx.commit().await.map_err(store_error)
    }
}

/// The audit entry for a role being granted outside the tenant API.
fn membership_granted(membership: &Membership, reason: &str) -> AuditRecord {
    AuditRecord::provisioning(
        membership.tenant_id(),
        AuditAction::MembershipGranted,
        ResourceType::Membership,
        membership.subject().as_str(),
        format!("{reason} (role {})", membership.role()),
        membership.created_at(),
    )
}

async fn insert_membership(
    conn: &mut PgConnection,
    membership: &Membership,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO memberships (tenant_id, subject, role, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(membership.tenant_id().as_uuid())
    .bind(membership.subject().as_str())
    .bind(membership.role().as_str())
    .bind(membership.created_at())
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

async fn insert_audit(conn: &mut PgConnection, audit: &AuditRecord) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO audit_records \
         (id, tenant_id, actor, action, resource_type, resource_id, reason, trace_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(audit.id().as_uuid())
    .bind(audit.tenant_id().as_uuid())
    .bind(audit.actor().as_str())
    .bind(audit.action().as_str())
    .bind(audit.resource_type().as_str())
    .bind(audit.resource_id())
    .bind(audit.reason())
    .bind(audit.trace_id())
    .bind(audit.created_at())
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

/// Appends `audit` unless an identical event (same tenant, actor, action and resource) was already
/// recorded within `window`.
async fn insert_denial_once(
    pool: &PgPool,
    audit: &AuditRecord,
    window: Duration,
) -> Result<(), StoreError> {
    // Two identical events racing would both pass the NOT EXISTS check before either commits.
    // A transaction-scoped advisory lock on the event's identity serialises exactly those, so the
    // second one sees the first one's row.
    let key = format!(
        "{}|{}|{}|{}|{}",
        audit.tenant_id(),
        audit.actor(),
        audit.action(),
        audit.resource_type(),
        audit.resource_id()
    );
    let mut tx = pool.begin().await.map_err(store_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(&key)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
    sqlx::query(
        "INSERT INTO audit_records \
         (id, tenant_id, actor, action, resource_type, resource_id, reason, trace_id, created_at) \
         SELECT $1::uuid, $2::uuid, $3::text, $4::text, $5::text, $6::text, $7::text, $8::text, $9::timestamptz \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM audit_records \
             WHERE tenant_id = $2::uuid AND actor = $3::text AND action = $4::text \
               AND resource_type = $5::text AND resource_id = $6::text \
               AND created_at > $9::timestamptz - make_interval(secs => $10::double precision))",
    )
    .bind(audit.id().as_uuid())
    .bind(audit.tenant_id().as_uuid())
    .bind(audit.actor().as_str())
    .bind(audit.action().as_str())
    .bind(audit.resource_type().as_str())
    .bind(audit.resource_id())
    .bind(audit.reason())
    .bind(audit.trace_id())
    .bind(audit.created_at())
    .bind(window.as_secs_f64())
    .execute(&mut *tx)
    .await
    .map_err(store_error)?;
    tx.commit().await.map_err(store_error)
}

impl IdentityStore for PgIdentityStore {
    async fn find_tenant_by_slug(&self, slug: &Slug) -> Result<Option<Tenant>, StoreError> {
        let row = sqlx::query("SELECT id, slug, created_at FROM tenants WHERE slug = $1")
            .bind(slug.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
        row.as_ref().map(tenant_from_row).transpose()
    }

    async fn find_membership(
        &self,
        tenant: TenantId,
        subject: &Subject,
    ) -> Result<Option<Membership>, StoreError> {
        let row = sqlx::query(
            "SELECT tenant_id, subject, role, created_at FROM memberships \
             WHERE tenant_id = $1 AND subject = $2",
        )
        .bind(tenant.as_uuid())
        .bind(subject.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(membership_from_row).transpose()
    }

    async fn create_workspace(
        &self,
        workspace: &Workspace,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        sqlx::query(
            "INSERT INTO workspaces (id, tenant_id, slug, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(workspace.id().as_uuid())
        .bind(workspace.tenant_id().as_uuid())
        .bind(workspace.slug().as_str())
        .bind(workspace.created_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn get_workspace(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> Result<Option<Workspace>, StoreError> {
        let row = sqlx::query(
            "SELECT id, tenant_id, slug, created_at FROM workspaces \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(workspace_from_row).transpose()
    }

    async fn list_workspaces(
        &self,
        tenant: TenantId,
        after: Option<WorkspaceId>,
        limit: u32,
    ) -> Result<Vec<Workspace>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, slug, created_at FROM workspaces \
             WHERE tenant_id = $1 AND ($2::uuid IS NULL OR id > $2) \
             ORDER BY id LIMIT $3",
        )
        .bind(tenant.as_uuid())
        .bind(after.map(WorkspaceId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(workspace_from_row).collect()
    }

    async fn create_namespace(
        &self,
        namespace: &Namespace,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(store_error)?;
        sqlx::query(
            "INSERT INTO namespaces (id, tenant_id, workspace_id, slug, environment, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(namespace.id().as_uuid())
        .bind(namespace.tenant_id().as_uuid())
        .bind(namespace.workspace_id().as_uuid())
        .bind(namespace.slug().as_str())
        .bind(namespace.environment().as_str())
        .bind(namespace.created_at())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn get_namespace(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<Option<Namespace>, StoreError> {
        let row = sqlx::query(
            "SELECT id, tenant_id, workspace_id, slug, environment, created_at FROM namespaces \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        row.as_ref().map(namespace_from_row).transpose()
    }

    async fn list_namespaces(
        &self,
        tenant: TenantId,
        after: Option<NamespaceId>,
        limit: u32,
    ) -> Result<Vec<Namespace>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, workspace_id, slug, environment, created_at FROM namespaces \
             WHERE tenant_id = $1 AND ($2::uuid IS NULL OR id > $2) \
             ORDER BY id LIMIT $3",
        )
        .bind(tenant.as_uuid())
        .bind(after.map(NamespaceId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        rows.iter().map(namespace_from_row).collect()
    }

    // The two `*_exists_outside` queries are the only statements that intentionally look past the
    // caller's tenant. They return a bare boolean that the application uses to decide whether to
    // write an audit record; it is never exposed to API clients.
    async fn workspace_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workspaces WHERE id = $2 AND tenant_id <> $1)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn namespace_exists_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM namespaces WHERE id = $2 AND tenant_id <> $1)",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        // Off the response path and bounded: a flood of denials must neither slow the caller (which
        // would reveal that the identifier exists elsewhere) nor pile up work in the database.
        let Ok(slot) = Arc::clone(&self.denial_slots).try_acquire_owned() else {
            tracing::warn!(action = %audit.action(), "too many audit writes pending; dropping a denial event");
            return Ok(());
        };
        let pool = self.pool.clone();
        let audit = audit.clone();
        let window = self.denial_window;
        tokio::spawn(async move {
            let _slot = slot;
            if let Err(error) = insert_denial_once(&pool, &audit, window).await {
                tracing::error!(%error, "failed to record a cross-tenant denial");
            }
        });
        Ok(())
    }

    async fn ping(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }
}

// ----------------------------------------------------------------------------- row decoding

fn col<'r, T>(row: &'r PgRow, name: &str) -> Result<T, StoreError>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(name).map_err(store_error)
}

/// A stored value that no longer satisfies a domain rule means the database was changed behind the
/// application's back; surface it as an internal error instead of guessing.
fn corrupt(what: &str) -> StoreError {
    StoreError::Internal {
        detail: format!("stored {what} violates a domain invariant"),
    }
}

fn tenant_from_row(row: &PgRow) -> Result<Tenant, StoreError> {
    let slug: String = col(row, "slug")?;
    Ok(Tenant::restore(
        TenantId::new(col::<Uuid>(row, "id")?),
        Slug::parse(&slug).map_err(|_| corrupt("tenant slug"))?,
        col::<OffsetDateTime>(row, "created_at")?,
    ))
}

fn membership_from_row(row: &PgRow) -> Result<Membership, StoreError> {
    let subject: String = col(row, "subject")?;
    let role: String = col(row, "role")?;
    Ok(Membership::restore(
        TenantId::new(col::<Uuid>(row, "tenant_id")?),
        Subject::parse(&subject).map_err(|_| corrupt("subject"))?,
        role.parse::<Role>().map_err(|_| corrupt("role"))?,
        col::<OffsetDateTime>(row, "created_at")?,
    ))
}

fn workspace_from_row(row: &PgRow) -> Result<Workspace, StoreError> {
    let slug: String = col(row, "slug")?;
    Ok(Workspace::restore(
        WorkspaceId::new(col::<Uuid>(row, "id")?),
        TenantId::new(col::<Uuid>(row, "tenant_id")?),
        Slug::parse(&slug).map_err(|_| corrupt("workspace slug"))?,
        col::<OffsetDateTime>(row, "created_at")?,
    ))
}

fn namespace_from_row(row: &PgRow) -> Result<Namespace, StoreError> {
    let slug: String = col(row, "slug")?;
    let environment: String = col(row, "environment")?;
    Ok(Namespace::restore(
        NamespaceId::new(col::<Uuid>(row, "id")?),
        TenantId::new(col::<Uuid>(row, "tenant_id")?),
        WorkspaceId::new(col::<Uuid>(row, "workspace_id")?),
        Slug::parse(&slug).map_err(|_| corrupt("namespace slug"))?,
        environment
            .parse::<Environment>()
            .map_err(|_| corrupt("environment"))?,
        col::<OffsetDateTime>(row, "created_at")?,
    ))
}

// -------------------------------------------------------------------------- error mapping

/// Translates driver errors into the port's typed errors and records the root cause.
///
/// The application layer deliberately drops the detail (clients must never see it), so this is the
/// only place where an operator can learn *why* storage failed. Server messages carry no row
/// values (those live in a separate `DETAIL` field that is not logged).
fn store_error(error: sqlx::Error) -> StoreError {
    let mapped = classify(&error);
    match &mapped {
        StoreError::Unavailable { detail } => tracing::warn!(%detail, "storage unavailable"),
        StoreError::Internal { detail } => tracing::error!(%detail, "storage failure"),
        StoreError::Conflict(_) | StoreError::MissingReference => {}
    }
    mapped
}

fn classify(error: &sqlx::Error) -> StoreError {
    use sqlx::Error;
    let detail = error.to_string();
    match &error {
        Error::Database(db) => {
            let code = db.code();
            let code = code.as_deref().unwrap_or_default();
            match code {
                // unique_violation
                "23505" => match db.constraint() {
                    Some("tenants_slug_uniq") => StoreError::Conflict(ConflictKind::TenantSlug),
                    Some("workspaces_tenant_slug_uniq") => {
                        StoreError::Conflict(ConflictKind::WorkspaceSlug)
                    }
                    Some("namespaces_tenant_slug_uniq") => {
                        StoreError::Conflict(ConflictKind::NamespaceSlug)
                    }
                    _ => StoreError::Internal { detail },
                },
                // foreign_key_violation
                "23503" => StoreError::MissingReference,
                // Conditions that can clear on their own, so the caller may retry:
                //   08 connection exceptions        40 transaction rollback (deadlock,
                //   53 insufficient resources          serialization; fully undone)
                //   55 object not in prerequisite   57 operator intervention (admin shutdown,
                //      state (e.g. database not        statement-timeout cancellation,
                //      accepting connections)          server starting up)
                c if ["08", "40", "53", "55", "57"]
                    .iter()
                    .any(|class| c.starts_with(class)) =>
                {
                    StoreError::Unavailable { detail }
                }
                _ => StoreError::Internal { detail },
            }
        }
        Error::Io(_)
        | Error::Tls(_)
        | Error::PoolTimedOut
        | Error::PoolClosed
        | Error::WorkerCrashed => StoreError::Unavailable { detail },
        _ => StoreError::Internal { detail },
    }
}
