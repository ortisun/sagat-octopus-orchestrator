//! Events out: delivering what happened on the platform to the endpoints that asked to be told
//! (`FR-004-006`).
//!
//! A subscription names a destination and the kinds of event it wants. Delivery is at-least-once:
//! an attempt that cannot say whether the receiver got the event is repeated, so every request
//! carries the id of the delivery, which does not change between attempts, and a receiver that
//! keeps it can drop the repeat. What is retried, how long it waits and when it gives up are
//! decided here, as functions, so they can be tested without a network.

use std::fmt;
use std::time::Duration;

use crate::egress::Destination;

uuid_id!(
    /// Identifier of a subscription to outbound events.
    SubscriptionId
);
uuid_id!(
    /// Identifier of one delivery of one event to one subscription. It is also the idempotency key
    /// the receiver sees, so it never changes between the attempts of a delivery.
    DeliveryId
);

/// Attempts a delivery gets (the first and the retries) before it is set aside as dead.
pub const MAX_DELIVERY_ATTEMPTS: u32 = 12;
/// The longest the receiver may ask to wait with `Retry-After`; a longer request is capped.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
/// Deliveries that may be waiting for one subscription. A subscription that reaches it is not
/// keeping up (a receiver that answers too slowly, a rate too low for the events), and the platform
/// switches it off rather than queue for it without end; turning it on again redrives what was dead.
pub const MAX_PENDING_PER_SUBSCRIPTION: u64 = 10_000;
/// Dead deliveries in a row after which a subscription is switched off instead of being tried on
/// every new event for ever.
pub const DISABLE_AFTER_DEAD: u32 = 20;
/// Longest URL accepted.
pub const MAX_URL_LEN: usize = 2048;
/// Event kinds one subscription may name.
pub const MAX_EVENT_KINDS: usize = 8;

/// Why a subscription setting was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutboundError {
    #[error("the URL must be http or https, with a host, no credentials and no fragment")]
    InvalidUrl,
    #[error("a subscription names between 1 and {MAX_EVENT_KINDS} event types it can deliver")]
    InvalidEventKinds,
    #[error("the rate is between 1 and 100 deliveries per second")]
    InvalidRate,
    #[error("the timeout is between 1 and 30 seconds")]
    InvalidTimeout,
}

/// A kind of event the platform delivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OutboundKind {
    /// A run reached a terminal status.
    RunFinished,
}

impl OutboundKind {
    pub const ALL: [Self; 1] = [Self::RunFinished];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunFinished => "run.finished",
        }
    }
}

impl fmt::Display for OutboundKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OutboundKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == value)
            .ok_or(())
    }
}

/// Whether a subscription is sent events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Active,
    /// Held by an administrator: new events are not queued for it; what is queued waits.
    Paused,
    /// Switched off by the platform after [`DISABLE_AFTER_DEAD`] dead deliveries in a row: nothing
    /// is queued until an administrator turns it back on, which also redrives what is dead.
    Disabled,
}

impl SubscriptionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
            Self::Disabled => "DISABLED",
        }
    }
}

impl std::str::FromStr for SubscriptionStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "PAUSED" => Ok(Self::Paused),
            "DISABLED" => Ok(Self::Disabled),
            _ => Err(()),
        }
    }
}

/// Where a delivery stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    /// Waiting for its turn (or for its next attempt).
    Pending,
    Delivered,
    /// Given up on: the receiver refused it for good, or every attempt failed. Kept, with all its
    /// attempts, until it is redriven or expires.
    Dead,
}

impl DeliveryStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Delivered => "DELIVERED",
            Self::Dead => "DEAD",
        }
    }
}

impl std::str::FromStr for DeliveryStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "PENDING" => Ok(Self::Pending),
            "DELIVERED" => Ok(Self::Delivered),
            "DEAD" => Ok(Self::Dead),
            _ => Err(()),
        }
    }
}

/// How one attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// A 2xx: the receiver has it.
    Delivered,
    /// A 4xx that retrying will not change.
    Rejected,
    /// A 5xx, or a status that asks to come back later.
    ServerError,
    /// `429`: the receiver asks for fewer requests.
    Throttled,
    /// No answer in time. The receiver may or may not have the event.
    Timeout,
    /// The connection failed or broke. The receiver may or may not have the event.
    Network,
    /// The egress policy of the namespace refused the destination.
    Denied,
    /// There was nothing to sign with: the secret was deleted, or no key of the ring opens it.
    Unsigned,
}

impl AttemptOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "DELIVERED",
            Self::Rejected => "REJECTED",
            Self::ServerError => "SERVER_ERROR",
            Self::Throttled => "THROTTLED",
            Self::Timeout => "TIMEOUT",
            Self::Network => "NETWORK",
            Self::Denied => "DENIED",
            Self::Unsigned => "UNSIGNED",
        }
    }
}

impl std::str::FromStr for AttemptOutcome {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        [
            Self::Delivered,
            Self::Rejected,
            Self::ServerError,
            Self::Throttled,
            Self::Timeout,
            Self::Network,
            Self::Denied,
            Self::Unsigned,
        ]
        .into_iter()
        .find(|o| o.as_str() == value)
        .ok_or(())
    }
}

/// What to do next about a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Done,
    /// Try again; `after` is what the receiver asked for, if it asked.
    Retry {
        after: Option<Duration>,
    },
    /// Do not try again.
    Dead,
}

/// Judges the answer of a receiver.
///
/// Redirects are not followed (a signed event must not be handed to another host), so a `3xx` is a
/// refusal; a `4xx` is the receiver saying the event is wrong, which sending it again will not fix,
/// except the ones that mean "not now" (`408`, `425`, `429`).
#[must_use]
pub fn judge_response(status: u16, retry_after: Option<Duration>) -> (AttemptOutcome, Verdict) {
    let retry_after = retry_after.map(|d| d.min(MAX_RETRY_AFTER));
    match status {
        200..=299 => (AttemptOutcome::Delivered, Verdict::Done),
        429 => (
            AttemptOutcome::Throttled,
            Verdict::Retry { after: retry_after },
        ),
        408 | 425 | 500..=599 => (
            AttemptOutcome::ServerError,
            Verdict::Retry { after: retry_after },
        ),
        _ => (AttemptOutcome::Rejected, Verdict::Dead),
    }
}

/// How long to wait before attempt number `failed + 1`, after `failed` attempts have failed.
///
/// A fixed schedule that stretches over about three days (5 s, 30 s, 2 min, 10 min, 30 min, 1 h, 3 h,
/// 6 h, 12 h, 24 h, 24 h), each step spread by up to ±20 % with `jitter` in `[0, 1)` so that the
/// deliveries a receiver's outage held back do not come back in the same second. What the receiver
/// asked for with `Retry-After` is honoured when it is longer than the schedule.
#[must_use]
pub fn delivery_backoff(failed: u32, jitter: f64, retry_after: Option<Duration>) -> Duration {
    const STEPS: [u64; 11] = [
        5, 30, 120, 600, 1_800, 3_600, 10_800, 21_600, 43_200, 86_400, 86_400,
    ];
    let index = usize::try_from(failed.max(1) - 1)
        .unwrap_or(usize::MAX)
        .min(STEPS.len() - 1);
    let base = Duration::from_secs(STEPS[index]);
    let spread = base.mul_f64(0.8 + 0.4 * jitter.clamp(0.0, 1.0));
    spread.max(retry_after.unwrap_or_default().min(MAX_RETRY_AFTER))
}

/// Whether a delivery that has failed `failed` times is out of attempts.
#[must_use]
pub const fn out_of_attempts(failed: u32) -> bool {
    failed >= MAX_DELIVERY_ATTEMPTS
}

/// A URL a subscription delivers to, taken apart far enough to be judged by the egress policy.
/// Whether the host may be reached is the policy's decision; this only refuses what cannot be a
/// destination at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetUrl {
    text: String,
    scheme_is_https: bool,
    host: String,
    port: u16,
}

impl TargetUrl {
    /// # Errors
    /// [`OutboundError::InvalidUrl`].
    pub fn parse(text: &str) -> Result<Self, OutboundError> {
        if text.len() > MAX_URL_LEN || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(OutboundError::InvalidUrl);
        }
        let (scheme, rest) = text.split_once("://").ok_or(OutboundError::InvalidUrl)?;
        let https = match scheme {
            "https" => true,
            "http" => false,
            _ => return Err(OutboundError::InvalidUrl),
        };
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(authority_end);
        if authority.is_empty() || authority.contains('@') || tail.contains('#') {
            return Err(OutboundError::InvalidUrl);
        }
        let (host, port) = match authority.rsplit_once(':') {
            // An IPv6 literal is bracketed; its colons are not a port separator.
            Some((host, port)) if !authority.ends_with(']') => (
                host,
                Some(port.parse::<u16>().map_err(|_| OutboundError::InvalidUrl)?),
            ),
            _ => (authority, None),
        };
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if bare.is_empty()
            || !bare
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        {
            return Err(OutboundError::InvalidUrl);
        }
        Ok(Self {
            text: text.to_owned(),
            scheme_is_https: https,
            host: bare.to_ascii_lowercase(),
            port: port.unwrap_or(if https { 443 } else { 80 }),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// What the egress policy judges.
    #[must_use]
    pub fn destination(&self) -> Destination<'_> {
        Destination {
            scheme: if self.scheme_is_https {
                "https"
            } else {
                "http"
            },
            host: &self.host,
            port: self.port,
        }
    }
}

/// The settings that decide how a subscription is delivered to, checked together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryLimits {
    rate_per_second: u32,
    timeout: Duration,
}

impl DeliveryLimits {
    pub const DEFAULT_RATE: u32 = 10;
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

    /// # Errors
    /// [`OutboundError::InvalidRate`], [`OutboundError::InvalidTimeout`].
    pub fn new(rate_per_second: u32, timeout: Duration) -> Result<Self, OutboundError> {
        if !(1..=100).contains(&rate_per_second) {
            return Err(OutboundError::InvalidRate);
        }
        if !(Duration::from_secs(1)..=Duration::from_secs(30)).contains(&timeout) {
            return Err(OutboundError::InvalidTimeout);
        }
        Ok(Self {
            rate_per_second,
            timeout,
        })
    }

    #[must_use]
    pub const fn rate_per_second(&self) -> u32 {
        self.rate_per_second
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Default for DeliveryLimits {
    fn default() -> Self {
        Self {
            rate_per_second: Self::DEFAULT_RATE,
            timeout: Self::DEFAULT_TIMEOUT,
        }
    }
}

/// Checks the kinds a subscription names: at least one, not too many, none twice.
///
/// # Errors
/// [`OutboundError::InvalidEventKinds`].
pub fn check_kinds(kinds: &[OutboundKind]) -> Result<(), OutboundError> {
    if kinds.is_empty() || kinds.len() > MAX_EVENT_KINDS {
        return Err(OutboundError::InvalidEventKinds);
    }
    for (i, kind) in kinds.iter().enumerate() {
        if kinds[..i].contains(kind) {
            return Err(OutboundError::InvalidEventKinds);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_2xx_is_delivered_and_a_server_error_or_a_request_for_patience_is_retried() {
        assert_eq!(
            judge_response(200, None),
            (AttemptOutcome::Delivered, Verdict::Done)
        );
        assert_eq!(judge_response(204, None).1, Verdict::Done);
        for status in [500, 502, 503, 504, 408, 425] {
            assert_eq!(
                judge_response(status, None),
                (AttemptOutcome::ServerError, Verdict::Retry { after: None }),
                "{status}"
            );
        }
        assert_eq!(
            judge_response(429, Some(Duration::from_secs(90))),
            (
                AttemptOutcome::Throttled,
                Verdict::Retry {
                    after: Some(Duration::from_secs(90))
                }
            )
        );
    }

    #[test]
    fn a_refusal_that_retrying_cannot_change_is_dead_and_so_is_a_redirect() {
        for status in [301, 302, 307, 400, 401, 403, 404, 410, 422] {
            assert_eq!(
                judge_response(status, None),
                (AttemptOutcome::Rejected, Verdict::Dead),
                "{status}"
            );
        }
    }

    #[test]
    fn what_the_receiver_asks_to_wait_is_capped() {
        let (_, verdict) = judge_response(503, Some(Duration::from_secs(30 * 86_400)));
        assert_eq!(
            verdict,
            Verdict::Retry {
                after: Some(MAX_RETRY_AFTER)
            }
        );
    }

    #[test]
    fn the_backoff_grows_along_the_schedule_with_jitter_and_honours_retry_after() {
        let at = |failed, jitter| delivery_backoff(failed, jitter, None);
        assert_eq!(at(1, 0.5), Duration::from_secs(5));
        assert_eq!(at(2, 0.5), Duration::from_secs(30));
        assert_eq!(at(4, 0.5), Duration::from_secs(600));
        assert_eq!(at(10, 0.5), Duration::from_secs(86_400));
        assert_eq!(
            at(11, 0.5),
            Duration::from_secs(86_400),
            "the schedule ends flat"
        );
        assert_eq!(
            at(400, 0.5),
            Duration::from_secs(86_400),
            "and never overflows"
        );
        // Jitter spreads each step by 20 % either way, and a sample out of range is clamped.
        assert_eq!(at(3, 0.0), Duration::from_secs(96));
        assert_eq!(at(3, 1.0), Duration::from_secs(144));
        assert_eq!(at(3, 9.0), Duration::from_secs(144));
        // A receiver that asks for longer than the schedule is obeyed; one that asks for less is not.
        assert_eq!(
            delivery_backoff(1, 0.5, Some(Duration::from_secs(120))),
            Duration::from_secs(120)
        );
        assert_eq!(
            delivery_backoff(4, 0.5, Some(Duration::from_secs(60))),
            Duration::from_secs(600)
        );
        assert_eq!(
            at(0, 0.5),
            Duration::from_secs(5),
            "zero failures is treated as the first"
        );
    }

    #[test]
    fn a_delivery_gets_a_fixed_number_of_attempts() {
        assert!(!out_of_attempts(MAX_DELIVERY_ATTEMPTS - 1));
        assert!(out_of_attempts(MAX_DELIVERY_ATTEMPTS));
    }

    #[test]
    fn a_target_is_an_http_url_with_a_host_and_nothing_that_hides_another() {
        let ok = TargetUrl::parse("https://hooks.example.com/in?x=1").expect("url");
        assert_eq!(ok.destination().host, "hooks.example.com");
        assert_eq!(ok.destination().port, 443);
        assert_eq!(ok.destination().scheme, "https");
        let plain = TargetUrl::parse("http://Hooks.Example.com:8080/").expect("url");
        assert_eq!(
            (plain.destination().host, plain.destination().port),
            ("hooks.example.com", 8080)
        );
        let v6 = TargetUrl::parse("https://[2001:db8::1]:8443/x").expect("url");
        assert_eq!(
            (v6.destination().host, v6.destination().port),
            ("2001:db8::1", 8443)
        );

        for bad in [
            "",
            "ftp://example.com/",
            "https://",
            "https:///path",
            "https://user:pass@example.com/",
            "https://allowed.example.com@evil.example.net/",
            "https://example.com/#frag",
            "https://example.com:99999/",
            "https://exa mple.com/",
            "https://example.com/\n",
            "https://ex\u{e4}mple.com/",
            "example.com/path",
        ] {
            assert_eq!(
                TargetUrl::parse(bad),
                Err(OutboundError::InvalidUrl),
                "{bad:?}"
            );
        }
        assert_eq!(
            TargetUrl::parse(&format!("https://example.com/{}", "a".repeat(MAX_URL_LEN))),
            Err(OutboundError::InvalidUrl)
        );
    }

    #[test]
    fn rates_timeouts_and_kinds_are_bounded() {
        assert!(DeliveryLimits::new(1, Duration::from_secs(1)).is_ok());
        assert!(DeliveryLimits::new(100, Duration::from_secs(30)).is_ok());
        assert_eq!(
            DeliveryLimits::new(0, Duration::from_secs(5)),
            Err(OutboundError::InvalidRate)
        );
        assert_eq!(
            DeliveryLimits::new(101, Duration::from_secs(5)),
            Err(OutboundError::InvalidRate)
        );
        assert_eq!(
            DeliveryLimits::new(5, Duration::from_millis(999)),
            Err(OutboundError::InvalidTimeout)
        );
        assert_eq!(
            DeliveryLimits::new(5, Duration::from_secs(31)),
            Err(OutboundError::InvalidTimeout)
        );
        assert_eq!(check_kinds(&[]), Err(OutboundError::InvalidEventKinds));
        assert_eq!(
            check_kinds(&[OutboundKind::RunFinished, OutboundKind::RunFinished]),
            Err(OutboundError::InvalidEventKinds)
        );
        assert!(check_kinds(&[OutboundKind::RunFinished]).is_ok());
        assert_eq!("run.finished".parse(), Ok(OutboundKind::RunFinished));
        assert!("run.started".parse::<OutboundKind>().is_err());
    }
}
