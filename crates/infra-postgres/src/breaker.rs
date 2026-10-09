//! The circuit breaker of compensations (FR-003-003): per tenant, namespace and activity, kept in
//! `compensation_breakers`, written in the transaction that records the outcome of an attempt and
//! read in the one that leases the next.
//!
//! Closed: compensations are leased. After `threshold` failed attempts in a row it opens for
//! `open_for`. When that window is over the next lease is the *probe*: its success deletes the row,
//! its failure reopens the breaker for twice as long (up to `max_open`). A failure that arrives
//! while the breaker is open, or half open with no probe in flight, belongs to an attempt that was
//! already running when it opened: it is counted and changes nothing else, so a burst of such
//! failures cannot walk the window up to its ceiling without a single probe.

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::execution::BreakerPolicy;
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::identity::store_error;

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

/// What the breaker remembers about one provider operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct State {
    pub failures: u32,
    /// `None`: closed. Later than now: open, or a probe is in flight. Earlier: half open.
    pub opened_until: Option<OffsetDateTime>,
    pub probing: bool,
    pub level: u32,
}

impl State {
    /// How long the breaker stays open after `level` failed probes: the base doubled that many
    /// times, never more than the ceiling (the exponent is capped so the arithmetic cannot
    /// overflow).
    fn window(policy: BreakerPolicy, level: u32) -> Duration {
        policy
            .open_for
            .saturating_mul(1u32 << level.min(20))
            .min(policy.max_open)
    }

    /// The state after an attempt of a compensation ended without success, and whether that
    /// opened (or reopened) the breaker.
    pub(crate) fn failed(self, policy: BreakerPolicy, now: OffsetDateTime) -> (Self, bool) {
        let failures = self.failures.saturating_add(1);
        let reopen = |level: u32| Self {
            failures,
            opened_until: now.checked_add(
                time::Duration::try_from(Self::window(policy, level))
                    .unwrap_or(time::Duration::MAX),
            ),
            probing: false,
            level,
        };
        match (self.opened_until, self.probing) {
            // The probe failed: the provider is still down.
            (_, true) => (reopen(self.level.saturating_add(1)), true),
            // Closed, and this is the failure that reaches the threshold.
            (None, false) if failures >= policy.threshold => (reopen(0), true),
            // Closed, below the threshold; or already open (an attempt that was in flight).
            _ => (Self { failures, ..self }, false),
        }
    }
}

/// An attempt of a compensation of `activity` ended without success. Returns whether the breaker
/// opened (or reopened) because of it.
pub(crate) async fn record_failure(
    tx: &mut Transaction<'static, Postgres>,
    policy: BreakerPolicy,
    tenant: Uuid,
    namespace: Uuid,
    activity: &str,
) -> Result<bool, StoreError> {
    if policy.threshold == 0 {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO compensation_breakers \
         (tenant_id, namespace_id, activity, failures, probing, level, updated_at) \
         VALUES ($1, $2, $3, 0, false, 0, clock_timestamp()) ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(activity)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    // The row is locked while it is read and rewritten, so concurrent outcomes are applied one
    // after the other and none is lost.
    let (failures, opened_until, probing, level, now): (
        i32,
        Option<OffsetDateTime>,
        bool,
        i32,
        OffsetDateTime,
    ) = sqlx::query_as(
        "SELECT failures, opened_until, probing, level, clock_timestamp() \
         FROM compensation_breakers \
         WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3 FOR UPDATE",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(activity)
    .fetch_one(&mut **tx)
    .await
    .map_err(store_error)?;
    let (next, opened) = State {
        failures: u32::try_from(failures).unwrap_or(0),
        opened_until,
        probing,
        level: u32::try_from(level).unwrap_or(0),
    }
    .failed(policy, now);
    sqlx::query(
        "UPDATE compensation_breakers \
         SET failures = $4, opened_until = $5, probing = $6, level = $7, \
             updated_at = clock_timestamp() \
         WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(activity)
    .bind(i32::try_from(next.failures).unwrap_or(i32::MAX))
    .bind(next.opened_until)
    .bind(next.probing)
    .bind(i32::try_from(next.level).unwrap_or(i32::MAX))
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    Ok(opened)
}

/// An attempt of a compensation of `activity` succeeded: the provider answers, the breaker closes.
pub(crate) async fn record_success(
    tx: &mut Transaction<'static, Postgres>,
    policy: BreakerPolicy,
    tenant: Uuid,
    namespace: Uuid,
    activity: &str,
) -> Result<(), StoreError> {
    if policy.threshold == 0 {
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM compensation_breakers \
         WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(activity)
    .execute(&mut **tx)
    .await
    .map_err(store_error)?;
    Ok(())
}

/// Whether a compensation of `activity` may be leased now, and, when the window is over, claims the
/// probe: of all the callers that find the breaker half open exactly one wins the update and the
/// others are told no until the probe's window (`open_for`) has passed or it reports.
pub(crate) async fn admit(
    tx: &mut Transaction<'static, Postgres>,
    policy: BreakerPolicy,
    tenant: Uuid,
    namespace: Uuid,
    activity: &str,
) -> Result<bool, StoreError> {
    if policy.threshold == 0 {
        return Ok(true);
    }
    let row: Option<(Option<bool>,)> = sqlx::query_as(
        "SELECT opened_until > clock_timestamp() FROM compensation_breakers \
         WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3",
    )
    .bind(tenant)
    .bind(namespace)
    .bind(activity)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store_error)?;
    match row {
        // Never failed, or failures below the threshold: closed.
        None | Some((None,)) => Ok(true),
        Some((Some(true),)) => Ok(false),
        // The window is over: whoever moves the deadline first is the probe.
        Some((Some(false),)) => {
            let claimed = sqlx::query(
                "UPDATE compensation_breakers \
                 SET opened_until = clock_timestamp() + $4 * interval '1 microsecond', \
                     probing = true \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND activity = $3 \
                   AND opened_until <= clock_timestamp()",
            )
            .bind(tenant)
            .bind(namespace)
            .bind(activity)
            .bind(micros(policy.open_for))
            .execute(&mut **tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            Ok(claimed == 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: BreakerPolicy = BreakerPolicy {
        threshold: 3,
        open_for: Duration::from_secs(30),
        max_open: Duration::from_secs(100),
    };

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(seconds)
    }

    const CLOSED: State = State {
        failures: 0,
        opened_until: None,
        probing: false,
        level: 0,
    };

    #[test]
    fn it_opens_at_the_threshold_and_not_before() {
        let (one, opened) = CLOSED.failed(POLICY, at(0));
        assert!(!opened && one.opened_until.is_none() && one.failures == 1);
        let (two, opened) = one.failed(POLICY, at(1));
        assert!(!opened && two.opened_until.is_none());
        let (three, opened) = two.failed(POLICY, at(2));
        assert!(opened, "the third failure opens it");
        assert_eq!(three.opened_until, Some(at(32)), "for the base window");
        assert_eq!((three.failures, three.level, three.probing), (3, 0, false));
    }

    #[test]
    fn failures_of_attempts_that_were_already_running_do_not_walk_the_window_up() {
        let (open, _) = State {
            failures: 2,
            ..CLOSED
        }
        .failed(POLICY, at(0));
        let mut state = open;
        for second in 1..30 {
            let (next, opened) = state.failed(POLICY, at(second));
            assert!(!opened, "an attempt in flight is not a new reason to open");
            assert_eq!(
                next.opened_until, open.opened_until,
                "the window did not move"
            );
            assert_eq!(next.level, 0);
            state = next;
        }
        assert_eq!(state.failures, 3 + 29, "but they are counted");
    }

    #[test]
    fn a_probe_that_fails_reopens_it_for_twice_as_long_up_to_the_ceiling() {
        let probing = State {
            failures: 3,
            opened_until: Some(at(0)),
            probing: true,
            level: 0,
        };
        let (first, opened) = probing.failed(POLICY, at(40));
        assert!(opened);
        assert_eq!(first.opened_until, Some(at(40 + 60)), "twice the base");
        assert!(!first.probing && first.level == 1);
        let (second, _) = State {
            probing: true,
            ..first
        }
        .failed(POLICY, at(200));
        assert_eq!(
            second.opened_until,
            Some(at(200 + 100)),
            "the ceiling, not 120 seconds"
        );
        let (third, _) = State {
            probing: true,
            ..second
        }
        .failed(POLICY, at(400));
        assert_eq!(third.opened_until, Some(at(400 + 100)));
        assert_eq!(third.level, 3);
    }

    #[test]
    fn the_exponent_cannot_overflow_however_often_the_probe_fails() {
        let mut state = State {
            failures: 3,
            opened_until: Some(at(0)),
            probing: true,
            level: u32::MAX - 1,
        };
        for _ in 0..3 {
            let (next, _) = state.failed(POLICY, at(0));
            assert_eq!(next.opened_until, Some(at(100)));
            state = State {
                probing: true,
                ..next
            };
        }
    }
}
