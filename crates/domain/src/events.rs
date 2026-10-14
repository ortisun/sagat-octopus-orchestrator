//! Events that arrive from outside and what is kept about them (`FR-004-002`, `FR-004-003`,
//! `FR-004-008`).
//!
//! An event is admitted once per tenant, source and event id for as long as the endpoint's
//! deduplication window lasts. What it wakes or starts is decided afterwards, from the inbox, so
//! admitting it never waits for that work (the ingress promise) and a crash cannot lose it.

use std::fmt;
use std::time::Duration;

use crate::execution::CorrelationKey;
use crate::identity::Slug;
use crate::workflow::EventType;

uuid_id!(
    /// Identifier of an event endpoint: what the provider's URL carries.
    EndpointId
);

impl EndpointId {
    /// A random identifier. The id is part of a URL anybody holding it can post to, so unlike the
    /// other identifiers it is not time-ordered: nothing about when the endpoint was made leaks.
    #[must_use]
    pub fn random() -> Self {
        Self::new(::uuid::Uuid::new_v4())
    }
}

/// Longest event id accepted.
pub const MAX_EVENT_ID_LEN: usize = 200;
/// Longest subject accepted.
pub const MAX_SUBJECT_LEN: usize = 256;
/// Shortest and longest time an event is remembered for deduplication (and buffered for waits that
/// look back).
pub const MIN_DEDUPE_TTL: Duration = Duration::from_secs(60);
pub const MAX_DEDUPE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// What an endpoint gets when it says nothing.
pub const DEFAULT_DEDUPE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How far a request's timestamp may be from the clock of the platform, by default and at most.
pub const DEFAULT_SKEW: Duration = Duration::from_secs(300);
pub const MAX_SKEW: Duration = Duration::from_secs(3600);
/// What a deduplication window must exceed twice the skew by.
pub const REPLAY_MARGIN: Duration = Duration::from_secs(60);
/// Largest body an endpoint accepts, by default and at most: the platform's payload limit (256 KiB)
/// less room for the envelope the data travels in once it becomes a run's input.
pub const DEFAULT_MAX_BODY: usize = 240 * 1024;
/// Requests per second one endpoint takes, by default and at most.
pub const DEFAULT_RATE: u32 = 50;
pub const MAX_RATE: u32 = 10_000;

/// Why a value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    #[error("an event id is 1 to {MAX_EVENT_ID_LEN} printable characters without spaces")]
    InvalidEventId,
    #[error("a subject is at most {MAX_SUBJECT_LEN} characters without control characters")]
    InvalidSubject,
    #[error("the deduplication window is between 1 minute and 30 days")]
    InvalidDedupeTtl,
    #[error("the allowed clock skew is between 1 second and 1 hour")]
    InvalidSkew,
    #[error(
        "the deduplication window must be at least twice the allowed clock skew and a minute more, \
         or a replay of a signed request could be admitted as a new event"
    )]
    DedupeShorterThanReplayWindow,
    #[error("the body limit is between 1 KiB and 240 KiB")]
    InvalidBodyLimit,
    #[error("the rate is between 1 and {MAX_RATE} requests per second")]
    InvalidRate,
}

/// The identity a provider (or a connector) gives an event: unique within its source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventId(String);

impl EventId {
    /// # Errors
    /// [`EventError::InvalidEventId`].
    pub fn parse(value: &str) -> Result<Self, EventError> {
        if value.is_empty()
            || value.len() > MAX_EVENT_ID_LEN
            || !value.chars().all(|c| c.is_ascii_graphic())
        {
            Err(EventError::InvalidEventId)
        } else {
            Ok(Self(value.to_owned()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What identifies an event in the inbox, apart from the tenant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventKey {
    pub source: Slug,
    pub event_id: EventId,
}

/// The envelope of an event: everything but its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnvelope {
    pub key: EventKey,
    pub event_type: EventType,
    pub subject: Option<String>,
    pub correlation_key: Option<CorrelationKey>,
}

impl EventEnvelope {
    /// # Errors
    /// [`EventError::InvalidSubject`].
    pub fn new(
        key: EventKey,
        event_type: EventType,
        subject: Option<&str>,
        correlation_key: Option<CorrelationKey>,
    ) -> Result<Self, EventError> {
        if subject.is_some_and(|s| s.len() > MAX_SUBJECT_LEN || s.chars().any(char::is_control)) {
            return Err(EventError::InvalidSubject);
        }
        Ok(Self {
            key,
            event_type,
            subject: subject.map(str::to_owned),
            correlation_key,
        })
    }
}

/// A deduplication window for an event that is not signed (one published through the API): it only
/// has to be inside the bounds.
///
/// # Errors
/// [`EventError::InvalidDedupeTtl`].
pub fn checked_dedupe_ttl(ttl: Duration) -> Result<Duration, EventError> {
    bounded(
        ttl,
        MIN_DEDUPE_TTL,
        MAX_DEDUPE_TTL,
        EventError::InvalidDedupeTtl,
    )
}

/// A duration that must stay inside bounds.
fn bounded(
    value: Duration,
    min: Duration,
    max: Duration,
    error: EventError,
) -> Result<Duration, EventError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(error)
    }
}

/// The settings that make an endpoint safe to expose, checked together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointLimits {
    dedupe_ttl: Duration,
    skew: Duration,
    max_body: usize,
    rate_per_second: u32,
}

impl EndpointLimits {
    /// # Errors
    /// The [`EventError`] of the first setting out of bounds.
    pub fn new(
        dedupe_ttl: Duration,
        skew: Duration,
        max_body: usize,
        rate_per_second: u32,
    ) -> Result<Self, EventError> {
        let dedupe_ttl = bounded(
            dedupe_ttl,
            MIN_DEDUPE_TTL,
            MAX_DEDUPE_TTL,
            EventError::InvalidDedupeTtl,
        )?;
        let skew = bounded(
            skew,
            Duration::from_secs(1),
            MAX_SKEW,
            EventError::InvalidSkew,
        )?;
        // A signed request is accepted for `skew` after its timestamp, and its timestamp may be up
        // to `skew` ahead of the clock: a copy of it can be replayed for up to twice the skew after
        // the event was admitted. The event must be remembered at least that long, or the replay
        // would be admitted as a new event.
        // The margin is for the replicas whose clocks are not the database's, which stamps the
        // window: a replay checked on one replica and an admission on another could otherwise
        // differ by their offset.
        if dedupe_ttl < skew * 2 + REPLAY_MARGIN {
            return Err(EventError::DedupeShorterThanReplayWindow);
        }
        Ok(Self {
            dedupe_ttl,
            skew,
            max_body: if (1024..=DEFAULT_MAX_BODY).contains(&max_body) {
                max_body
            } else {
                return Err(EventError::InvalidBodyLimit);
            },
            rate_per_second: if (1..=MAX_RATE).contains(&rate_per_second) {
                rate_per_second
            } else {
                return Err(EventError::InvalidRate);
            },
        })
    }

    #[must_use]
    pub const fn dedupe_ttl(&self) -> Duration {
        self.dedupe_ttl
    }

    #[must_use]
    pub const fn skew(&self) -> Duration {
        self.skew
    }

    #[must_use]
    pub const fn max_body(&self) -> usize {
        self.max_body
    }

    #[must_use]
    pub const fn rate_per_second(&self) -> u32 {
        self.rate_per_second
    }
}

impl Default for EndpointLimits {
    fn default() -> Self {
        Self {
            dedupe_ttl: DEFAULT_DEDUPE_TTL,
            skew: DEFAULT_SKEW,
            max_body: DEFAULT_MAX_BODY,
            rate_per_second: DEFAULT_RATE,
        }
    }
}

/// Whether an endpoint takes requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointStatus {
    Active,
    /// Refuses requests (`503`) until resumed; nothing already admitted is affected.
    Paused,
}

impl EndpointStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
        }
    }
}

impl std::str::FromStr for EndpointStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "PAUSED" => Ok(Self::Paused),
            _ => Err(()),
        }
    }
}

/// Where the inbox stands for one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxStatus {
    /// Admitted; what it wakes or starts has not been decided yet (or is being retried).
    Pending,
    /// Everything it was due to wake or start has been.
    Dispatched,
    /// Dispatching it failed again and again; it is kept for inspection until it expires.
    Dead,
}

impl InboxStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Dispatched => "DISPATCHED",
            Self::Dead => "DEAD",
        }
    }
}

impl std::str::FromStr for InboxStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        match value {
            "PENDING" => Ok(Self::Pending),
            "DISPATCHED" => Ok(Self::Dispatched),
            "DEAD" => Ok(Self::Dead),
            _ => Err(()),
        }
    }
}

/// How long to wait before dispatching an event again after its `attempts`-th failure: exponential
/// from one second to ten minutes, with a jitter sample in `[0, 1)` of up to a quarter of it.
#[must_use]
pub fn dispatch_backoff(attempts: u32, jitter: f64) -> Duration {
    let base = Duration::from_secs(1u64 << attempts.min(9)).min(Duration::from_secs(600));
    base + base.mul_f64(0.25 * jitter.clamp(0.0, 1.0))
}

/// How many times an event is dispatched before it is given up on.
pub const MAX_DISPATCH_ATTEMPTS: u32 = 12;

/// How long an event may wait for room (a quota of the tenant, a database that is down) before it is
/// set aside. Waiting for room is not a failure of the event, so it does not use up its attempts;
/// it is bounded by time instead, and the sender is told to slow down meanwhile (the backlog).
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(24 * 3600);

/// When to look again at an event that is waiting for room: about half a minute, spread by up to
/// half of it with a jitter sample in `[0, 1)` so that the events a limit held back do not all come
/// back in the same second.
#[must_use]
pub fn capacity_backoff(jitter: f64) -> Duration {
    let base = Duration::from_secs(30);
    base + base.mul_f64(0.5 * jitter.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_id_is_graphic_ascii_without_spaces() {
        assert!(EventId::parse("evt_123:abc/def=+").is_ok());
        for bad in ["", "has space", "tab\t", "caf\u{e9}", &"x".repeat(201)] {
            assert_eq!(
                EventId::parse(bad),
                Err(EventError::InvalidEventId),
                "{bad:?}"
            );
        }
        assert!(EventId::parse(&"x".repeat(200)).is_ok());
    }

    #[test]
    fn the_limits_of_an_endpoint_are_checked_together() {
        let ok = EndpointLimits::new(
            Duration::from_secs(3600),
            Duration::from_secs(300),
            4096,
            10,
        );
        assert!(ok.is_ok());
        let ttl = |s| EndpointLimits::new(Duration::from_secs(s), DEFAULT_SKEW, 4096, 10);
        assert_eq!(ttl(59), Err(EventError::InvalidDedupeTtl));
        assert_eq!(ttl(30 * 86_400 + 1), Err(EventError::InvalidDedupeTtl));
        assert_eq!(
            EndpointLimits::new(DEFAULT_DEDUPE_TTL, Duration::ZERO, 4096, 10),
            Err(EventError::InvalidSkew)
        );
        // The window a replay could be admitted in is twice the skew: the event is remembered at
        // least that long, and not one second less.
        let window = |ttl, skew| {
            EndpointLimits::new(
                Duration::from_secs(ttl),
                Duration::from_secs(skew),
                4096,
                10,
            )
        };
        // ...plus a margin for the clocks of the replicas, which are not the database's.
        let margin = REPLAY_MARGIN.as_secs();
        assert!(window(600 + margin, 300).is_ok());
        assert_eq!(
            window(600 + margin - 1, 300),
            Err(EventError::DedupeShorterThanReplayWindow)
        );
        assert_eq!(
            window(600, 300),
            Err(EventError::DedupeShorterThanReplayWindow),
            "exactly twice the skew leaves no room for a clock that is a little off"
        );
        assert_eq!(
            window(7200 + margin, 3600).map(|l| l.skew()),
            Ok(Duration::from_secs(3600))
        );
        assert_eq!(
            window(7200 + margin - 1, 3600),
            Err(EventError::DedupeShorterThanReplayWindow)
        );
        assert_eq!(
            EndpointLimits::new(DEFAULT_DEDUPE_TTL, DEFAULT_SKEW, 100, 10),
            Err(EventError::InvalidBodyLimit)
        );
        assert_eq!(
            EndpointLimits::new(DEFAULT_DEDUPE_TTL, DEFAULT_SKEW, 4096, 0),
            Err(EventError::InvalidRate)
        );
    }

    #[test]
    fn a_window_for_events_nobody_signs_only_has_to_be_a_window() {
        // An event published through the API carries no signature, so there is no skew to outlast.
        assert_eq!(
            checked_dedupe_ttl(Duration::from_secs(60)),
            Ok(Duration::from_secs(60))
        );
        assert_eq!(checked_dedupe_ttl(MAX_DEDUPE_TTL), Ok(MAX_DEDUPE_TTL));
        for bad in [
            Duration::from_secs(59),
            MAX_DEDUPE_TTL + Duration::from_secs(1),
        ] {
            assert_eq!(checked_dedupe_ttl(bad), Err(EventError::InvalidDedupeTtl));
        }
    }

    #[test]
    fn dispatch_backs_off_exponentially_to_a_ceiling_with_jitter() {
        assert_eq!(dispatch_backoff(0, 0.0), Duration::from_secs(1));
        assert_eq!(dispatch_backoff(3, 0.0), Duration::from_secs(8));
        assert_eq!(dispatch_backoff(9, 0.0), Duration::from_secs(512));
        assert_eq!(
            dispatch_backoff(40, 0.0),
            Duration::from_secs(512),
            "the exponent is capped"
        );
        assert_eq!(dispatch_backoff(3, 1.0), Duration::from_secs(10));
        assert!(
            dispatch_backoff(3, 7.0) <= Duration::from_secs(10),
            "jitter is clamped"
        );
    }

    #[test]
    fn a_subject_has_no_control_characters() {
        let key = EventKey {
            source: Slug::parse("billing").expect("slug"),
            event_id: EventId::parse("e-1").expect("id"),
        };
        let kind = EventType::parse("invoice.paid").expect("type");
        assert!(EventEnvelope::new(key.clone(), kind.clone(), Some("invoices/7"), None).is_ok());
        assert_eq!(
            EventEnvelope::new(key, kind, Some("bad\nsubject"), None),
            Err(EventError::InvalidSubject)
        );
    }
}

/// The one credential mechanism accepted by an inbound webhook endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WebhookAuthentication {
    #[default]
    Hmac,
    Oidc {
        profile: Slug,
        subject: String,
    },
    Mtls {
        certificate_sha256: String,
    },
}

impl WebhookAuthentication {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Hmac => "HMAC",
            Self::Oidc { .. } => "OIDC",
            Self::Mtls { .. } => "MTLS",
        }
    }

    /// Validate the non-secret identity bound to the endpoint.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Hmac => true,
            Self::Oidc { subject, .. } => {
                !subject.is_empty()
                    && subject.len() <= 255
                    && !subject.chars().any(char::is_control)
            }
            Self::Mtls { certificate_sha256 } => {
                certificate_sha256.len() == 64
                    && certificate_sha256
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            }
        }
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn identity_profiles_are_exact_and_bounded() {
        let profile = Slug::parse("provider").expect("slug");
        for subject in ["".to_owned(), "x".repeat(256), "a\nb".to_owned()] {
            assert!(
                !WebhookAuthentication::Oidc {
                    profile: profile.clone(),
                    subject
                }
                .is_valid()
            );
        }
        assert!(
            WebhookAuthentication::Oidc {
                profile,
                subject: "service/account".to_owned()
            }
            .is_valid()
        );
        assert!(
            WebhookAuthentication::Mtls {
                certificate_sha256: "ab".repeat(32)
            }
            .is_valid()
        );
        assert!(
            !WebhookAuthentication::Mtls {
                certificate_sha256: "AB".repeat(32)
            }
            .is_valid()
        );
        assert_eq!(WebhookAuthentication::default().kind(), "HMAC");
    }
}
