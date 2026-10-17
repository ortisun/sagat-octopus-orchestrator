//! Typed logical identities for durable executions of a definition node.
//!
//! An attempt or lease generation is deliberately absent: retrying the same work must retain
//! its logical identity and the idempotency key derived from it. Old journal events have the
//! empty path and continue to address the original root instance.

use crate::workflow::NodeId;

/// Maximum number of nested iteration scopes in one logical instance.
pub const MAX_SCOPE_DEPTH: usize = 8;
/// Iteration indices are zero based, with the final legal index at 9,999.
pub const MAX_ITERATION_INDEX: u32 = 9_999;

/// A malformed path cannot enter the reducer or a storage identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InstanceIdentityError {
    #[error("an iteration index must be in 0..=9999")]
    IterationIndexOutOfRange,
    #[error("an instance path may contain at most eight scopes")]
    ScopeDepthExceeded,
}

/// One declared scope and its immutable, zero-based member index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeSegment {
    scope_node: NodeId,
    iteration_index: u32,
}

impl ScopeSegment {
    /// # Errors
    /// Returns [`InstanceIdentityError::IterationIndexOutOfRange`] for an index outside the
    /// declared collection/loop ceiling.
    pub fn new(scope_node: NodeId, iteration_index: u32) -> Result<Self, InstanceIdentityError> {
        if iteration_index > MAX_ITERATION_INDEX {
            return Err(InstanceIdentityError::IterationIndexOutOfRange);
        }
        Ok(Self {
            scope_node,
            iteration_index,
        })
    }

    #[must_use]
    pub fn scope_node(&self) -> &NodeId {
        &self.scope_node
    }

    #[must_use]
    pub const fn iteration_index(&self) -> u32 {
        self.iteration_index
    }
}

/// Ordered, bounded scope ancestry; empty for existing root steps.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstancePath(Vec<ScopeSegment>);

impl InstancePath {
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    /// Appends an iteration only after validating both the member and nesting ceilings.
    ///
    /// # Errors
    /// Returns [`InstanceIdentityError`] if either ceiling would be exceeded.
    pub fn child(
        &self,
        scope_node: NodeId,
        iteration_index: u32,
    ) -> Result<Self, InstanceIdentityError> {
        if self.0.len() >= MAX_SCOPE_DEPTH {
            return Err(InstanceIdentityError::ScopeDepthExceeded);
        }
        let mut segments = self.0.clone();
        segments.push(ScopeSegment::new(scope_node, iteration_index)?);
        Ok(Self(segments))
    }

    #[must_use]
    pub fn segments(&self) -> &[ScopeSegment] {
        &self.0
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The collection or loop instance whose iteration this path is: the owner of its last
    /// segment, living in the path before it. `None` for the root path.
    #[must_use]
    pub fn scope_owner(&self) -> Option<ExecutionInstanceId> {
        let (last, parents) = self.0.split_last()?;
        Some(ExecutionInstanceId::in_path(
            Self(parents.to_vec()),
            last.scope_node().clone(),
        ))
    }
}

/// One execution of a definition node, distinct from its attempt and fencing generation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutionInstanceId {
    path: InstancePath,
    node: NodeId,
}

impl ExecutionInstanceId {
    #[must_use]
    pub fn root(node: NodeId) -> Self {
        Self::in_path(InstancePath::root(), node)
    }

    #[must_use]
    pub const fn in_path(path: InstancePath, node: NodeId) -> Self {
        Self { path, node }
    }

    #[must_use]
    pub const fn path(&self) -> &InstancePath {
        &self.path
    }

    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Derived work retains its forward instance's scope path.
    #[must_use]
    pub fn compensation(&self) -> Self {
        Self::in_path(self.path.clone(), self.node.compensation())
    }

    /// Derived work retains its forward instance's scope path.
    #[must_use]
    pub fn reconciliation(&self) -> Self {
        Self::in_path(self.path.clone(), self.node.reconciliation())
    }
}
