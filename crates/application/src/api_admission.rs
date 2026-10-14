//! Canonical namespace resolution and durable admission for public API requests.
//!
//! Authentication remains at the HTTP boundary. This service then authorizes the exact use case,
//! resolves its stored namespace inside the authenticated tenant, and consumes one durable API
//! token. Callers must invoke it once, after typed request parsing and before any mutation or
//! streaming body is started.
//!
//! The charged inventory is closed by [`ApiOperation`]: namespace reads; workflow definition and
//! publication operations; run start/read/inspect/operate/signal; namespace quotas, bindings,
//! secrets and egress; events, endpoints, subscriptions and connectors; scripts; and namespace
//! operations. Tenant-wide identity/workspace/namespace collections and tenant quotas, health,
//! version, unfiltered audit, public webhooks, tenant-deduplicated script artifacts, worker RPC and
//! script-runner callbacks are exempt. A future namespace-filtered audit operation must join the
//! charged inventory using a canonically resolved namespace.

use std::future::Future;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::WorkflowId;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::versioning::RequestId;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::identity::RequestContext;
use crate::quotas::{NamespaceApiAdmission, RateDecision};

/// The closed authorization choices used by namespace-scoped public HTTP operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiOperation {
    NamespaceRead,
    DefinitionRead,
    DefinitionWrite,
    DefinitionPublish,
    RunStart,
    RunRead,
    PayloadInspect,
    RunOperate,
    RunRerun,
    RunSignal,
    SagaOverride,
    QuotaRead,
    QuotaManage,
    SecretRead,
    SecretManage,
    EgressManage,
    WorkerBind,
    EventPublish,
    EventRead,
    EventManage,
    ScriptRead,
    ScriptWrite,
    ScriptPublish,
    OpsRead,
    WorkerManage,
    PublicationManage,
    PublicationApprove,
    VersionActivate,
    Promotion,
}

impl ApiOperation {
    const fn permissions(self) -> &'static [Permission] {
        use Permission::{
            DefinitionPublish, DefinitionRead, DefinitionWrite, EgressManage, EventManage,
            EventPublish, EventRead, NamespaceRead, OpsRead, PublicationApprove, PublicationManage,
            QuotaManage, QuotaRead, RunOperate, RunPayloadInspect, RunRead, RunSignal, RunStart,
            SagaOverride, ScriptPublish, ScriptRead, ScriptWrite, SecretManage, SecretRead,
            VersionActivate, WorkerBind, WorkerManage,
        };
        match self {
            Self::NamespaceRead => &[NamespaceRead],
            Self::DefinitionRead => &[DefinitionRead],
            Self::DefinitionWrite => &[DefinitionWrite],
            Self::DefinitionPublish => &[DefinitionPublish],
            Self::RunStart => &[RunStart],
            Self::RunRead => &[RunRead],
            Self::PayloadInspect => &[RunRead, RunPayloadInspect],
            Self::RunOperate => &[RunOperate],
            Self::RunRerun => &[RunOperate, RunStart],
            Self::RunSignal => &[RunSignal],
            Self::SagaOverride => &[SagaOverride],
            Self::QuotaRead => &[QuotaRead],
            Self::QuotaManage => &[QuotaManage],
            Self::SecretRead => &[SecretRead],
            Self::SecretManage => &[SecretManage],
            Self::EgressManage => &[EgressManage],
            Self::WorkerBind => &[WorkerBind],
            Self::EventPublish => &[EventPublish],
            Self::EventRead => &[EventRead],
            Self::EventManage => &[EventManage],
            Self::ScriptRead => &[ScriptRead],
            Self::ScriptWrite => &[ScriptWrite],
            Self::ScriptPublish => &[ScriptPublish],
            Self::OpsRead => &[OpsRead],
            Self::WorkerManage => &[WorkerManage],
            Self::PublicationManage => &[PublicationManage],
            Self::PublicationApprove => &[PublicationApprove],
            Self::VersionActivate => &[VersionActivate],
            Self::Promotion => &[DefinitionPublish, DefinitionWrite],
        }
    }
}

/// A typed resource from which the authoritative namespace is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiTarget {
    Namespace(NamespaceId),
    NamespaceSlug(Slug),
    Workflow(WorkflowId),
    Run(RunId),
    PublicationRequest(RequestId),
    /// Both sides are authorized in the tenant; only the destination bucket is charged.
    Promotion {
        source: WorkflowId,
        destination: NamespaceId,
    },
}

/// Canonical, tenant-filtered namespace lookups needed before durable admission.
pub trait ApiAdmissionStore: NamespaceDirectory + NamespaceApiAdmission {
    fn namespace_id_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> impl Future<Output = Result<Option<NamespaceId>, StoreError>> + Send;

    fn workflow_namespace(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
    ) -> impl Future<Output = Result<Option<NamespaceId>, StoreError>> + Send;

    fn workflow_exists_outside(
        &self,
        tenant: TenantId,
        workflow: WorkflowId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn run_namespace(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<NamespaceId>, StoreError>> + Send;

    fn run_exists_outside(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn publication_request_namespace(
        &self,
        tenant: TenantId,
        request: RequestId,
    ) -> impl Future<Output = Result<Option<NamespaceId>, StoreError>> + Send;

    fn publication_request_exists_outside(
        &self,
        tenant: TenantId,
        request: RequestId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

#[derive(Debug, Clone)]
pub struct ApiAdmissionService<S, C> {
    store: S,
    clock: C,
}

impl<S, C> ApiAdmissionService<S, C>
where
    S: ApiAdmissionStore,
    C: Clock,
{
    pub const fn new(store: S, clock: C) -> Self {
        Self { store, clock }
    }

    /// Authorizes, canonically resolves and consumes exactly one namespace API token.
    ///
    /// A denied role or foreign/missing resource consumes no token. Once admitted, later domain
    /// validation, conflict or idempotent replay still represents one authenticated API request.
    pub async fn admit(
        &self,
        principal: &Principal,
        operation: ApiOperation,
        target: ApiTarget,
        ctx: &RequestContext,
    ) -> Result<NamespaceId, AppError> {
        for permission in operation.permissions() {
            principal.authorize(*permission)?;
        }

        let tenant = principal.tenant_id();
        let namespace = match target {
            ApiTarget::Namespace(namespace) => {
                require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
                namespace
            }
            ApiTarget::NamespaceSlug(slug) => self
                .store
                .namespace_id_by_slug(tenant, &slug)
                .await?
                .ok_or(AppError::NotFound)?,
            ApiTarget::Workflow(workflow) => {
                self.resolve_workflow(principal, workflow, ctx).await?
            }
            ApiTarget::Run(run) => self.resolve_run(principal, run, ctx).await?,
            ApiTarget::PublicationRequest(request) => {
                self.resolve_request(principal, request, ctx).await?
            }
            ApiTarget::Promotion {
                source,
                destination,
            } => {
                self.resolve_workflow(principal, source, ctx).await?;
                require_namespace(&self.store, &self.clock, principal, destination, ctx).await?;
                destination
            }
        };

        self.admit_resolved_namespace(tenant, namespace).await?;
        Ok(namespace)
    }

    /// Consumes one token for a namespace already authorized and canonically resolved by another
    /// application use case. This narrow entry point exists for payload inspection, whose prepare
    /// phase must resolve the run without reading its payload before admission.
    pub async fn admit_resolved_namespace(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<(), AppError> {
        match self
            .store
            .admit_namespace_request(tenant, namespace)
            .await?
        {
            RateDecision::Admitted => {
                metrics::counter!("api_namespace_rate_admission_total", "outcome" => "admitted")
                    .increment(1);
                Ok(())
            }
            RateDecision::Refused { retry_after_secs } => {
                metrics::counter!("api_namespace_rate_admission_total", "outcome" => "refused")
                    .increment(1);
                Err(AppError::Busy { retry_after_secs })
            }
        }
    }

    async fn resolve_workflow(
        &self,
        principal: &Principal,
        workflow: WorkflowId,
        ctx: &RequestContext,
    ) -> Result<NamespaceId, AppError> {
        if let Some(namespace) = self
            .store
            .workflow_namespace(principal.tenant_id(), workflow)
            .await?
        {
            return Ok(namespace);
        }
        if let Ok(true) = self
            .store
            .workflow_exists_outside(principal.tenant_id(), workflow)
            .await
        {
            self.record_cross_tenant(principal, ResourceType::Workflow, workflow.to_string(), ctx)
                .await;
        }
        Err(AppError::NotFound)
    }

    async fn resolve_run(
        &self,
        principal: &Principal,
        run: RunId,
        ctx: &RequestContext,
    ) -> Result<NamespaceId, AppError> {
        if let Some(namespace) = self.store.run_namespace(principal.tenant_id(), run).await? {
            return Ok(namespace);
        }
        if let Ok(true) = self
            .store
            .run_exists_outside(principal.tenant_id(), run)
            .await
        {
            self.record_cross_tenant(principal, ResourceType::Run, run.to_string(), ctx)
                .await;
        }
        Err(AppError::NotFound)
    }

    async fn resolve_request(
        &self,
        principal: &Principal,
        request: RequestId,
        ctx: &RequestContext,
    ) -> Result<NamespaceId, AppError> {
        if let Some(namespace) = self
            .store
            .publication_request_namespace(principal.tenant_id(), request)
            .await?
        {
            return Ok(namespace);
        }
        if let Ok(true) = self
            .store
            .publication_request_exists_outside(principal.tenant_id(), request)
            .await
        {
            self.record_cross_tenant(
                principal,
                ResourceType::PublicationRequest,
                request.to_string(),
                ctx,
            )
            .await;
        }
        Err(AppError::NotFound)
    }

    async fn record_cross_tenant(
        &self,
        principal: &Principal,
        resource_type: ResourceType,
        resource_id: String,
        ctx: &RequestContext,
    ) {
        let record = AuditRecord::by(
            principal,
            AuditAction::CrossTenantAccessDenied,
            resource_type,
            resource_id,
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason("resource not accessible from this tenant");
        if self.store.record_denial(&record).await.is_err() {
            tracing::error!(
                action = %record.action(),
                "failed to record cross-tenant API denial"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use sagat_domain::definition::WorkflowId;
    use sagat_domain::execution::RunId;
    use sagat_domain::identity::{Permission, Role, Subject};
    use sagat_domain::versioning::RequestId;
    use time::OffsetDateTime;

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct Store(Arc<Mutex<State>>);

    #[derive(Debug)]
    struct State {
        namespaces: Vec<(TenantId, NamespaceId)>,
        slug: Option<(TenantId, Slug, NamespaceId)>,
        workflow: Option<(TenantId, WorkflowId, NamespaceId)>,
        run: Option<(TenantId, RunId, NamespaceId)>,
        request: Option<(TenantId, RequestId, NamespaceId)>,
        decision: Result<RateDecision, StoreError>,
        charges: Vec<(TenantId, NamespaceId)>,
        denials: usize,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                namespaces: Vec::new(),
                slug: None,
                workflow: None,
                run: None,
                request: None,
                decision: Ok(RateDecision::Admitted),
                charges: Vec::new(),
                denials: 0,
            }
        }
    }

    impl NamespaceDirectory for Store {
        async fn namespace_exists(
            &self,
            tenant: TenantId,
            namespace: NamespaceId,
        ) -> Result<bool, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .namespaces
                .contains(&(tenant, namespace)))
        }

        async fn namespace_exists_outside(
            &self,
            tenant: TenantId,
            namespace: NamespaceId,
        ) -> Result<bool, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .namespaces
                .iter()
                .any(|entry| entry.0 != tenant && entry.1 == namespace))
        }

        async fn record_denial(&self, _audit: &AuditRecord) -> Result<(), StoreError> {
            self.0.lock().unwrap().denials += 1;
            Ok(())
        }
    }

    impl NamespaceApiAdmission for Store {
        async fn admit_namespace_request(
            &self,
            tenant: TenantId,
            namespace: NamespaceId,
        ) -> Result<RateDecision, StoreError> {
            let mut state = self.0.lock().unwrap();
            state.charges.push((tenant, namespace));
            state.decision.clone()
        }
    }

    impl ApiAdmissionStore for Store {
        async fn namespace_id_by_slug(
            &self,
            tenant: TenantId,
            slug: &Slug,
        ) -> Result<Option<NamespaceId>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .slug
                .as_ref()
                .filter(|entry| entry.0 == tenant && &entry.1 == slug)
                .map(|entry| entry.2))
        }

        async fn workflow_namespace(
            &self,
            tenant: TenantId,
            workflow: WorkflowId,
        ) -> Result<Option<NamespaceId>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .workflow
                .filter(|entry| entry.0 == tenant && entry.1 == workflow)
                .map(|entry| entry.2))
        }

        async fn workflow_exists_outside(
            &self,
            tenant: TenantId,
            workflow: WorkflowId,
        ) -> Result<bool, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .workflow
                .is_some_and(|entry| entry.0 != tenant && entry.1 == workflow))
        }

        async fn run_namespace(
            &self,
            tenant: TenantId,
            run: RunId,
        ) -> Result<Option<NamespaceId>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .run
                .filter(|entry| entry.0 == tenant && entry.1 == run)
                .map(|entry| entry.2))
        }

        async fn run_exists_outside(
            &self,
            tenant: TenantId,
            run: RunId,
        ) -> Result<bool, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .run
                .is_some_and(|entry| entry.0 != tenant && entry.1 == run))
        }

        async fn publication_request_namespace(
            &self,
            tenant: TenantId,
            request: RequestId,
        ) -> Result<Option<NamespaceId>, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .request
                .filter(|entry| entry.0 == tenant && entry.1 == request)
                .map(|entry| entry.2))
        }

        async fn publication_request_exists_outside(
            &self,
            tenant: TenantId,
            request: RequestId,
        ) -> Result<bool, StoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .request
                .is_some_and(|entry| entry.0 != tenant && entry.1 == request))
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> OffsetDateTime {
            OffsetDateTime::UNIX_EPOCH
        }
    }

    fn principal(tenant: TenantId, role: Role) -> Principal {
        Principal::new(tenant, Subject::parse("api-user").unwrap(), role)
    }

    #[tokio::test]
    async fn denied_permission_does_not_consume_a_token() {
        let store = Store::default();
        let namespace = NamespaceId::generate();
        let tenant = TenantId::generate();
        store.0.lock().unwrap().namespaces.push((tenant, namespace));
        let service = ApiAdmissionService::new(store.clone(), FixedClock);

        let result = service
            .admit(
                &principal(tenant, Role::Viewer),
                ApiOperation::DefinitionWrite,
                ApiTarget::Namespace(namespace),
                &RequestContext::default(),
            )
            .await;

        assert_eq!(
            result,
            Err(AppError::Forbidden {
                permission: Permission::DefinitionWrite
            })
        );
        assert!(store.0.lock().unwrap().charges.is_empty());
    }

    #[tokio::test]
    async fn foreign_resource_is_opaque_audited_and_not_charged() {
        let store = Store::default();
        let caller_tenant = TenantId::generate();
        let workflow = WorkflowId::generate();
        store.0.lock().unwrap().workflow =
            Some((TenantId::generate(), workflow, NamespaceId::generate()));
        let service = ApiAdmissionService::new(store.clone(), FixedClock);

        let result = service
            .admit(
                &principal(caller_tenant, Role::Viewer),
                ApiOperation::DefinitionRead,
                ApiTarget::Workflow(workflow),
                &RequestContext::default(),
            )
            .await;

        assert_eq!(result, Err(AppError::NotFound));
        let state = store.0.lock().unwrap();
        assert!(state.charges.is_empty());
        assert_eq!(state.denials, 1);
    }

    #[tokio::test]
    async fn run_is_charged_once_to_its_stored_namespace() {
        let store = Store::default();
        let tenant = TenantId::generate();
        let namespace = NamespaceId::generate();
        let run = RunId::generate();
        store.0.lock().unwrap().run = Some((tenant, run, namespace));
        let service = ApiAdmissionService::new(store.clone(), FixedClock);

        let resolved = service
            .admit(
                &principal(tenant, Role::Viewer),
                ApiOperation::RunRead,
                ApiTarget::Run(run),
                &RequestContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(resolved, namespace);
        assert_eq!(store.0.lock().unwrap().charges, vec![(tenant, namespace)]);
    }

    #[tokio::test]
    async fn promotion_charges_only_the_authorized_destination() {
        let store = Store::default();
        let tenant = TenantId::generate();
        let source_namespace = NamespaceId::generate();
        let destination = NamespaceId::generate();
        let workflow = WorkflowId::generate();
        {
            let mut state = store.0.lock().unwrap();
            state.workflow = Some((tenant, workflow, source_namespace));
            state.namespaces.push((tenant, destination));
        }
        let service = ApiAdmissionService::new(store.clone(), FixedClock);

        service
            .admit(
                &principal(tenant, Role::Admin),
                ApiOperation::Promotion,
                ApiTarget::Promotion {
                    source: workflow,
                    destination,
                },
                &RequestContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(store.0.lock().unwrap().charges, vec![(tenant, destination)]);
    }

    #[tokio::test]
    async fn refusal_and_storage_failure_are_closed_typed_errors() {
        let store = Store::default();
        let tenant = TenantId::generate();
        let namespace = NamespaceId::generate();
        store.0.lock().unwrap().decision = Ok(RateDecision::Refused {
            retry_after_secs: 7,
        });
        let service = ApiAdmissionService::new(store.clone(), FixedClock);
        assert_eq!(
            service.admit_resolved_namespace(tenant, namespace).await,
            Err(AppError::Busy {
                retry_after_secs: 7
            })
        );

        store.0.lock().unwrap().decision = Err(StoreError::Unavailable {
            detail: "database unavailable".to_owned(),
        });
        assert_eq!(
            service.admit_resolved_namespace(tenant, namespace).await,
            Err(AppError::Unavailable)
        );
        assert_eq!(store.0.lock().unwrap().charges.len(), 2);
    }
}
