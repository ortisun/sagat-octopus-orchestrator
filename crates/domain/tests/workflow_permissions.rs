use sagat_domain::identity::{Permission, Role};

#[test]
fn declared_operations_and_human_decisions_have_explicit_role_grants() {
    use Permission::{ApprovalDecide, ApprovalRead, RunQuery, RunUpdate};

    for role in [Role::Admin, Role::Developer, Role::Operator] {
        for permission in [ApprovalRead, ApprovalDecide, RunQuery, RunUpdate] {
            assert!(role.grants(permission), "{role} needs {permission}");
        }
    }
    assert!(Role::Viewer.grants(ApprovalRead));
    for permission in [ApprovalDecide, RunQuery, RunUpdate] {
        assert!(!Role::Viewer.grants(permission));
    }
    for role in [Role::Worker, Role::Runner] {
        for permission in [ApprovalRead, ApprovalDecide, RunQuery, RunUpdate] {
            assert!(!role.grants(permission));
        }
    }
}
