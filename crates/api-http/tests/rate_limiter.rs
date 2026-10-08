//! Token-bucket behaviour with an injected clock (gap G13).
#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use sagat_api_http::{RateLimitConfig, RateLimiter};
use sagat_domain::identity::TenantId;

fn limiter(per_second: f64, burst: f64) -> RateLimiter {
    RateLimiter::new(RateLimitConfig { per_second, burst })
}

#[test]
fn a_tenant_may_burst_up_to_its_budget_and_then_is_refused() {
    let limiter = limiter(10.0, 5.0);
    let tenant = TenantId::generate();
    let now = Instant::now();

    for i in 0..5 {
        assert!(
            limiter.check_at(tenant, now).is_ok(),
            "request {i} is inside the burst"
        );
    }
    let retry_after = limiter
        .check_at(tenant, now)
        .expect_err("the 6th request exceeds the burst");

    // One token takes 1/10 s to come back.
    assert!(
        retry_after > Duration::from_millis(90) && retry_after <= Duration::from_millis(101),
        "{retry_after:?}"
    );
}

#[test]
fn tokens_come_back_at_the_sustained_rate_and_never_beyond_the_burst() {
    let limiter = limiter(10.0, 5.0);
    let tenant = TenantId::generate();
    let start = Instant::now();
    for _ in 0..5 {
        limiter.check_at(tenant, start).expect("burst");
    }
    assert!(limiter.check_at(tenant, start).is_err());

    // 250 ms later: 2.5 tokens, so two requests pass and the third does not.
    let later = start + Duration::from_millis(250);
    assert!(limiter.check_at(tenant, later).is_ok());
    assert!(limiter.check_at(tenant, later).is_ok());
    assert!(limiter.check_at(tenant, later).is_err());

    // A long idle period refills to the burst, not to infinity.
    let much_later = start + Duration::from_secs(3600);
    for i in 0..5 {
        assert!(limiter.check_at(tenant, much_later).is_ok(), "{i}");
    }
    assert!(limiter.check_at(tenant, much_later).is_err());
}

#[test]
fn tenants_have_independent_budgets() {
    let limiter = limiter(1.0, 2.0);
    let (noisy, quiet) = (TenantId::generate(), TenantId::generate());
    let now = Instant::now();

    limiter.check_at(noisy, now).expect("1");
    limiter.check_at(noisy, now).expect("2");
    assert!(
        limiter.check_at(noisy, now).is_err(),
        "the noisy tenant is exhausted"
    );

    assert!(
        limiter.check_at(quiet, now).is_ok(),
        "another tenant is unaffected"
    );
    assert!(limiter.check_at(quiet, now).is_ok());
}

#[test]
fn a_clock_that_goes_backwards_cannot_mint_tokens() {
    let limiter = limiter(1000.0, 1.0);
    let tenant = TenantId::generate();
    let now = Instant::now();
    limiter
        .check_at(tenant, now + Duration::from_secs(10))
        .expect("first");

    // An earlier timestamp must not be treated as elapsed time.
    assert!(limiter.check_at(tenant, now).is_err());
}

#[test]
fn idle_tenants_are_forgotten_once_the_table_grows() {
    let limiter = limiter(100.0, 10.0);
    let start = Instant::now();
    for _ in 0..10_050 {
        limiter
            .check_at(TenantId::generate(), start)
            .expect("first request of a tenant");
    }
    assert!(limiter.tracked_tenants() > 10_000);

    // After enough idle time every bucket is full again and carries no information.
    limiter
        .check_at(TenantId::generate(), start + Duration::from_secs(60))
        .expect("one more");

    assert!(
        limiter.tracked_tenants() < 100,
        "tracked: {}",
        limiter.tracked_tenants()
    );
}
