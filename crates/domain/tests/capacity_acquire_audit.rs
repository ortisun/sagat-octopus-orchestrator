use sagat_domain::audit::AuditAction;

#[test]
fn capacity_acquire_audit_keeps_every_transition() {
    let action = AuditAction::CapacityMemberAcquired;
    assert_eq!(action.as_str(), "capacity.member.acquired");
    assert!(!action.collapses_repeats());
    assert_eq!(action.repeat_window(), None);
    assert_eq!(action.distinct_cap(), None);
}
