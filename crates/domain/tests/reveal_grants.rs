use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::identity::{Permission, Role};

#[test]
fn reveal_is_never_inherited_from_a_role_and_only_eligible_roles_read_internal_values() {
    for role in [
        Role::Admin,
        Role::Developer,
        Role::Operator,
        Role::Viewer,
        Role::Worker,
        Role::Runner,
    ] {
        assert!(!role.grants(Permission::PayloadRevealPii), "{role}");
    }
    assert!(Role::Admin.grants(Permission::PayloadReadInternal));
    assert!(Role::Developer.grants(Permission::PayloadReadInternal));
    assert!(Role::Operator.grants(Permission::PayloadReadInternal));
    assert!(!Role::Viewer.grants(Permission::PayloadReadInternal));
}

#[test]
fn grant_audit_vocabulary_is_explicit_and_never_collapses() {
    assert_eq!(
        AuditAction::PayloadRevealGranted.as_str(),
        "payload.reveal_granted"
    );
    assert_eq!(
        AuditAction::PayloadRevealRevoked.as_str(),
        "payload.reveal_revoked"
    );
    assert!(!AuditAction::PayloadRevealGranted.collapses_repeats());
    assert!(!AuditAction::PayloadRevealRevoked.collapses_repeats());
    assert_eq!(
        ResourceType::PayloadRevealGrant.as_str(),
        "payload_reveal_grant"
    );
}
