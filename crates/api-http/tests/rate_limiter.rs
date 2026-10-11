//! Token-bucket behaviour with an injected clock (gap G13).
#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use sagat_api_http::{MAX_RETRY_AFTER, RateLimitConfig, RateLimitConfigError, RateLimiter};
use sagat_domain::identity::TenantId;

fn limiter(per_second: f64, burst: f64) -> RateLimiter {
    RateLimiter::new(RateLimitConfig { per_second, burst }).expect("a valid budget")
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
    assert!(limiter.tracked_keys() > 10_000);

    // After enough idle time every bucket is full again and carries no information.
    limiter
        .check_at(TenantId::generate(), start + Duration::from_secs(60))
        .expect("one more");

    assert!(
        limiter.tracked_keys() < 100,
        "tracked: {}",
        limiter.tracked_keys()
    );
}

#[test]
fn a_late_stale_timestamp_cannot_rewind_the_refill_clock() {
    // rate 1/s, burst 1: the bucket is emptied at +10 s, then a call arrives with an earlier
    // timestamp (clocks of concurrent requests are not ordered). It must not move `updated`
    // backwards, or the next call would see 10.5 s of refill instead of 0.5 s.
    let limiter = limiter(1.0, 1.0);
    let tenant = TenantId::generate();
    let t0 = Instant::now();
    limiter
        .check_at(tenant, t0 + Duration::from_secs(10))
        .expect("first");
    assert!(limiter.check_at(tenant, t0).is_err());

    assert!(
        limiter
            .check_at(tenant, t0 + Duration::from_millis(10_500))
            .is_err(),
        "only half a token has accrued since the last forward timestamp"
    );
}

#[test]
fn pruning_is_throttled_to_once_per_interval_not_once_per_call() {
    // 100 req/s, burst 10: a bucket is full again after 0.1 s of idleness.
    let limiter = limiter(100.0, 10.0);
    let t0 = Instant::now();
    for _ in 0..10_050 {
        limiter
            .check_at(TenantId::generate(), t0)
            .expect("first request of a tenant");
    }
    assert!(limiter.tracked_keys() > 10_000);

    // Half a second later every bucket is stale, but a prune just ran: the table is not walked
    // again under the global lock on every call.
    limiter
        .check_at(TenantId::generate(), t0 + Duration::from_millis(500))
        .expect("one more");
    assert!(
        limiter.tracked_keys() > 10_000,
        "pruned again too soon: {}",
        limiter.tracked_keys()
    );

    // Once the interval has passed the idle buckets are released.
    limiter
        .check_at(TenantId::generate(), t0 + Duration::from_millis(1500))
        .expect("one more");
    assert!(
        limiter.tracked_keys() < 100,
        "tracked: {}",
        limiter.tracked_keys()
    );
}

#[test]
fn unusable_budgets_are_refused_with_a_typed_error() {
    let bad_rate = [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300, 0.000_999];
    for per_second in bad_rate {
        let config = RateLimitConfig {
            per_second,
            burst: 10.0,
        };
        assert_eq!(
            RateLimiter::<TenantId>::new(config).expect_err("rate"),
            RateLimitConfigError::PerSecond,
            "{per_second}"
        );
    }
    for burst in [0.0, 0.99, -3.0, f64::NAN, f64::INFINITY] {
        let config = RateLimitConfig {
            per_second: 10.0,
            burst,
        };
        assert_eq!(
            RateLimiter::<TenantId>::new(config).expect_err("burst"),
            RateLimitConfigError::Burst,
            "{burst}"
        );
    }
    // The boundaries themselves are fine.
    assert!(
        RateLimiter::<TenantId>::new(RateLimitConfig {
            per_second: RateLimitConfig::MIN_PER_SECOND,
            burst: RateLimitConfig::MIN_BURST,
        })
        .is_ok()
    );
    assert!(RateLimitConfig::default().validate().is_ok());
    assert!(RateLimitConfig::subject_default().validate().is_ok());
}

#[test]
fn the_slowest_accepted_rate_never_panics_and_caps_retry_after() {
    // The review's `SAGAT_RATE_LIMIT_RPS=1e-300` panicked in Duration::from_secs_f64 on the first
    // over-budget request. The smallest rate that is still accepted must report a sane wait.
    let limiter = limiter(RateLimitConfig::MIN_PER_SECOND, 1.0);
    let tenant = TenantId::generate();
    let now = Instant::now();
    limiter.check_at(tenant, now).expect("the burst");

    let wait = limiter
        .check_at(tenant, now)
        .expect_err("over budget with a token every 1000 s");

    assert!(wait > Duration::ZERO && wait <= MAX_RETRY_AFTER, "{wait:?}");
}

#[test]
fn keys_beyond_the_capacity_share_one_overflow_bucket() {
    // Capacity 4, burst 2: a flood of fresh keys must not grow the table, must not each get a
    // fresh allowance, and must not touch the budget of keys that are already tracked.
    let limiter = RateLimiter::<u32>::with_capacity(
        RateLimitConfig {
            per_second: 1.0,
            burst: 2.0,
        },
        4,
    )
    .expect("valid");
    let now = Instant::now();
    for key in 0..4 {
        limiter.check_at(key, now).expect("tracked key, first call");
    }
    assert_eq!(limiter.tracked_keys(), 4);

    // Untracked keys 100.. share the overflow bucket (burst 2).
    assert!(limiter.check_at(100, now).is_ok());
    assert!(limiter.check_at(101, now).is_ok());
    assert!(
        limiter.check_at(102, now).is_err(),
        "a third fresh key does not get a fresh allowance"
    );
    assert_eq!(limiter.tracked_keys(), 4, "the table did not grow");

    // A tracked key keeps its own bucket.
    assert!(limiter.check_at(0, now).is_ok(), "key 0 had used 1 of 2");
    assert!(limiter.check_at(0, now).is_err());
    assert!(limiter.check_at(1, now).is_ok(), "and key 1 is independent");
}

#[test]
fn a_full_table_recovers_once_idle_keys_are_swept() {
    let limiter = RateLimiter::<u32>::with_capacity(
        RateLimitConfig {
            per_second: 10.0,
            burst: 2.0,
        },
        4,
    )
    .expect("valid");
    let t0 = Instant::now();
    for key in 0..4 {
        limiter.check_at(key, t0).expect("fill");
    }

    // 5 s later every bucket has refilled and the sweep interval has passed: room for new keys.
    limiter
        .check_at(500, t0 + Duration::from_secs(5))
        .expect("new key");
    assert_eq!(limiter.tracked_keys(), 1, "the idle keys were released");
}

#[test]
fn the_limiter_works_for_string_keys_too() {
    let limiter = RateLimiter::<String>::new(RateLimitConfig {
        per_second: 1.0,
        burst: 1.0,
    })
    .expect("valid");
    let now = Instant::now();
    assert!(limiter.check_at("alice".to_owned(), now).is_ok());
    assert!(limiter.check_at("alice".to_owned(), now).is_err());
    assert!(limiter.check_at("bob".to_owned(), now).is_ok());
}
