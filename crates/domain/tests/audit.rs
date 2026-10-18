//! Audit record invariants (FR-001-007).
#![allow(clippy::expect_used)]

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType, RunDataLimitAuditReason};
use sagat_domain::identity::{Principal, Role, Subject, TenantId};
use time::OffsetDateTime;

fn principal() -> Principal {
    Principal::new(
        TenantId::generate(),
        Subject::parse("alice").expect("subject"),
        Role::Admin,
    )
}

#[test]
fn tenant_and_actor_come_from_the_principal() {
    let p = principal();
    let record = AuditRecord::by(
        &p,
        AuditAction::WorkspaceCreated,
        ResourceType::Workspace,
        "ws-1",
        Some("4bf92f3577b34da6a3ce929d0e0e4736".into()),
        OffsetDateTime::UNIX_EPOCH,
    );

    assert_eq!(record.tenant_id(), p.tenant_id());
    assert_eq!(record.actor(), p.subject());
    assert_eq!(record.action(), AuditAction::WorkspaceCreated);
    assert_eq!(record.resource_type(), ResourceType::Workspace);
    assert_eq!(record.resource_id(), "ws-1");
    assert_eq!(record.trace_id(), Some("4bf92f3577b34da6a3ce929d0e0e4736"));
    assert_eq!(record.reason(), None);
    assert_eq!(record.created_at(), OffsetDateTime::UNIX_EPOCH);
}

#[test]
fn reason_is_optional_and_attached_on_request() {
    let record = AuditRecord::by(
        &principal(),
        AuditAction::CrossTenantAccessDenied,
        ResourceType::Namespace,
        "some-id",
        None,
        OffsetDateTime::UNIX_EPOCH,
    )
    .with_reason("resource not accessible from this tenant");

    assert_eq!(
        record.reason(),
        Some("resource not accessible from this tenant")
    );
}

#[test]
fn each_record_gets_a_distinct_id() {
    let p = principal();
    let make = || {
        AuditRecord::by(
            &p,
            AuditAction::NamespaceCreated,
            ResourceType::Namespace,
            "ns",
            None,
            OffsetDateTime::UNIX_EPOCH,
        )
    };
    assert_ne!(make().id(), make().id());
}

/// These strings are stored in `audit_records.action` / `resource_type` and read by operators:
/// changing one is a breaking change of the audit contract.
#[test]
fn stored_names_are_a_stable_contract() {
    assert_eq!(
        AuditAction::TenantProvisioned.as_str(),
        "tenant.provisioned"
    );
    assert_eq!(AuditAction::WorkspaceCreated.as_str(), "workspace.created");
    assert_eq!(AuditAction::NamespaceCreated.as_str(), "namespace.created");
    assert_eq!(AuditAction::RunQueried.as_str(), "run.queried");
    assert_eq!(AuditAction::RunUpdated.as_str(), "run.updated");
    assert_eq!(
        AuditAction::RunDataLimitExceeded.as_str(),
        "run.data_limit_exceeded"
    );
    assert_eq!(
        AuditAction::RunStorageQuotaExceeded.as_str(),
        "run.storage_quota_exceeded"
    );
    assert!(AuditAction::RunQueried.collapses_repeats());
    assert_eq!(
        AuditAction::RunQueried.repeat_window(),
        Some(std::time::Duration::from_secs(300))
    );
    assert!(!AuditAction::RunUpdated.collapses_repeats());
    assert_eq!(
        AuditAction::MembershipGranted.as_str(),
        "membership.granted"
    );
    assert_eq!(
        AuditAction::CrossTenantAccessDenied.as_str(),
        "access.denied.cross_tenant"
    );
    assert_eq!(
        AuditAction::PermissionDenied.as_str(),
        "access.denied.permission"
    );
    assert_eq!(ResourceType::Endpoint.as_str(), "endpoint");
    assert_eq!(ResourceType::Tenant.as_str(), "tenant");
    assert_eq!(ResourceType::Workspace.as_str(), "workspace");
    assert_eq!(ResourceType::Namespace.as_str(), "namespace");
    assert_eq!(ResourceType::Membership.as_str(), "membership");
    assert_eq!(AuditAction::PayloadDisclosed.as_str(), "payload.disclosed");
    assert_eq!(
        AuditAction::PayloadPiiRevealed.as_str(),
        "payload.pii_revealed"
    );
    assert_eq!(
        AuditAction::PayloadRevealDenied.as_str(),
        "payload.reveal_denied"
    );
    assert_eq!(
        AuditAction::PayloadRevealGranted.as_str(),
        "payload.reveal_granted"
    );
    assert_eq!(
        AuditAction::PayloadRevealRevoked.as_str(),
        "payload.reveal_revoked"
    );
    assert_eq!(
        ResourceType::PayloadRevealGrant.as_str(),
        "payload_reveal_grant"
    );
}

#[test]
fn run_data_limit_audit_has_closed_noncollapsing_reason_vocabulary() {
    let action = AuditAction::RunDataLimitExceeded;
    assert!(!action.collapses_repeats());
    assert_eq!(action.repeat_window(), None);
    assert_eq!(action.distinct_cap(), None);
    assert_ne!(action, AuditAction::RunStorageQuotaExceeded);

    let reasons = [
        (RunDataLimitAuditReason::Aggregate, "aggregate"),
        (RunDataLimitAuditReason::ExistingOutputs, "existing_outputs"),
        (
            RunDataLimitAuditReason::StoppedForRunDataLimit,
            "stopped_for_run_data_limit",
        ),
        (
            RunDataLimitAuditReason::SuccessfulScopedOutputRefused,
            "successful scoped output refused by run data limit",
        ),
    ];
    for (reason, stored) in reasons {
        assert_eq!(reason.as_str(), stored);
        assert_eq!(reason.to_string(), stored);
    }
}

#[test]
fn payload_disclosure_audits_have_the_decided_repeat_semantics() {
    let five_minutes = Some(std::time::Duration::from_secs(300));

    for action in [
        AuditAction::PayloadDisclosed,
        AuditAction::PayloadRevealDenied,
    ] {
        assert!(action.collapses_repeats(), "{action}");
        assert_eq!(action.repeat_window(), five_minutes, "{action}");
    }

    for action in [
        AuditAction::PayloadPiiRevealed,
        AuditAction::PayloadRevealGranted,
        AuditAction::PayloadRevealRevoked,
    ] {
        assert!(!action.collapses_repeats(), "{action}");
        assert_eq!(action.repeat_window(), None, "{action}");
    }
}

#[test]
fn provisioning_records_are_attributed_to_the_system_and_cannot_be_mistaken_for_a_user() {
    let tenant = TenantId::generate();
    let record = AuditRecord::provisioning(
        tenant,
        AuditAction::MembershipGranted,
        ResourceType::Membership,
        "alice",
        "first administrator",
        OffsetDateTime::UNIX_EPOCH,
    );

    assert_eq!(record.tenant_id(), tenant);
    assert_eq!(record.actor().as_str(), "system:provisioning");
    assert_eq!(record.reason(), Some("first administrator"));
    assert_eq!(record.trace_id(), None);
    // Tokens only ever authenticate through membership lookup, and none can carry this subject.
    assert_eq!(*record.actor(), Subject::provisioning());
}

#[test]
fn capacity_settlement_audit_is_an_engine_transition_without_generic_repeat_collapse() {
    let tenant = TenantId::generate();
    let record = AuditRecord::engine(
        tenant,
        AuditAction::CapacityMemberFinished,
        ResourceType::CapacityGroup,
        "group-id",
        "member reached its own chain FINAL",
        None,
        OffsetDateTime::UNIX_EPOCH,
    );
    assert_eq!(record.tenant_id(), tenant);
    assert_eq!(*record.actor(), Subject::engine());
    assert_eq!(record.action().as_str(), "capacity.member.finished");
    assert_eq!(record.resource_type().as_str(), "capacity_group");
    assert_ne!(record.action(), AuditAction::RunFinished);
    assert!(!record.action().collapses_repeats());
    assert_eq!(record.action().repeat_window(), None);
}

#[test]
fn leaf_certification_audit_uses_noncollapsing_metadata_vocabulary() {
    let action = AuditAction::LeafCollectionSuccessCertified;
    assert!(!action.collapses_repeats());
    assert_eq!(action.repeat_window(), None);
    assert_eq!(action.distinct_cap(), None);
    assert_ne!(action, AuditAction::RunFinished);
    assert_ne!(action, AuditAction::CapacityMemberFinished);
    assert_eq!(action.as_str(), "collection.leaf_success_certified");
    assert_eq!(
        ResourceType::LeafCollectionSuccess.as_str(),
        "leaf_collection_success"
    );
}
