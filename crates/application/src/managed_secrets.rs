//! Recoverable provider provisioning. Commit intent, perform external I/O, then activate metadata.
use crate::{
    authz::require_namespace,
    clock::Clock,
    error::{AppError, StoreError},
    identity::RequestContext,
    secret_provider::{ProviderError, ProviderReference, SecretProvider},
    secrets::{SecretReader, SecretStore, SecretView, StoredSecret},
};
use ring::hmac;
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    identity::{NamespaceId, Permission, Principal},
    quota::QuotaResource,
    secrets::{SecretCipher, SecretName, SecretValue},
};
use std::{future::Future, sync::Arc};
use uuid::Uuid;

pub struct ManagedSecretWrite<'a> {
    pub name: &'a str,
    pub value: SecretValue,
    /// Zero creates, otherwise an optimistic logical-version fence.
    pub expected_version: u32,
    pub request_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct ProviderIntent {
    pub reference: ProviderReference,
    pub secret_id: Uuid,
    pub name: SecretName,
    pub actor: String,
    pub request_id: Uuid,
    pub expected_version: u32,
    pub binding: Vec<u8>,
    pub value_bytes: u32,
    pub activated: bool,
    pub source_version: Option<Uuid>,
}

pub trait ManagedSecretStore: SecretStore {
    fn provider_request(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        request: Uuid,
    ) -> impl Future<Output = Result<Option<ProviderIntent>, StoreError>> + Send;
    fn legacy_provider_source(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &SecretName,
        expected_version: u32,
    ) -> impl Future<Output = Result<StoredSecret, StoreError>> + Send;

    /// Requires a current administrator, a matching name/revision and available quotas. Commits
    /// the intent and audit atomically. A replay checks the binding before returning the receipt.
    fn prepare_provider_intent(
        &self,
        intent: &ProviderIntent,
        secret_limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<ProviderIntent, StoreError>> + Send;
    fn provider_intent(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        operation: Uuid,
    ) -> impl Future<Output = Result<ProviderIntent, StoreError>> + Send;
    /// Checks current membership and original logical identity/revision under the management lock.
    fn activate_provider_intent(
        &self,
        intent: &ProviderIntent,
        secret_limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<SecretView, StoreError>> + Send;
}

pub struct ManagedSecretService<S, C> {
    store: S,
    clock: C,
    provider: Arc<dyn SecretProvider>,
    binding_key: hmac::Key,
    legacy: Option<SecretReader>,
}

impl<S: ManagedSecretStore, C: Clock> ManagedSecretService<S, C> {
    pub fn new(
        store: S,
        clock: C,
        provider: Arc<dyn SecretProvider>,
        binding_key: SecretValue,
    ) -> Result<Self, AppError> {
        if binding_key.len() != 32 {
            return Err(AppError::NotAllowed(
                "secret intent binding key must contain 32 bytes",
            ));
        }
        Ok(Self {
            store,
            clock,
            provider,
            binding_key: hmac::Key::new(hmac::HMAC_SHA256, binding_key.expose()),
            legacy: None,
        })
    }

    fn binding_message(intent: &ProviderIntent, value: &SecretValue) -> Vec<u8> {
        // Only a transient in-process input to HMAC, never persisted, displayed or logged.
        // The value itself is fed separately, so this buffer contains metadata only.
        let mut message = b"sagat-secret-intent-v1\0".to_vec();
        for field in [
            intent.reference.tenant.to_string(),
            intent.reference.namespace.to_string(),
            intent.actor.clone(),
            intent.request_id.to_string(),
            intent.name.to_string(),
            intent.expected_version.to_string(),
            intent
                .source_version
                .map(|id| id.to_string())
                .unwrap_or_default(),
            value.len().to_string(),
        ] {
            message.extend_from_slice(&(field.len() as u64).to_be_bytes());
            message.extend_from_slice(field.as_bytes());
        }
        message
    }

    fn binding(&self, intent: &ProviderIntent, value: &SecretValue) -> hmac::Tag {
        let mut context = hmac::Context::with_key(&self.binding_key);
        context.update(&Self::binding_message(intent, value));
        context.update(value.expose());
        context.sign()
    }

    pub fn with_legacy_cipher(mut self, cipher: Arc<dyn SecretCipher>) -> Self {
        self.legacy = Some(SecretReader::local(cipher));
        self
    }

    pub async fn migrate(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &str,
        expected_version: u32,
        request_id: Uuid,
        ctx: &RequestContext,
    ) -> Result<SecretView, AppError> {
        principal.authorize(Permission::SecretManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let name =
            SecretName::parse(name).map_err(|_| AppError::NotAllowed("invalid secret name"))?;
        if let Some(receipt) = self
            .store
            .provider_request(principal, namespace, request_id)
            .await?
        {
            if receipt.name != name
                || receipt.expected_version != expected_version
                || receipt.source_version.is_none()
            {
                return Err(AppError::IdempotencyKeyReused);
            }
            if receipt.activated
                || self
                    .provider
                    .read(&receipt.reference)
                    .await
                    .map_err(provider_error)?
                    .is_some()
            {
                return self
                    .reconcile(principal, namespace, receipt.reference.operation, ctx)
                    .await;
            }
        }
        let source = self
            .store
            .legacy_provider_source(principal, namespace, &name, expected_version)
            .await?;
        let value = self
            .legacy
            .as_ref()
            .ok_or(AppError::NotAllowed(
                "legacy migration requires the local key ring",
            ))?
            .resolve(&source)
            .await?;
        let write = ManagedSecretWrite {
            name: name.as_str(),
            value,
            expected_version,
            request_id,
        };
        let intent = self
            .prepare_mode(principal, namespace, &write, Some(source.meta.id), ctx)
            .await?;
        self.finish_write(principal, namespace, intent, write.value, ctx)
            .await
    }

    pub async fn prepare(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: &ManagedSecretWrite<'_>,
        ctx: &RequestContext,
    ) -> Result<ProviderIntent, AppError> {
        self.prepare_mode(principal, namespace, write, None, ctx)
            .await
    }

    async fn prepare_mode(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: &ManagedSecretWrite<'_>,
        source_version: Option<Uuid>,
        ctx: &RequestContext,
    ) -> Result<ProviderIntent, AppError> {
        principal.authorize(Permission::SecretManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let name = SecretName::parse(write.name)
            .map_err(|_| AppError::NotAllowed("invalid secret name"))?;
        if write.request_id.is_nil() || write.expected_version >= i32::MAX as u32 {
            return Err(AppError::NotAllowed(
                "invalid secret request identity or version",
            ));
        }
        let mut intent = ProviderIntent {
            reference: ProviderReference {
                tenant: principal.tenant_id(),
                namespace,
                operation: Uuid::now_v7(),
            },
            secret_id: Uuid::now_v7(),
            name,
            actor: principal.subject().as_str().to_owned(),
            request_id: write.request_id,
            expected_version: write.expected_version,
            binding: Vec::new(),
            value_bytes: write.value.len() as u32,
            activated: false,
            source_version,
        };
        intent.binding = self.binding(&intent, &write.value).as_ref().to_vec();
        let audit = self.audit(
            principal,
            &intent,
            AuditAction::SecretProvisioningStarted,
            ctx,
        );
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::Secrets,
            )
            .await?;
        Ok(self
            .store
            .prepare_provider_intent(&intent, limit, &audit)
            .await?)
    }

    pub async fn put(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        write: ManagedSecretWrite<'_>,
        ctx: &RequestContext,
    ) -> Result<SecretView, AppError> {
        let intent = self.prepare(principal, namespace, &write, ctx).await?;
        self.finish_write(principal, namespace, intent, write.value, ctx)
            .await
    }

    async fn finish_write(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        intent: ProviderIntent,
        value: SecretValue,
        ctx: &RequestContext,
    ) -> Result<SecretView, AppError> {
        if !intent.activated
            && self
                .provider
                .read(&intent.reference)
                .await
                .map_err(provider_error)?
                .is_none()
        {
            match self.provider.create(&intent.reference, &value).await {
                Ok(()) | Err(ProviderError::Conflict) => {}
                Err(error) => return Err(provider_error(error)),
            }
        }
        self.reconcile(principal, namespace, intent.reference.operation, ctx)
            .await
    }

    pub async fn reconcile_request(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        request: Uuid,
        ctx: &RequestContext,
    ) -> Result<SecretView, AppError> {
        principal.authorize(Permission::SecretManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let intent = self
            .store
            .provider_request(principal, namespace, request)
            .await?
            .ok_or(AppError::NotFound)?;
        self.reconcile(principal, namespace, intent.reference.operation, ctx)
            .await
    }

    /// A provider receipt can finish a write without resubmitting plaintext. If none exists, ask
    /// for secure resubmission; no plaintext retry queue is created.
    pub async fn reconcile(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        operation: Uuid,
        ctx: &RequestContext,
    ) -> Result<SecretView, AppError> {
        principal.authorize(Permission::SecretManage)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        let intent = self
            .store
            .provider_intent(principal, namespace, operation)
            .await?;
        if !intent.activated {
            let value = self
                .provider
                .read(&intent.reference)
                .await
                .map_err(provider_error)?
                .ok_or(AppError::StateConflict("secret value must be resubmitted"))?;
            // HMAC comparison is constant time and does not disclose stored credential equality.
            let expected = self.binding(&intent, &value);
            if !constant_time_equal(expected.as_ref(), &intent.binding) {
                return Err(AppError::StateConflict(
                    "secret provider receipt does not match its intent",
                ));
            }
        }
        let audit = self.audit(principal, &intent, AuditAction::SecretStored, ctx);
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::Secrets,
            )
            .await?;
        Ok(self
            .store
            .activate_provider_intent(&intent, limit, &audit)
            .await?)
    }

    fn audit(
        &self,
        principal: &Principal,
        intent: &ProviderIntent,
        action: AuditAction,
        ctx: &RequestContext,
    ) -> AuditRecord {
        AuditRecord::by(
            principal,
            action,
            ResourceType::Secret,
            format!("{}/{}", intent.reference.namespace, intent.name),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("openbao operation {}", intent.reference.operation))
    }
}

pub(crate) fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0_u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

pub fn provider_error(error: ProviderError) -> AppError {
    match error {
        ProviderError::Unavailable => AppError::Unavailable,
        ProviderError::Denied => AppError::NotAllowed("secret provider denied access"),
        ProviderError::Conflict => AppError::StateConflict("secret provider operation conflicts"),
        ProviderError::Configuration | ProviderError::Rejected | ProviderError::InvalidResponse => {
            AppError::NotAllowed("secret provider is not ready for this operation")
        }
    }
}
