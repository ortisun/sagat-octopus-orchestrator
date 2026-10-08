//! The per-tenant cap on requests in flight (second security review, finding 2).

use std::sync::Arc;

use sagat_api_http::{HttpLimits, TenantAtCapacity, TenantConcurrency};
use sagat_domain::identity::TenantId;

#[test]
fn a_tenant_gets_exactly_its_cap_and_others_are_unaffected() {
    let gate = Arc::new(TenantConcurrency::new(2));
    let (noisy, quiet) = (TenantId::generate(), TenantId::generate());

    let first = gate.try_acquire(noisy).expect("1st");
    let second = gate.try_acquire(noisy).expect("2nd");
    assert_eq!(
        gate.try_acquire(noisy).expect_err("3rd"),
        TenantAtCapacity { cap: 2 }
    );

    assert!(gate.try_acquire(quiet).is_ok(), "another tenant is served");
    drop((first, second));
}

#[test]
fn finishing_a_request_frees_its_slot() {
    let gate = Arc::new(TenantConcurrency::new(1));
    let tenant = TenantId::generate();

    let permit = gate.try_acquire(tenant).expect("first");
    assert!(gate.try_acquire(tenant).is_err());
    drop(permit);

    assert!(gate.try_acquire(tenant).is_ok(), "the slot came back");
}

#[test]
fn the_table_only_holds_tenants_with_requests_in_flight() {
    let gate = Arc::new(TenantConcurrency::new(3));
    let mut held = Vec::new();
    for _ in 0..50 {
        held.push(gate.try_acquire(TenantId::generate()).expect("permit"));
    }
    assert_eq!(gate.tenants_in_flight(), 50);

    held.clear();

    assert_eq!(gate.tenants_in_flight(), 0, "idle tenants leave no entry");
}

#[test]
fn a_refused_request_leaves_no_residue() {
    let gate = Arc::new(TenantConcurrency::new(1));
    let tenant = TenantId::generate();
    let permit = gate.try_acquire(tenant).expect("first");
    for _ in 0..100 {
        assert!(gate.try_acquire(tenant).is_err());
    }
    drop(permit);

    assert_eq!(gate.tenants_in_flight(), 0);
    assert!(gate.try_acquire(tenant).is_ok());
}

#[test]
fn a_cap_of_zero_is_raised_to_one_instead_of_refusing_everybody() {
    let gate = Arc::new(TenantConcurrency::new(0));
    assert_eq!(gate.cap(), 1);
    assert!(gate.try_acquire(TenantId::generate()).is_ok());
}

#[test]
fn the_default_cap_is_a_quarter_of_the_global_cap_and_at_least_one() {
    assert_eq!(HttpLimits::default_per_tenant(512), 128);
    assert_eq!(HttpLimits::default_per_tenant(8), 2);
    assert_eq!(HttpLimits::default_per_tenant(3), 1);
    assert_eq!(HttpLimits::default_per_tenant(1), 1);
    assert_eq!(
        HttpLimits::default().max_in_flight_per_tenant,
        HttpLimits::default_per_tenant(HttpLimits::default().max_in_flight)
    );
}
