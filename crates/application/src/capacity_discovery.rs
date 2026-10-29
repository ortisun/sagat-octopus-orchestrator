//! Read-only registry hints. Every future mutation must revalidate its own authority.
use crate::{error::StoreError, maintenance::Window};
use sagat_domain::{
    execution::{CapacityGroupId, CapacityMemberId, RunId},
    identity::{NamespaceId, TenantId},
};
use std::future::Future;

pub const CAPACITY_DISCOVERY_PAGE_MAX: u32 = 256;

/// Exclusive keyset checkpoint through consumed memberships, including filtered rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityDiscoveryCursor {
    pub tenant_id: TenantId,
    pub group_id: CapacityGroupId,
    pub member_id: CapacityMemberId,
}

/// Observed binding and revisions only; this confers no execution or mutation authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityCandidate {
    pub cursor: CapacityDiscoveryCursor,
    pub namespace_id: NamespaceId,
    pub chain_id: RunId,
    pub head_run_id: RunId,
    pub group_revision: u64,
    pub group_epoch: u64,
    pub chain_revision: u64,
    pub run_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityDiscoveryPage {
    pub candidates: Vec<CapacityCandidate>,
    pub scanned_through: Option<CapacityDiscoveryCursor>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapacityDiscoveryError {
    #[error("capacity discovery requires a page size in 1..=256 and at most 256 exclusions")]
    InvalidWindow,
    #[error("capacity discovery metadata is contradictory")]
    ContradictoryProjection,
    #[error("capacity discovery storage failed: {0}")]
    Storage(#[from] StoreError),
}

pub trait CapacityDiscoveryStore: Send + Sync {
    fn discover_vacant_child_roots(
        &self,
        window: &Window,
        after: Option<CapacityDiscoveryCursor>,
    ) -> impl Future<Output = Result<CapacityDiscoveryPage, CapacityDiscoveryError>> + Send;
}
