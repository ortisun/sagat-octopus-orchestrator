//! Structural API checks only; no source writer, canonical row or recovery proof.
#![allow(clippy::expect_used)]

use sagat_application::leaf_certification::{
    CertifyLeafCollectionSuccess, LeafCollectionPathIdentity, LeafCollectionSuccessError as Error,
    LeafCollectionSuccessProfile, LeafCollectionSuccessRecord,
};
use sagat_application::primitives::{CollectionScopeId, SchedulerAuthority};
use sagat_domain::{
    audit::AuditId,
    definition::WorkflowVersionId,
    execution::{ExecutionInstanceId, InstancePath, RunId},
    identity::{NamespaceId, TenantId},
    workflow::{Digest, NodeId},
};
use std::collections::HashSet;
use uuid::Uuid;

fn id(value: &str) -> NodeId {
    NodeId::parse(value).expect("node")
}
fn path(owner: &str, index: u32) -> InstancePath {
    InstancePath::root().child(id(owner), index).expect("path")
}
fn identity() -> LeafCollectionPathIdentity {
    LeafCollectionPathIdentity::new(
        TenantId::new(Uuid::from_u128(1)),
        NamespaceId::new(Uuid::from_u128(2)),
        RunId::new(Uuid::from_u128(3)),
        path("items", 4),
    )
    .expect("depth-one identity")
}
fn record() -> LeafCollectionSuccessRecord {
    let identity = identity();
    LeafCollectionSuccessRecord {
        profile: LeafCollectionSuccessProfile::OrdinaryLeafV1,
        definition_version: WorkflowVersionId::new(Uuid::from_u128(5)),
        scope_id: CollectionScopeId::new(Uuid::from_u128(6)),
        body: id("line"),
        return_instance: ExecutionInstanceId::in_path(identity.path().clone(), id("returned")),
        admitted_sequence: 2,
        returned_sequence: 8,
        checkpoint_sequence: 9,
        checkpoint_result_digest: Digest::new([0x11; 32]),
        finished_sequence: 12,
        aggregate_result_digest: Digest::new([0x22; 32]),
        owner_routed_sequence: 13,
        certified_run_revision: 20,
        certified_at: time::OffsetDateTime::UNIX_EPOCH,
        audit_id: AuditId::new(Uuid::from_u128(7)),
        identity,
    }
}

#[test]
fn identity_is_exactly_depth_one_and_preserves_shared_qualifiers() {
    let exact = identity();
    assert_eq!(exact.tenant_id(), TenantId::new(Uuid::from_u128(1)));
    assert_eq!(exact.namespace_id(), NamespaceId::new(Uuid::from_u128(2)));
    assert_eq!(exact.run_id(), RunId::new(Uuid::from_u128(3)));
    assert_eq!(
        exact.path().scope_owner(),
        Some(ExecutionInstanceId::root(id("items")))
    );
    for wrong in [
        InstancePath::root(),
        path("items", 4)
            .child(id("nested"), 0)
            .expect("deeper path"),
    ] {
        assert_eq!(
            LeafCollectionPathIdentity::new(
                exact.tenant_id(),
                exact.namespace_id(),
                exact.run_id(),
                wrong
            ),
            Err(Error::UnsupportedPath)
        );
    }
}

#[test]
fn canonical_identity_never_drops_tenant_namespace_run_owner_or_index() {
    let base = identity();
    let mut identities = HashSet::from([base.clone()]);
    for (tenant, namespace, run, member) in [
        (
            TenantId::new(Uuid::from_u128(10)),
            base.namespace_id(),
            base.run_id(),
            base.path().clone(),
        ),
        (
            base.tenant_id(),
            NamespaceId::new(Uuid::from_u128(10)),
            base.run_id(),
            base.path().clone(),
        ),
        (
            base.tenant_id(),
            base.namespace_id(),
            RunId::new(Uuid::from_u128(10)),
            base.path().clone(),
        ),
        (
            base.tenant_id(),
            base.namespace_id(),
            base.run_id(),
            path("other", 4),
        ),
        (
            base.tenant_id(),
            base.namespace_id(),
            base.run_id(),
            path("items", 5),
        ),
    ] {
        assert!(identities.insert(
            LeafCollectionPathIdentity::new(tenant, namespace, run, member).expect("identity")
        ));
    }
    assert_eq!(identities.len(), 6);
    assert!(!identities.insert(base));
}

#[test]
fn creation_cas_bounds_are_checked_but_renewable_authority_does_not_change_identity() {
    let identity = identity();
    let mut command = CertifyLeafCollectionSuccess {
        scheduler: SchedulerAuthority::for_run("first", identity.run_id(), 1),
        identity: identity.clone(),
        expected_run_revision: 1,
    };
    for valid in [1, (1_u64 << 63) - 1] {
        command.expected_run_revision = valid;
        assert_eq!(command.validate(), Ok(()));
    }
    for invalid in [0, 1_u64 << 63, u64::MAX] {
        command.expected_run_revision = invalid;
        assert_eq!(command.validate(), Err(Error::InvalidRevision));
    }
    command.scheduler = SchedulerAuthority::for_run("renewed", identity.run_id(), 2);
    assert_eq!(command.identity, identity);
}

#[test]
fn v1_record_uses_existing_distinct_reduction_and_checkpoint_references() {
    let mut record = record();
    assert_ne!(
        record.checkpoint_result_digest,
        record.aggregate_result_digest
    );
    assert_eq!(record.validate_structure(), Ok(()));
    // Activity in another scope can advance this revision after the closed path's routing.
    record.certified_run_revision = (1_u64 << 63) - 1;
    assert_eq!(record.validate_structure(), Ok(()));
}

#[test]
fn malformed_source_sequence_snapshots_fail_without_arithmetic_overflow() {
    for (admitted, returned, checkpoint, finished, routed, revision) in [
        (0, 8, 9, 12, 13, 20),
        (8, 8, 9, 12, 13, 20),
        (2, 8, 10, 12, 13, 20),
        (2, 8, 9, 9, 10, 20),
        (2, 8, 9, 12, 14, 20),
        (2, 8, 9, 12, 13, 12),
        (2, u64::MAX, 0, 12, 13, 20),
        (2, 8, 9, u64::MAX, 0, u64::MAX),
        (2, 8, 9, 12, 13, 1_u64 << 63),
    ] {
        let mut record = record();
        record.admitted_sequence = admitted;
        record.returned_sequence = returned;
        record.checkpoint_sequence = checkpoint;
        record.finished_sequence = finished;
        record.owner_routed_sequence = routed;
        record.certified_run_revision = revision;
        assert_eq!(record.validate_structure(), Err(Error::ContradictorySource));
    }
}

#[test]
fn a_return_from_another_path_cannot_satisfy_structural_record_binding() {
    let mut record = record();
    for wrong in [InstancePath::root(), path("items", 5), path("other", 4)] {
        record.return_instance = ExecutionInstanceId::in_path(wrong, id("returned"));
        assert_eq!(record.validate_structure(), Err(Error::ContradictorySource));
    }
}

#[test]
fn error_formatting_keeps_storage_diagnostics_and_path_text_private() {
    let canary = "sk-live1234567890abcdef";
    let error = Error::Storage(sagat_application::error::StoreError::Internal {
        detail: format!("driver text with {canary}"),
    });
    assert!(!error.to_string().contains(canary));
    assert!(!format!("{error:?}").contains(canary));
    let identity = LeafCollectionPathIdentity::new(
        identity().tenant_id(),
        identity().namespace_id(),
        identity().run_id(),
        path(canary, 0),
    )
    .expect("synthetic secret-shaped node identity");
    let error = Error::CommitOutcomeUnknown { identity };
    assert!(!error.to_string().contains(canary));
    assert!(!format!("{error:?}").contains(canary));
}
