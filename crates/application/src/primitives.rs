//! Transaction boundary for durable collection progress.
//!
//! This port deliberately exposes no end-user operation. It gives the execution engine one place
//! to commit a collection journal event together with its projection, budget accounting, optional
//! audit evidence and task outbox hint. Public workflow admission remains gated elsewhere.

use std::future::Future;

use sagat_domain::audit::AuditRecord;
use sagat_domain::execution::{CollectionDrainReason, ExecutionInstanceId, RunId};
use sagat_domain::identity::{NamespaceId, Subject, TenantId};
use sagat_domain::sharding::Shard;
use sagat_domain::workflow::{ActivityName, Digest};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;
use crate::shards::Lease;

/// Database-backed scheduler lease authorizing a transition for one run shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerAuthority {
    owner: String,
    lease: Lease,
}

impl SchedulerAuthority {
    #[must_use]
    pub fn new(owner: impl Into<String>, lease: Lease) -> Self {
        Self {
            owner: owner.into(),
            lease,
        }
    }

    #[must_use]
    pub fn for_run(owner: impl Into<String>, run_id: RunId, generation: u64) -> Self {
        Self::new(
            owner,
            Lease {
                shard: Shard::of(run_id.as_uuid()),
                generation,
            },
        )
    }

    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    #[must_use]
    pub const fn lease(&self) -> Lease {
        self.lease
    }
}

/// Stable identity of one collection activation within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CollectionScopeId(Uuid);

impl CollectionScopeId {
    #[must_use]
    pub const fn new(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Immutable captured values and committed results needed to resume collection scheduling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionProgress {
    pub scope_id: CollectionScopeId,
    pub snapshot: Vec<u8>,
    pub items: Vec<Vec<u8>>,
    pub results: Vec<(u32, Vec<u8>)>,
}

/// Effective definition and tenant limits pinned when a collection enters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionLimits {
    max_items: u32,
    max_concurrency: u16,
    max_instances: u32,
    max_depth: u8,
}

impl CollectionLimits {
    /// # Errors
    /// Returns [`PrimitiveInputError::Limits`] when a value is outside the accepted ceilings.
    pub const fn new(
        max_items: u32,
        max_concurrency: u16,
        max_instances: u32,
        max_depth: u8,
    ) -> Result<Self, PrimitiveInputError> {
        if max_items == 0
            || max_items > 10_000
            || max_concurrency == 0
            || max_concurrency > 256
            || max_instances == 0
            || max_instances > 100_000
            || max_depth == 0
            || max_depth > 8
        {
            return Err(PrimitiveInputError::Limits);
        }
        Ok(Self {
            max_items,
            max_concurrency,
            max_instances,
            max_depth,
        })
    }

    #[must_use]
    pub const fn max_items(self) -> u32 {
        self.max_items
    }

    #[must_use]
    pub const fn max_concurrency(self) -> u16 {
        self.max_concurrency
    }

    #[must_use]
    pub const fn max_instances(self) -> u32 {
        self.max_instances
    }

    #[must_use]
    pub const fn max_depth(self) -> u8 {
        self.max_depth
    }
}

/// Run, scheduler authority and journal metadata shared by one transactional collection change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionTransition {
    tenant_id: TenantId,
    namespace_id: NamespaceId,
    run_id: RunId,
    expected_run_version: u64,
    owner: ExecutionInstanceId,
    scheduler: SchedulerAuthority,
    actor: Subject,
    trace_id: Option<String>,
    occurred_at: OffsetDateTime,
    audit: Option<AuditRecord>,
}

impl CollectionTransition {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        tenant_id: TenantId,
        namespace_id: NamespaceId,
        run_id: RunId,
        expected_run_version: u64,
        owner: ExecutionInstanceId,
        scheduler: SchedulerAuthority,
        actor: Subject,
        trace_id: Option<String>,
        occurred_at: OffsetDateTime,
        audit: Option<AuditRecord>,
    ) -> Self {
        Self {
            tenant_id,
            namespace_id,
            run_id,
            expected_run_version,
            owner,
            scheduler,
            actor,
            trace_id,
            occurred_at,
            audit,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn namespace_id(&self) -> NamespaceId {
        self.namespace_id
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn expected_run_version(&self) -> u64 {
        self.expected_run_version
    }

    #[must_use]
    pub const fn owner(&self) -> &ExecutionInstanceId {
        &self.owner
    }

    #[must_use]
    pub const fn scheduler(&self) -> &SchedulerAuthority {
        &self.scheduler
    }

    #[must_use]
    pub const fn actor(&self) -> &Subject {
        &self.actor
    }

    #[must_use]
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    #[must_use]
    pub const fn occurred_at(&self) -> OffsetDateTime {
        self.occurred_at
    }

    #[must_use]
    pub const fn audit(&self) -> Option<&AuditRecord> {
        self.audit.as_ref()
    }

    #[must_use]
    pub fn with_audit(mut self, audit: AuditRecord) -> Self {
        self.audit = Some(audit);
        self
    }
}

/// Immutable membership captured before any collection member is dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionEntry {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub item_count: u32,
    pub snapshot_digest: Digest,
    pub limits: CollectionLimits,
    snapshot_content: Option<Vec<u8>>,
}

impl CollectionEntry {
    /// # Errors
    /// Returns [`PrimitiveInputError::ItemCount`] if the snapshot exceeds the pinned item limit.
    pub fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        item_count: u32,
        snapshot_digest: Digest,
        limits: CollectionLimits,
    ) -> Result<Self, PrimitiveInputError> {
        if item_count > limits.max_items() {
            return Err(PrimitiveInputError::ItemCount);
        }
        Ok(Self {
            transition,
            scope_id,
            item_count,
            snapshot_digest,
            limits,
            snapshot_content: None,
        })
    }

    /// Constructs a runtime entry whose snapshot must be retained with the transition.
    ///
    /// # Errors
    /// Returns [`PrimitiveInputError::ItemCount`] if the snapshot exceeds the pinned item limit.
    pub fn from_snapshot(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        item_count: u32,
        snapshot_content: Vec<u8>,
        limits: CollectionLimits,
    ) -> Result<Self, PrimitiveInputError> {
        let snapshot_digest = digest(&snapshot_content);
        let mut entry = Self::new(transition, scope_id, item_count, snapshot_digest, limits)?;
        entry.snapshot_content = Some(snapshot_content);
        Ok(entry)
    }

    #[must_use]
    pub fn snapshot_content(&self) -> Option<&[u8]> {
        self.snapshot_content.as_deref()
    }
}

/// One bounded activity-body admission and its durable task notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionAdmission {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub index: u32,
    pub entry: ExecutionInstanceId,
    pub activity: ActivityName,
    pub item_digest: Digest,
    pub reserved_instances: u16,
    pub outbox_id: Uuid,
    item_content: Option<Vec<u8>>,
}

impl CollectionAdmission {
    /// Constructs a shallow activity admission. One reservation is mandatory for the body's return
    /// instance; additional reservations remain available for declared derived obligations.
    ///
    /// # Errors
    /// Returns [`PrimitiveInputError`] if the index/path or reservation is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        index: u32,
        entry: ExecutionInstanceId,
        activity: ActivityName,
        item_digest: Digest,
        reserved_instances: u16,
        outbox_id: Uuid,
    ) -> Result<Self, PrimitiveInputError> {
        let expected_path = transition
            .owner()
            .path()
            .child(transition.owner().node().clone(), index)
            .map_err(|_| PrimitiveInputError::InstancePath)?;
        if entry.path() != &expected_path {
            return Err(PrimitiveInputError::InstancePath);
        }
        if reserved_instances == 0 {
            return Err(PrimitiveInputError::MissingReturnReservation);
        }
        Ok(Self {
            transition,
            scope_id,
            index,
            entry,
            activity,
            item_digest,
            reserved_instances,
            outbox_id,
            item_content: None,
        })
    }

    /// Constructs a runtime admission whose item must be retained with the transition.
    ///
    /// # Errors
    /// Returns [`PrimitiveInputError`] if the index/path or reservation is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn from_item(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        index: u32,
        entry: ExecutionInstanceId,
        activity: ActivityName,
        item_content: Vec<u8>,
        reserved_instances: u16,
        outbox_id: Uuid,
    ) -> Result<Self, PrimitiveInputError> {
        let item_digest = digest(&item_content);
        let mut admission = Self::new(
            transition,
            scope_id,
            index,
            entry,
            activity,
            item_digest,
            reserved_instances,
            outbox_id,
        )?;
        admission.item_content = Some(item_content);
        Ok(admission)
    }

    #[must_use]
    pub fn item_content(&self) -> Option<&[u8]> {
        self.item_content.as_deref()
    }
}

/// One returned body result made durable in input-index order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionCheckpoint {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub index: u32,
    pub return_instance: ExecutionInstanceId,
    pub result_digest: Digest,
    pub outbox_id: Uuid,
    result_content: Option<Vec<u8>>,
}

impl CollectionCheckpoint {
    /// # Errors
    /// Returns [`PrimitiveInputError::InstancePath`] if the return is outside this iteration.
    pub fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        index: u32,
        return_instance: ExecutionInstanceId,
        result_digest: Digest,
        outbox_id: Uuid,
    ) -> Result<Self, PrimitiveInputError> {
        let expected_path = transition
            .owner()
            .path()
            .child(transition.owner().node().clone(), index)
            .map_err(|_| PrimitiveInputError::InstancePath)?;
        if return_instance.path() != &expected_path {
            return Err(PrimitiveInputError::InstancePath);
        }
        Ok(Self {
            transition,
            scope_id,
            index,
            return_instance,
            result_digest,
            outbox_id,
            result_content: None,
        })
    }

    /// Constructs a runtime checkpoint whose result must be retained with the transition.
    ///
    /// # Errors
    /// Returns [`PrimitiveInputError::InstancePath`] if the return is outside this iteration.
    pub fn from_result(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        index: u32,
        return_instance: ExecutionInstanceId,
        result_content: Vec<u8>,
        outbox_id: Uuid,
    ) -> Result<Self, PrimitiveInputError> {
        let result_digest = digest(&result_content);
        let mut checkpoint = Self::new(
            transition,
            scope_id,
            index,
            return_instance,
            result_digest,
            outbox_id,
        )?;
        checkpoint.result_content = Some(result_content);
        Ok(checkpoint)
    }

    #[must_use]
    pub fn result_content(&self) -> Option<&[u8]> {
        self.result_content.as_deref()
    }
}

/// Ordered aggregate written after every member is checkpointed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionFinish {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub result_digest: Digest,
    pub outbox_id: Uuid,
    result_content: Option<Vec<u8>>,
    source_digests: Option<Vec<(u32, Digest)>>,
}

/// Starts fail-fast or cancellation drain under the collection owner's scheduler fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionDrain {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub reason: CollectionDrainReason,
    pub outbox_id: Uuid,
}

impl CollectionDrain {
    #[must_use]
    pub const fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        reason: CollectionDrainReason,
        outbox_id: Uuid,
    ) -> Self {
        Self {
            transition,
            scope_id,
            reason,
            outbox_id,
        }
    }
}

/// Marks one admitted member drained only after all of its effect obligations are settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionIterationDrain {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub index: u32,
    pub outbox_id: Uuid,
}

impl CollectionIterationDrain {
    #[must_use]
    pub const fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        index: u32,
        outbox_id: Uuid,
    ) -> Self {
        Self {
            transition,
            scope_id,
            index,
            outbox_id,
        }
    }
}

/// Closes a drained collection with the outcome pinned when drain began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionDrainFinish {
    pub transition: CollectionTransition,
    pub scope_id: CollectionScopeId,
    pub outbox_id: Uuid,
}

impl CollectionDrainFinish {
    #[must_use]
    pub const fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        outbox_id: Uuid,
    ) -> Self {
        Self {
            transition,
            scope_id,
            outbox_id,
        }
    }
}

impl CollectionFinish {
    #[must_use]
    pub const fn new(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        result_digest: Digest,
        outbox_id: Uuid,
    ) -> Self {
        Self {
            transition,
            scope_id,
            result_digest,
            outbox_id,
            result_content: None,
            source_digests: None,
        }
    }

    #[must_use]
    pub fn from_result(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        result_content: Vec<u8>,
        outbox_id: Uuid,
    ) -> Self {
        let result_digest = digest(&result_content);
        let mut finish = Self::new(transition, scope_id, result_digest, outbox_id);
        finish.result_content = Some(result_content);
        finish
    }

    #[must_use]
    pub fn from_evaluated_result(
        transition: CollectionTransition,
        scope_id: CollectionScopeId,
        result_content: Vec<u8>,
        source_results: &[(u32, Vec<u8>)],
        outbox_id: Uuid,
    ) -> Self {
        let mut finish = Self::from_result(transition, scope_id, result_content, outbox_id);
        finish.source_digests = Some(
            source_results
                .iter()
                .map(|(index, content)| (*index, digest(content)))
                .collect(),
        );
        finish
    }

    #[must_use]
    pub fn result_content(&self) -> Option<&[u8]> {
        self.result_content.as_deref()
    }

    #[must_use]
    pub fn source_digests(&self) -> Option<&[(u32, Digest)]> {
        self.source_digests.as_deref()
    }
}

fn digest(content: &[u8]) -> Digest {
    Digest::new(Sha256::digest(content).into())
}

/// Whether a write changed durable state or replayed the already committed transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimitiveWrite {
    pub applied: bool,
    pub run_version: u64,
}

/// Invalid data rejected before storage is touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PrimitiveInputError {
    #[error("collection limits are outside the accepted ceilings")]
    Limits,
    #[error("the captured collection exceeds its item limit")]
    ItemCount,
    #[error("the instance does not belong to the named collection member")]
    InstancePath,
    #[error("an admitted body must reserve its return instance")]
    MissingReturnReservation,
}

/// A durable transition refusal. Details contain no workflow values or secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrimitiveStoreError {
    #[error("run or scope not found")]
    Missing,
    #[error("run version changed (current: {current})")]
    RevisionConflict { current: u64 },
    #[error("the collection transition is not legal in its current state")]
    InvalidTransition,
    #[error("the same collection identity was reused for different data")]
    DuplicateMismatch,
    #[error("tenant iteration capacity is currently full")]
    AtCapacity,
    #[error("the run instance budget is exhausted")]
    InstanceBudgetExceeded,
    #[error("audit evidence belongs to another tenant")]
    AuditTenantMismatch,
    #[error("tenant collection policy is not configured")]
    PolicyMissing,
    #[error("scheduler ownership changed before the transition committed")]
    SchedulerFenceLost,
    #[error("scoped body routing is not integrated with durable replay")]
    RuntimeNotIntegrated,
    #[error(transparent)]
    Storage(#[from] StoreError),
}

/// Storage contract for collection progress. Every applied method is one database transaction and
/// requires the current scheduler fence. A matching replay returns `applied = false` without
/// reacquiring capacity, charging, releasing or notifying again.
pub trait CollectionStore: Send + Sync {
    fn retain_collection_payload(
        &self,
        tenant_id: TenantId,
        content: &[u8],
    ) -> impl Future<Output = Result<Digest, PrimitiveStoreError>> + Send;

    fn collection_progress(
        &self,
        tenant_id: TenantId,
        run_id: RunId,
        owner: &ExecutionInstanceId,
    ) -> impl Future<Output = Result<Option<CollectionProgress>, PrimitiveStoreError>> + Send;

    fn enter_collection(
        &self,
        entry: &CollectionEntry,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;

    fn admit_collection_activity(
        &self,
        admission: &CollectionAdmission,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;

    fn checkpoint_collection_iteration(
        &self,
        checkpoint: &CollectionCheckpoint,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;

    fn start_collection_drain(
        &self,
        drain: &CollectionDrain,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;

    fn drain_collection_iteration(
        &self,
        drain: &CollectionIterationDrain,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;

    fn finish_collection_drain(
        &self,
        finish: &CollectionDrainFinish,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;

    fn finish_collection(
        &self,
        finish: &CollectionFinish,
    ) -> impl Future<Output = Result<PrimitiveWrite, PrimitiveStoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::execution::InstancePath;
    use sagat_domain::workflow::NodeId;

    #[test]
    fn admission_requires_the_exact_member_path_and_a_return_reservation() {
        let owner_node = NodeId::parse("items").expect("owner node");
        let owner = ExecutionInstanceId::root(owner_node.clone());
        let run_id = RunId::generate();
        let transition = CollectionTransition::new(
            TenantId::generate(),
            NamespaceId::generate(),
            run_id,
            4,
            owner,
            SchedulerAuthority::for_run("scheduler-a", run_id, 1),
            Subject::engine(),
            None,
            OffsetDateTime::UNIX_EPOCH,
            None,
        );
        let wrong = ExecutionInstanceId::in_path(
            InstancePath::root()
                .child(owner_node, 1)
                .expect("member path"),
            NodeId::parse("work").expect("entry node"),
        );
        let result = CollectionAdmission::new(
            transition,
            CollectionScopeId::generate(),
            0,
            wrong,
            ActivityName::parse("line.process").expect("activity"),
            Digest::new([1; 32]),
            1,
            Uuid::now_v7(),
        );
        assert_eq!(result, Err(PrimitiveInputError::InstancePath));
    }

    #[test]
    fn every_effective_limit_is_bounded() {
        assert!(CollectionLimits::new(10_000, 256, 100_000, 8).is_ok());
        for limits in [
            CollectionLimits::new(0, 1, 1, 1),
            CollectionLimits::new(1, 257, 1, 1),
            CollectionLimits::new(1, 1, 100_001, 1),
            CollectionLimits::new(1, 1, 1, 9),
        ] {
            assert_eq!(limits, Err(PrimitiveInputError::Limits));
        }
    }

    #[test]
    fn empty_collection_is_valid_when_atomic_finish_is_available() {
        let owner = ExecutionInstanceId::root(NodeId::parse("items").expect("owner node"));
        let run_id = RunId::generate();
        let transition = CollectionTransition::new(
            TenantId::generate(),
            NamespaceId::generate(),
            run_id,
            0,
            owner,
            SchedulerAuthority::for_run("scheduler-a", run_id, 1),
            Subject::engine(),
            None,
            OffsetDateTime::UNIX_EPOCH,
            None,
        );
        let result = CollectionEntry::new(
            transition,
            CollectionScopeId::generate(),
            0,
            Digest::new([1; 32]),
            CollectionLimits::new(1, 1, 1, 1).expect("limits"),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn runtime_commands_pin_the_digest_to_their_inline_content() {
        let owner_node = NodeId::parse("items").expect("owner node");
        let owner = ExecutionInstanceId::root(owner_node.clone());
        let run_id = RunId::generate();
        let transition = CollectionTransition::new(
            TenantId::generate(),
            NamespaceId::generate(),
            run_id,
            4,
            owner,
            SchedulerAuthority::for_run("scheduler-a", run_id, 1),
            Subject::engine(),
            None,
            OffsetDateTime::UNIX_EPOCH,
            None,
        );
        let scope = CollectionScopeId::generate();
        let snapshot = br#"[{"value":1}]"#.to_vec();
        let entry = CollectionEntry::from_snapshot(
            transition.clone(),
            scope,
            1,
            snapshot.clone(),
            CollectionLimits::new(1, 1, 4, 2).expect("limits"),
        )
        .expect("entry");
        assert_eq!(entry.snapshot_digest, digest(&snapshot));
        assert_eq!(entry.snapshot_content(), Some(snapshot.as_slice()));

        let path = InstancePath::root()
            .child(owner_node, 0)
            .expect("member path");
        let activity = ExecutionInstanceId::in_path(
            path.clone(),
            NodeId::parse("work").expect("activity node"),
        );
        let item = br#"{"value":1}"#.to_vec();
        let admission = CollectionAdmission::from_item(
            transition.clone(),
            scope,
            0,
            activity,
            ActivityName::parse("line.process").expect("activity"),
            item.clone(),
            1,
            Uuid::now_v7(),
        )
        .expect("admission");
        assert_eq!(admission.item_digest, digest(&item));
        assert_eq!(admission.item_content(), Some(item.as_slice()));

        let returned =
            ExecutionInstanceId::in_path(path, NodeId::parse("return").expect("return node"));
        let result = br#"{"ok":true}"#.to_vec();
        let checkpoint = CollectionCheckpoint::from_result(
            transition.clone(),
            scope,
            0,
            returned,
            result.clone(),
            Uuid::now_v7(),
        )
        .expect("checkpoint");
        assert_eq!(checkpoint.result_digest, digest(&result));
        assert_eq!(checkpoint.result_content(), Some(result.as_slice()));

        let aggregate = br#"[{"ok":true}]"#.to_vec();
        let finish =
            CollectionFinish::from_result(transition, scope, aggregate.clone(), Uuid::now_v7());
        assert_eq!(finish.result_digest, digest(&aggregate));
        assert_eq!(finish.result_content(), Some(aggregate.as_slice()));
    }
}
