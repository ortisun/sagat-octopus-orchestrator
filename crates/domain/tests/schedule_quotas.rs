//! The quota resources of schedules (`FR-018-045`, `T018-007`). Batch jobs and pending resets get
//! theirs with the tables they count.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use sagat_domain::quota::{QuotaKind, QuotaResource, QuotaScope, QuotaUnit};

#[test]
fn the_two_resources_have_their_names_and_are_found_by_name() {
    assert_eq!(QuotaResource::Schedules.as_str(), "schedules");
    assert_eq!(
        QuotaResource::ScheduleBackfills.as_str(),
        "schedule_backfills"
    );
    for resource in [QuotaResource::Schedules, QuotaResource::ScheduleBackfills] {
        assert_eq!(resource.as_str().parse::<QuotaResource>(), Ok(resource));
        assert_eq!(
            QuotaResource::ALL
                .iter()
                .filter(|r| **r == resource)
                .count(),
            1
        );
    }
}

#[test]
fn every_resource_has_its_own_name() {
    let names: BTreeSet<_> = QuotaResource::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(names.len(), QuotaResource::ALL.len());
}

#[test]
fn both_are_counts_of_the_namespace_that_report_their_usage() {
    for resource in [QuotaResource::Schedules, QuotaResource::ScheduleBackfills] {
        assert_eq!(resource.scope(), QuotaScope::Namespace, "{resource}");
        assert_eq!(resource.kind(), QuotaKind::Count, "{resource}");
        assert_eq!(resource.unit(), QuotaUnit::Items, "{resource}");
        assert!(resource.has_usage(), "{resource}");
        assert!(!resource.is_database(), "{resource}");
    }
}

/// The design values of the plan: 200 schedules per namespace (hard 10,000) and 4 concurrent
/// backfills (hard 16).
#[test]
fn the_defaults_and_the_hard_ceilings_are_those_of_the_plan() {
    assert_eq!(QuotaResource::Schedules.default_limit(), 200);
    assert_eq!(QuotaResource::Schedules.maximum(), 10_000);
    assert_eq!(QuotaResource::ScheduleBackfills.default_limit(), 4);
    assert_eq!(QuotaResource::ScheduleBackfills.maximum(), 16);
    for resource in [QuotaResource::Schedules, QuotaResource::ScheduleBackfills] {
        assert!(resource.default_limit() <= resource.default_ceiling());
        assert!(
            resource.default_ceiling() <= resource.maximum(),
            "{resource}"
        );
        assert!(!resource.limit_name().is_empty());
    }
}
