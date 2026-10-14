//! One durable namespace-rate admission point shared by public HTTP handlers.

use sagat_application::api_admission::{
    ApiAdmissionService, ApiAdmissionStore, ApiOperation, ApiTarget,
};
use sagat_application::clock::Clock;
use sagat_domain::identity::{NamespaceId, Slug};

use crate::core::Core;
use crate::error::ApiError;
use crate::extract::Caller;

pub trait Admission: Core {
    type AdmissionStore: ApiAdmissionStore + 'static;
    type AdmissionClock: Clock + 'static;

    fn api_admission(&self) -> &ApiAdmissionService<Self::AdmissionStore, Self::AdmissionClock>;
}

pub(crate) async fn admit<B: Admission>(
    state: &B,
    caller: &Caller,
    operation: ApiOperation,
    target: ApiTarget,
) -> Result<NamespaceId, ApiError> {
    state
        .api_admission()
        .admit(&caller.session.principal, operation, target, &caller.ctx)
        .await
        .map_err(ApiError::from)
}

pub(crate) async fn admit_resolved_namespace<B: Admission>(
    state: &B,
    caller: &Caller,
    namespace: NamespaceId,
) -> Result<(), ApiError> {
    state
        .api_admission()
        .admit_resolved_namespace(caller.session.principal.tenant_id(), namespace)
        .await
        .map_err(ApiError::from)
}

pub(crate) fn namespace_slug(raw: &str) -> Result<Slug, ApiError> {
    Slug::parse(raw).map_err(|_| ApiError::not_found())
}
