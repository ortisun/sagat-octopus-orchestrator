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
/// What the lease of a lake step needs from the store, read in the lease transaction (feature 035, D035-138).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LakeLeaseFacts {
    /// The limits of the connection at the revision the run was admitted with, not the revision in force.
    pub connection_limits: sagat_domain::database::LakeLimits,
    /// The number of the published version of the workflow the run is of.
    pub workflow_version: u32,
    /// When the first attempt of the step was leased, by the database's clock; `None` while no attempt has been.
    pub first_leased_at: Option<OffsetDateTime>,
}

/// One secret a connection names, selected under the current run/ACL locks (feature 035, D035-140).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedSecret {
    /// What the secret is for in the connection; the pin of the attempt is keyed by it.
    pub role: sagat_domain::database::SecretRole,
    /// The version the attempt is to use: the one it pinned, when it has pinned one.
    pub stored: crate::secrets::StoredSecret,
    /// Current authority even when the attempt has pinned an older retained credential.
    pub current_version: Uuid,
}

/// What the step is to do, with the secrets it needs. A SQL step has one statement and one secret; a lake step has
/// no statement and up to two secrets (the catalog's and the storage's), and which of them exist is the connection's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseWork {
    Sql {
        operation: sagat_domain::database::DatabaseOperation,
        secret: Box<SelectedSecret>,
    },
    Lake {
        secrets: Vec<SelectedSecret>,
    },
}

/// Metadata selected under current run/ACL locks. It is compared again after provider I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseInput {
    pub connection: ConnectionRecord,
    pub work: DatabaseWork,
}

impl DatabaseInput {
    /// The secrets of the step, in the order they are read from the provider and pinned.
    #[must_use]
    pub fn secrets(&self) -> &[SelectedSecret] {
        match &self.work {
            DatabaseWork::Sql { secret, .. } => std::slice::from_ref(&**secret),
            DatabaseWork::Lake { secrets } => secrets,
        }
    }
}

/// What a lake step is handed: the secrets of its connection that exist, each under its role.
#[derive(Debug)]
pub struct ResolvedLake {
    pub catalog: Option<crate::worker::ResolvedSecret>,
    pub storage: Option<crate::worker::ResolvedSecret>,
}

/// What the executor is handed for the work of its step.
#[derive(Debug)]
pub enum ResolvedGrant {
    Sql {
        operation: sagat_domain::database::DatabaseOperation,
        credential: crate::worker::ResolvedSecret,
    },
    Lake(ResolvedLake),
}

/// A lease-bound capability for the isolated executor. Provider routing stays in the server.
#[derive(Debug)]
pub struct ResolvedDatabase {
    pub connection: ConnectionRecord,
    pub grant: ResolvedGrant,
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
    /// `allowUnauthenticatedMetastore`: whether a Hive metastore may be used without TLS. Off unless the deployment says
    /// so, since it has neither transport security nor authentication.
    allow_unauthenticated_metastore: bool,
}
impl<S: ConnectionStore, C: Clock> ConnectionService<S, C> {
    pub fn new(store: S, clock: C) -> Self {
        Self {
            store,
            clock,
            allow_unauthenticated_metastore: false,
        }
    }
    /// Lets a connection use a Hive metastore with `tls: none` (the deployment's `allowUnauthenticatedMetastore`).
    #[must_use]
    pub fn with_unauthenticated_metastore_allowed(mut self, allowed: bool) -> Self {
        self.allow_unauthenticated_metastore = allowed;
        self
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
        write.config.validate().map_err(invalid_connection)?;
        if write.config.uses_unauthenticated_metastore() && !self.allow_unauthenticated_metastore {
            return Err(AppError::InvalidConnection {
                reason: "UNAUTHENTICATED_METASTORE_NOT_ALLOWED",
            });
        }
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

/// What a refused connection configuration says: the reason of a lake connection, which is a closed name, and the
/// general refusal for the rest.
pub fn invalid_connection(error: sagat_domain::database::DatabaseError) -> AppError {
    match error {
        sagat_domain::database::DatabaseError::Lake(reason) => AppError::InvalidConnection {
            reason: reason.reason(),
        },
        _ => AppError::NotAllowed("invalid database connection configuration"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::database::{DatabaseError, LakeConfigError};

    #[test]
    fn a_lake_refusal_keeps_its_reason_and_any_other_is_the_general_one() {
        assert_eq!(
            invalid_connection(DatabaseError::Lake(LakeConfigError::CredentialInUri)),
            AppError::InvalidConnection {
                reason: "CREDENTIAL_IN_URI"
            }
        );
        for other in [
            DatabaseError::InvalidHost,
            DatabaseError::InvalidLimits,
            DatabaseError::InvalidTls,
        ] {
            assert_eq!(
                invalid_connection(other),
                AppError::NotAllowed("invalid database connection configuration")
            );
        }
    }
}
