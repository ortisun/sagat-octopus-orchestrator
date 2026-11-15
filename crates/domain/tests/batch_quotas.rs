//! The quota resources of batch jobs (`FR-018-033`, `FR-018-045`, `T018-007`): the jobs that have not
//! finished, counted in the namespace and in the tenant.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::quota::{QuotaKind, QuotaResource, QuotaScope, QuotaUnit};

const BOTH: [QuotaResource; 2] = [QuotaResource::BatchJobs, QuotaResource::TenantBatchJobs];

#[test]
fn the_two_resources_have_their_names_and_are_found_by_name() {
    assert_eq!(QuotaResource::BatchJobs.as_str(), "batch_jobs");
    assert_eq!(QuotaResource::TenantBatchJobs.as_str(), "tenant_batch_jobs");
    for resource in BOTH {
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
fn one_is_counted_in_the_namespace_and_the_other_in_the_tenant() {
    assert_eq!(QuotaResource::BatchJobs.scope(), QuotaScope::Namespace);
    assert_eq!(QuotaResource::TenantBatchJobs.scope(), QuotaScope::Tenant);
    for resource in BOTH {
        assert_eq!(resource.kind(), QuotaKind::Count, "{resource}");
        assert_eq!(resource.unit(), QuotaUnit::Items, "{resource}");
        assert!(resource.has_usage(), "{resource}");
        assert!(!resource.is_database(), "{resource}");
    }
}

/// The design values of the plan: 2 active jobs per namespace (hard 16) and 8 per tenant (hard 64).
#[test]
fn the_defaults_and_the_hard_ceilings_are_those_of_the_plan() {
    assert_eq!(QuotaResource::BatchJobs.default_limit(), 2);
    assert_eq!(QuotaResource::BatchJobs.maximum(), 16);
    assert_eq!(QuotaResource::TenantBatchJobs.default_limit(), 8);
    assert_eq!(QuotaResource::TenantBatchJobs.maximum(), 64);
    for resource in BOTH {
        assert!(resource.default_limit() <= resource.default_ceiling());
        assert!(
            resource.default_ceiling() <= resource.maximum(),
            "{resource}"
        );
        assert!(!resource.limit_name().is_empty());
    }
    assert!(
        QuotaResource::BatchJobs.default_limit() <= QuotaResource::TenantBatchJobs.default_limit(),
        "a namespace never has more room than its tenant"
    );
}
