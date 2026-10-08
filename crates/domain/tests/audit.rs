//! Audit record invariants (FR-001-007).
#![allow(clippy::expect_used)]

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
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
    assert_eq!(AuditAction::WorkspaceCreated.as_str(), "workspace.created");
    assert_eq!(AuditAction::NamespaceCreated.as_str(), "namespace.created");
    assert_eq!(
        AuditAction::MembershipGranted.as_str(),
        "membership.granted"
    );
    assert_eq!(
        AuditAction::CrossTenantAccessDenied.as_str(),
        "access.denied.cross_tenant"
    );
    assert_eq!(ResourceType::Workspace.as_str(), "workspace");
    assert_eq!(ResourceType::Namespace.as_str(), "namespace");
    assert_eq!(ResourceType::Membership.as_str(), "membership");
}
