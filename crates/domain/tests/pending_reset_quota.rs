//! The quota resource of the resets that wait for their source run to end (`CANCEL_SOURCE`, `FR-018-038`,
//! `FR-018-045`, `T018-057`): counted in the namespace, 16 by default and 64 at most.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::quota::{QuotaKind, QuotaResource, QuotaScope, QuotaUnit};

#[test]
fn the_resource_has_its_name_and_is_found_by_name_once() {
    let resource = QuotaResource::PendingResets;
    assert_eq!(resource.as_str(), "pending_resets");
    assert_eq!(resource.as_str().parse::<QuotaResource>(), Ok(resource));
    assert_eq!(
        QuotaResource::ALL
            .iter()
            .filter(|r| **r == resource)
            .count(),
        1
    );
}

#[test]
fn it_is_a_count_of_the_namespace_and_nothing_a_database_policy() {
    let resource = QuotaResource::PendingResets;
    assert_eq!(resource.scope(), QuotaScope::Namespace);
    assert_eq!(resource.kind(), QuotaKind::Count);
    assert_eq!(resource.unit(), QuotaUnit::Items);
    assert!(resource.has_usage());
    assert!(!resource.is_database());
    assert!(!resource.limit_name().is_empty());
}

/// The design values of the plan: 16 pending resets per namespace by default, 64 at most.
#[test]
fn the_default_and_the_hard_ceiling_are_those_of_the_plan() {
    let resource = QuotaResource::PendingResets;
    assert_eq!(resource.default_limit(), 16);
    assert_eq!(resource.maximum(), 64);
    assert!(resource.default_limit() <= resource.default_ceiling());
    assert!(resource.default_ceiling() <= resource.maximum());
}
