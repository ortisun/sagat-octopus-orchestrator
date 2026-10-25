//! Namespace connection catalog. External SQL and provider I/O belong to separate capabilities.
use crate::{
    authz::{NamespaceDirectory, require_namespace},
    clock::Clock,
    error::{AppError, StoreError},
    identity::RequestContext,
};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    database::{ConnectionConfig, ConnectionId, ConnectionStatus},
    identity::{NamespaceId, Permission, Principal, Slug, TenantId},
};
use std::future::Future;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRecord {
    pub id: ConnectionId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub name: Slug,
    pub revision: u32,
    /// Current authority, even when the config comes from a historical revision.
    pub status: ConnectionStatus,
    pub config: ConnectionConfig,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
/// Metadata selected under current run/ACL locks. It is compared again after provider I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseInput {
    pub connection: ConnectionRecord,
    pub operation: sagat_domain::database::DatabaseOperation,
    pub secret: crate::secrets::StoredSecret,
    /// Current authority even when the attempt has pinned an older retained credential.
    pub current_secret_version: Uuid,
}

/// A lease-bound capability for the isolated executor. Provider routing stays in the server.
#[derive(Debug)]
pub struct ResolvedDatabase {
    pub connection: ConnectionRecord,
    pub operation: sagat_domain::database::DatabaseOperation,
    pub credential: crate::worker::ResolvedSecret,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct ConnectionWrite {
    pub name: Slug,
    pub expected_revision: u32,
    pub request_id: Uuid,
    pub config: ConnectionConfig,
    pub status: ConnectionStatus,
}

pub trait ConnectionStore: NamespaceDirectory {
    /// Rechecks current membership, namespace, secret authority/ACL, receipt and expected revision
    /// while holding the catalog/secret management locks. Revision/head/audit commit together.
    fn write_connection(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: &ConnectionWrite,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<ConnectionRecord, StoreError>> + Send;
    fn connection(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &Slug,
        revision: Option<u32>,
    ) -> impl Future<Output = Result<ConnectionRecord, StoreError>> + Send;
    fn connections(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<ConnectionRecord>, StoreError>> + Send;
}
pub struct ConnectionService<S, C> {
    store: S,
    clock: C,
}
impl<S: ConnectionStore, C: Clock> ConnectionService<S, C> {
    pub fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }
    #[tracing::instrument(name = "database.catalog.write", skip_all)]
    pub async fn put(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: ConnectionWrite,
        ctx: &RequestContext,
    ) -> Result<ConnectionRecord, AppError> {
        principal.authorize(Permission::DatabaseManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        write
            .config
            .validate()
            .map_err(|_| AppError::NotAllowed("invalid database connection configuration"))?;
        if write.request_id.is_nil() || write.expected_revision >= i32::MAX as u32 {
            return Err(AppError::NotAllowed(
                "invalid database request identity or revision",
            ));
        }
        let audit = AuditRecord::by(
            principal,
            AuditAction::DatabaseConnectionChanged,
            ResourceType::DatabaseConnection,
            format!("{namespace}/{}", write.name),
            ctx.trace_id.clone(),
            self.clock.now(),
        );
        let start = std::time::Instant::now();
        let result = self
            .store
            .write_connection(principal, namespace, &write, &audit)
            .await;
        let outcome = if result.is_ok() { "success" } else { "refused" };
        metrics::counter!("database_catalog_writes_total", "outcome" => outcome).increment(1);
        metrics::histogram!("database_catalog_write_seconds").record(start.elapsed().as_secs_f64());
        Ok(result?)
    }
    pub async fn get(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &Slug,
        revision: Option<u32>,
        ctx: &RequestContext,
    ) -> Result<ConnectionRecord, AppError> {
        principal.authorize(Permission::NamespaceRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .connection(principal, namespace, name, revision)
            .await?)
    }
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<ConnectionRecord>, AppError> {
        principal.authorize(Permission::NamespaceRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self.store.connections(principal, namespace).await?)
    }
}

/// Credential fields have independent semantics and always own zeroized secret memory.
#[derive(Debug)]
pub struct DatabaseCredentials {
    pub username: sagat_domain::secrets::SecretValue,
    pub password: sagat_domain::secrets::SecretValue,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid managed database credential")]
pub struct InvalidDatabaseCredential;
/// Credential syntax belongs to an adapter. The output guard uses the same decoder as the runner.
pub trait DatabaseCredentialsCodec: Send + Sync {
    fn decode(
        &self,
        value: &sagat_domain::secrets::SecretValue,
    ) -> Result<DatabaseCredentials, InvalidDatabaseCredential>;
    fn contains_credential(
        &self,
        value: &sagat_domain::secrets::SecretValue,
        content: &[u8],
    ) -> Result<bool, InvalidDatabaseCredential>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid or oversized managed database result")]
pub struct InvalidDatabaseResult;
/// Validates retained managed output before it reaches the journal or blob store.
pub trait DatabaseResultValidator: Send + Sync {
    fn validate(
        &self,
        content: &[u8],
        limits: sagat_domain::database::DatabaseLimits,
    ) -> Result<(), InvalidDatabaseResult>;
}
