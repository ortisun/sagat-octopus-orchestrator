//! Secrets: storing them, deciding who may use them, and (in `worker`) handing them to a worker for
//! the length of one leased attempt (FR-009-002, FR-009-003, FR-009-004).
//!
//! What holds throughout:
//!
//! * A value enters through [`SecretService::put`] and leaves only through the lease-bound
//!   resolution of [`WorkerService`](crate::worker::WorkerService). No API returns it, no audit
//!   record, log line, journal event, error or `Debug` output contains it.
//! * It is stored sealed. The associated data binds the ciphertext to its tenant, namespace, name
//!   and row, so it opens nowhere else.
//! * A workflow that declares a secret can be published only by someone who may use it, and the
//!   publisher is checked again when a worker asks for the value (*ACL race when renewing a
//!   token*): losing access stops the next resolution, not only the next publish.

use std::future::Future;
use std::sync::Arc;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, Subject, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::{
    SealedSecret, SecretCipher, SecretMeta, SecretName, SecretValue, associated_data,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;

/// A stored secret: what may be shown, and the sealed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSecret {
    pub meta: SecretMeta,
    pub sealed: SealedSecret,
}

/// What a secret looks like from outside: never the value, never the ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretView {
    pub name: String,
    pub version: u32,
    pub key_id: String,
    pub created_by: String,
    pub updated_at: OffsetDateTime,
}

impl From<&SecretMeta> for SecretView {
    fn from(meta: &SecretMeta) -> Self {
        Self {
            name: meta.name.to_string(),
            version: meta.version,
            key_id: meta.key_id.clone(),
            created_by: meta.created_by.clone(),
            updated_at: meta.created_at,
        }
    }
}

/// Whether one secret named by a workflow exists, and whether a given subject is on its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretState {
    Missing,
    Present { granted: bool },
}

/// The access lists of the secrets a workflow names.
pub trait SecretAccessLookup: Send + Sync {
    /// For each of `names` in `namespace`: whether it exists, and whether `subject` is on its list.
    fn secret_states(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        names: &[SecretName],
        subject: &str,
    ) -> impl Future<Output = Result<Vec<(SecretName, SecretState)>, StoreError>> + Send;
}

/// Why a workflow cannot reference the secrets it names.
///
/// Whether the secret does not exist or may not be used is deliberately not said: a publisher who
/// cannot use a secret must not be able to learn which names exist in the namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretProblem(pub SecretName);

/// Checks that `role` (holding `subject`'s place in the tenant) may use every secret in `states`.
///
/// # Errors
/// The first name that does not exist or may not be used.
pub fn check_usable(
    role: Option<Role>,
    states: &[(SecretName, SecretState)],
) -> Result<(), SecretProblem> {
    for (name, state) in states {
        let usable = match (role, state) {
            (Some(role), SecretState::Present { granted }) => {
                sagat_domain::secrets::may_use(role, *granted)
            }
            _ => false,
        };
        if !usable {
            return Err(SecretProblem(name.clone()));
        }
    }
    Ok(())
}

/// One secret a step declares, as the store found it for the publisher of the workflow.
#[derive(Debug, Clone)]
pub struct SecretInput {
    pub name: SecretName,
    /// `None` when the secret does not exist (any more).
    pub stored: Option<StoredSecret>,
    /// Whether the publisher is on the secret's access list.
    pub granted: bool,
}

/// What a step's declared secrets resolve to, read in the step's own transaction.
#[derive(Debug, Clone)]
pub struct SecretInputs {
    /// Who published the version of the workflow the run is pinned to.
    pub publisher: String,
    /// The publisher's role in the tenant *now*; `None` when they are no longer a member.
    pub publisher_role: Option<Role>,
    pub items: Vec<SecretInput>,
}

/// The version being stored.
#[derive(Debug, Clone)]
pub struct NewVersion {
    /// The row of this version: part of the associated data the value was sealed with.
    pub row_id: Uuid,
    pub sealed: SealedSecret,
    pub created_by: String,
}

/// Storage port of the secrets. Implemented by `sagat-infra-postgres`.
pub trait SecretStore: NamespaceDirectory + QuotaLookup {
    /// Stores a new version of the secret, creating the secret if it does not exist (which counts
    /// against the namespace's quota), in one transaction with its audit record. Old versions beyond
    /// the few kept are removed.
    fn put_version(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        version: &NewVersion,
        secrets_limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<SecretMeta, StoreError>> + Send;

    /// The current version of every secret of a namespace, without values.
    fn list_secrets(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<SecretMeta>, StoreError>> + Send;

    /// Removes the secret, its versions and its access list. `false` when it did not exist.
    fn delete_secret(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Puts `subject` on the secret's access list. `false` when the secret does not exist.
    fn grant_access(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        subject: &str,
        granted_by: &str,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Takes `subject` off the list. `false` when the secret or the entry did not exist.
    fn revoke_access(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        subject: &str,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The access list of a secret; `None` when the secret does not exist.
    fn access_list(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
    ) -> impl Future<Output = Result<Option<Vec<String>>, StoreError>> + Send;

    /// The names of the secrets of the namespace whose access list holds `subject`.
    fn names_usable_by(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        subject: &str,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// Versions sealed under a key other than `active_key`, any tenant, in the order of their row
    /// ids, starting after `after`. For the operator's re-sealing job after a key rotation: the
    /// cursor lets the job step over a version it cannot open instead of meeting it again.
    fn versions_not_sealed_under(
        &self,
        active_key: &str,
        after: Option<Uuid>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StoredSecret>, StoreError>> + Send;

    /// Replaces the sealed value of a version if it is still sealed under `expected_key`; `false`
    /// when someone else got there first. The value is unchanged, only its envelope.
    fn replace_sealed(
        &self,
        row_id: Uuid,
        expected_key: &str,
        sealed: &SealedSecret,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

/// What one batch of re-sealing did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResealBatch {
    /// Versions now sealed under the active key.
    pub moved: u32,
    /// Versions that no key of the ring opens, left as they were.
    pub failed: u32,
    /// Where to continue; `None` when there is nothing after this batch.
    pub next: Option<Uuid>,
}

/// Administers secrets: store, rotate, delete, and decide who may use them.
pub struct SecretService<S, C> {
    store: S,
    clock: C,
    cipher: Arc<dyn SecretCipher>,
}

impl<S, C> SecretService<S, C>
where
    S: SecretStore,
    C: Clock,
{
    #[must_use]
    pub fn new(store: S, clock: C, cipher: Arc<dyn SecretCipher>) -> Self {
        Self {
            store,
            clock,
            cipher,
        }
    }

    /// Stores `value` as the next version of the secret `name` of `namespace` (creating it).
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`] for a namespace the tenant does not have;
    /// [`AppError::NotAllowed`] for a name or value that cannot be used;
    /// [`AppError::QuotaExceeded`] when the namespace holds as many secrets as it may.
    pub async fn put(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &str,
        value: Vec<u8>,
        ctx: &RequestContext,
    ) -> Result<SecretView, AppError> {
        principal.authorize(Permission::SecretManage)?;
        let name = SecretName::parse(name).map_err(|_| AppError::NotAllowed(NAME_RULE))?;
        let value = SecretValue::new(value)
            .map_err(|_| AppError::NotAllowed("a secret value is 1 to 16384 bytes"))?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;

        let tenant = principal.tenant_id();
        let row_id = Uuid::now_v7();
        let key_id = self.cipher.active_key_id().to_owned();
        let aad = associated_data(tenant, namespace, &name, row_id, &key_id);
        let sealed = self
            .cipher
            .seal(&value, &aad)
            .map_err(|_| AppError::Internal)?;
        if sealed.key_id != key_id {
            // The key the associated data names is not the one that sealed: it would never open.
            return Err(AppError::Internal);
        }
        let limit = self
            .store
            .effective_limit(tenant, Some(namespace), QuotaResource::Secrets)
            .await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::SecretStored,
            ResourceType::Secret,
            format!("{namespace}/{name}"),
            ctx.trace_id.clone(),
            self.clock.now(),
        );
        let meta = self
            .store
            .put_version(
                tenant,
                namespace,
                &name,
                &NewVersion {
                    row_id,
                    sealed,
                    created_by: principal.subject().as_str().to_owned(),
                },
                limit,
                &audit,
            )
            .await?;
        Ok(SecretView::from(&meta))
    }

    /// The secrets of a namespace, by name, without values. An administrator sees all of them;
    /// anybody else sees the ones whose access list names them, which is what they need to write a
    /// workflow and no more: a developer who lists every name can ask "does `payments-key` exist?"
    /// of the publish check, and the point of answering a missing secret and a forbidden one alike
    /// is that they cannot.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<SecretView>, AppError> {
        principal.authorize(Permission::SecretRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let tenant = principal.tenant_id();
        let all = self.store.list_secrets(tenant, namespace).await?;
        if principal.authorize(Permission::SecretManage).is_ok() {
            return Ok(all.iter().map(SecretView::from).collect());
        }
        let usable = self
            .store
            .names_usable_by(tenant, namespace, principal.subject().as_str())
            .await?;
        Ok(all
            .iter()
            .filter(|secret| usable.iter().any(|name| name == secret.name.as_str()))
            .map(SecretView::from)
            .collect())
    }

    /// Removes a secret. Workflows that declare it stop resolving it at once.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`] for a namespace the tenant does not have or a
    /// secret that does not exist.
    pub async fn delete(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::SecretManage)?;
        let name = SecretName::parse(name).map_err(|_| AppError::NotFound)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let audit = self.audit(
            principal,
            AuditAction::SecretDeleted,
            namespace,
            &name,
            None,
            ctx,
        );
        if self
            .store
            .delete_secret(principal.tenant_id(), namespace, &name, &audit)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// Lets `subject` reference the secret from workflows they publish.
    ///
    /// # Errors
    /// As [`Self::delete`], and [`AppError::NotAllowed`] for a subject that is not an identity.
    pub async fn grant(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &str,
        subject: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::SecretManage)?;
        let name = SecretName::parse(name).map_err(|_| AppError::NotFound)?;
        let subject = Subject::parse(subject)
            .map_err(|_| AppError::NotAllowed("the subject is not a valid identity"))?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let audit = self.audit(
            principal,
            AuditAction::SecretAccessGranted,
            namespace,
            &name,
            Some(subject.as_str()),
            ctx,
        );
        if self
            .store
            .grant_access(
                principal.tenant_id(),
                namespace,
                &name,
                subject.as_str(),
                principal.subject().as_str(),
                &audit,
            )
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// Takes `subject` off the secret's list.
    ///
    /// # Errors
    /// As [`Self::delete`]; also when the subject was not on the list.
    pub async fn revoke(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &str,
        subject: &str,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::SecretManage)?;
        let name = SecretName::parse(name).map_err(|_| AppError::NotFound)?;
        let subject = Subject::parse(subject).map_err(|_| AppError::NotFound)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let audit = self.audit(
            principal,
            AuditAction::SecretAccessRevoked,
            namespace,
            &name,
            Some(subject.as_str()),
            ctx,
        );
        if self
            .store
            .revoke_access(
                principal.tenant_id(),
                namespace,
                &name,
                subject.as_str(),
                &audit,
            )
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// Who is on the secret's list.
    ///
    /// # Errors
    /// As [`Self::delete`].
    pub async fn access(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &str,
        ctx: &RequestContext,
    ) -> Result<Vec<String>, AppError> {
        principal.authorize(Permission::SecretManage)?;
        let name = SecretName::parse(name).map_err(|_| AppError::NotFound)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        self.store
            .access_list(principal.tenant_id(), namespace, &name)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Re-seals up to `limit` versions, after the cursor `after`, that are sealed under a key other
    /// than the active one, so a retired key can be removed from the ring. Run by the operator after
    /// rotating the key ring; safe to repeat and to run from several instances.
    ///
    /// A version that cannot be opened with any key of the ring (the ring no longer holds the key
    /// it was sealed under) is left as it is and counted in [`ResealBatch::failed`]: one bad row
    /// does not stop the rotation of everybody else's.
    ///
    /// # Errors
    /// [`AppError::Unavailable`] and [`AppError::Internal`] when the store fails; what was moved
    /// before stays moved.
    pub async fn reseal(&self, after: Option<Uuid>, limit: u32) -> Result<ResealBatch, AppError> {
        let active = self.cipher.active_key_id().to_owned();
        let stale = self
            .store
            .versions_not_sealed_under(&active, after, limit)
            .await?;
        let next = if stale.len() >= usize::try_from(limit).unwrap_or(usize::MAX) {
            stale.last().map(|last| last.meta.id)
        } else {
            None
        };
        let (mut moved, mut failed) = (0, 0);
        for old in stale {
            let meta = &old.meta;
            let aad = associated_data(
                meta.tenant_id,
                meta.namespace_id,
                &meta.name,
                meta.id,
                &old.sealed.key_id,
            );
            let Ok(value) = self.cipher.open(&old.sealed, &aad) else {
                tracing::error!(
                    key = %old.sealed.key_id,
                    version = %meta.id,
                    "a stored secret cannot be opened with the key ring; it is left as it is"
                );
                failed += 1;
                continue;
            };
            let new_aad = associated_data(
                meta.tenant_id,
                meta.namespace_id,
                &meta.name,
                meta.id,
                &active,
            );
            let sealed = self
                .cipher
                .seal(&value, &new_aad)
                .map_err(|_| AppError::Internal)?;
            let audit = AuditRecord::engine(
                meta.tenant_id,
                AuditAction::SecretStored,
                ResourceType::Secret,
                format!("{}/{}", meta.namespace_id, meta.name),
                format!("re-sealed version {} under key {active}", meta.version),
                None,
                self.clock.now(),
            );
            if self
                .store
                .replace_sealed(meta.id, &old.sealed.key_id, &sealed, &audit)
                .await?
            {
                moved += 1;
            }
        }
        Ok(ResealBatch {
            moved,
            failed,
            next,
        })
    }

    fn audit(
        &self,
        principal: &Principal,
        action: AuditAction,
        namespace: NamespaceId,
        name: &SecretName,
        subject: Option<&str>,
        ctx: &RequestContext,
    ) -> AuditRecord {
        let record = AuditRecord::by(
            principal,
            action,
            ResourceType::Secret,
            format!("{namespace}/{name}"),
            ctx.trace_id.clone(),
            self.clock.now(),
        );
        match subject {
            Some(subject) => record.with_reason(format!("subject {subject}")),
            None => record,
        }
    }
}

const NAME_RULE: &str =
    "a secret name is 1 to 64 lower-case letters, digits, '.', '_' or '-', starting with a letter";
