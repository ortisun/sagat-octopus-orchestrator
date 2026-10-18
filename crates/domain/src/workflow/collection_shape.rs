//! Frozen static shape for the first successful leaf-path closure proof. This does not prove
//! publication support, version pinning, admission, source finality or execution authority.

use std::collections::BTreeSet;

use super::{BodyDefinition, Node, NodeId, NodeKind, WorkflowDefinition};
use crate::{
    execution::ExecutionInstanceId,
    saga::{EffectPolicy, NodeOrigin},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CollectionSuccessShapeError {
    #[error("the selected owner is not a root collection")]
    UnsupportedOwner,
    #[error("the root definition is outside the leaf collection shape")]
    UnsupportedDefinition,
    #[error("the definition contains an unsupported collection body")]
    UnsupportedBody,
}

/// Ordered, qualified definition facts for source-certification shape contract version 1.
///
/// This object deliberately contains no workflow version, run or settled flag. A storage writer
/// must obtain the matching pinned definition and prove exact committed own-path source facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafCollectionSuccessShape {
    owner: ExecutionInstanceId,
    body: NodeId,
    activities: Vec<NodeId>,
    return_node: NodeId,
}

impl LeafCollectionSuccessShape {
    /// Check the frozen root collection chain and selected ordinary leaf body. Other referenced
    /// bodies must retain the public linear shape, but may have their own declared companions.
    ///
    /// # Errors
    /// Reject a scoped/noncollection owner, broader root graph or nonordinary selected body.
    pub fn check(
        definition: &WorkflowDefinition,
        owner: &ExecutionInstanceId,
    ) -> Result<Self, CollectionSuccessShapeError> {
        use CollectionSuccessShapeError::{UnsupportedDefinition, UnsupportedOwner};
        if !owner.path().is_root() {
            return Err(UnsupportedOwner);
        }
        let Some(NodeKind::ForEach { body: selected, .. }) =
            definition.node(owner.node()).map(Node::kind)
        else {
            return Err(UnsupportedOwner);
        };
        if !definition.is_structured()
            || definition.continuation_policy().is_some()
            || !definition.triggers().is_empty()
        {
            return Err(UnsupportedDefinition);
        }
        let declared = definition.nodes().count();
        let mut visited = 0;
        let mut collections_started = false;
        let mut references = BTreeSet::new();
        let mut current = definition.entrypoint();
        loop {
            visited += 1;
            if visited > declared {
                return Err(UnsupportedDefinition);
            }
            let node = definition.node(current).ok_or(UnsupportedDefinition)?;
            if node.on_error().is_some() || node.effect() != &EffectPolicy::NONE {
                return Err(UnsupportedDefinition);
            }
            match node.kind() {
                NodeKind::Activity { .. } if !collections_started && ordinary(node) => {}
                NodeKind::ForEach { body, .. } => {
                    collections_started = true;
                    references.insert(body);
                }
                NodeKind::End { .. } if collections_started && node.next().is_none() => break,
                _ => return Err(UnsupportedDefinition),
            }
            current = node.next().ok_or(UnsupportedDefinition)?;
        }
        if visited != declared || definition.all_nodes().count() != declared {
            return Err(UnsupportedDefinition);
        }
        for body in definition.bodies() {
            if !references.contains(body.name()) {
                return Err(UnsupportedDefinition);
            }
            // Freeze the surrounding public shape too; a later publisher widening does not
            // silently widen the assumptions of this version of source certification.
            chain(body, false)?;
        }
        let body = definition.body(selected).ok_or(UnsupportedDefinition)?;
        let (activities, return_node) = chain(body, true)?;
        Ok(Self {
            owner: owner.clone(),
            body: selected.clone(),
            activities,
            return_node,
        })
    }

    #[must_use]
    pub const fn contract_version(&self) -> u16 {
        1
    }
    #[must_use]
    pub const fn owner(&self) -> &ExecutionInstanceId {
        &self.owner
    }
    #[must_use]
    pub const fn body(&self) -> &NodeId {
        &self.body
    }
    #[must_use]
    pub fn activities(&self) -> &[NodeId] {
        &self.activities
    }
    #[must_use]
    pub const fn return_node(&self) -> &NodeId {
        &self.return_node
    }
}

fn ordinary(node: &Node) -> bool {
    matches!(node.kind(), NodeKind::Activity { activity, compensate_with: None, reconcile_with: None, .. }
        if crate::scripts::digest_of_activity(activity.as_str()).is_none())
        && node.origin() == &NodeOrigin::Declared
        && node.database_operation().is_none()
        && node.effect() == &EffectPolicy::NONE
}

fn chain(
    body: &BodyDefinition,
    require_ordinary: bool,
) -> Result<(Vec<NodeId>, NodeId), CollectionSuccessShapeError> {
    use CollectionSuccessShapeError::UnsupportedBody;
    let mut current = body.entrypoint();
    let mut activities = Vec::new();
    loop {
        let node = body.node(current).ok_or(UnsupportedBody)?;
        if node.origin() != &NodeOrigin::Declared || node.on_error().is_some() {
            return Err(UnsupportedBody);
        }
        match node.kind() {
            NodeKind::Activity { .. }
                if activities.len() < 8 && (!require_ordinary || ordinary(node)) =>
            {
                activities.push(node.id().clone());
                current = node.next().ok_or(UnsupportedBody)?;
            }
            NodeKind::Return { .. }
                if !activities.is_empty()
                    && node.next().is_none()
                    && activities.len() + 1 == body.nodes().count() =>
            {
                return Ok((activities, node.id().clone()));
            }
            _ => return Err(UnsupportedBody),
        }
    }
}
