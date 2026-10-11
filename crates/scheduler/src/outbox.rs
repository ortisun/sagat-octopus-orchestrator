//! The outbox dispatcher: moves committed messages from PostgreSQL to the broker.
//!
//! PostgreSQL is the authority and the broker only carries hints, so this is deliberately simple
//! and at-least-once: a message is claimed (hidden for `visibility`), published with a stable
//! message id, and only then marked delivered. A crash between publishing and confirming makes the
//! message reappear and be published again, never be lost; the stable id lets the broker drop the
//! duplicate. A failure is recorded with a stable code and retried with capped, jittered backoff,
//! so a broker outage grows the backlog (a gauge) instead of losing messages or spinning.

use std::time::Duration;

use sagat_application::entropy::Entropy;
use sagat_application::error::StoreError;
use sagat_application::maintenance::{
    EventPublisher, OutboxMessage, OutboxStore, PublishError, Window,
};
use tracing::Instrument;
use uuid::Uuid;

use crate::fairness;
use crate::shards::Placement;

/// Tunable limits of the dispatcher.
#[derive(Debug, Clone, Copy)]
pub struct DispatchSettings {
    /// Messages claimed per round.
    pub batch: u32,
    /// How long a claimed message stays hidden from other dispatchers; it must exceed the time a
    /// round takes, or two dispatchers would publish the same message.
    pub visibility: Duration,
    /// Delay after the first failure; doubles with every further attempt.
    pub base_backoff: Duration,
    /// Longest delay between two attempts of one message.
    pub max_backoff: Duration,
    /// How long a dispatcher with a placement rests its search for other tenants after one that found
    /// nobody (see `SchedulerSettings::discovery_rest`); zero: never.
    pub discovery_rest: Duration,
}

impl Default for DispatchSettings {
    fn default() -> Self {
        Self {
            batch: 100,
            visibility: Duration::from_secs(30),
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            discovery_rest: Duration::from_millis(100),
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dispatched {
    pub published: u32,
    pub failed: u32,
}

impl Dispatched {
    /// Whether the round claimed anything (a busy outbox should be drained without pausing).
    #[must_use]
    pub const fn busy(&self) -> bool {
        self.published + self.failed > 0
    }
}

/// Publishes outbox messages.
pub struct OutboxDispatcher<S, P, R> {
    store: S,
    publisher: P,
    entropy: R,
    settings: DispatchSettings,
    /// Which shards this replica dispatches for; without it the dispatcher takes the oldest due
    /// messages of any shard.
    placement: Option<Placement>,
    /// Until when the search for other tenants is rested.
    rest_until: std::sync::Mutex<Option<std::time::Instant>>,
}

impl<S, P, R> std::fmt::Debug for OutboxDispatcher<S, P, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboxDispatcher").finish_non_exhaustive()
    }
}

impl<S, P, R> OutboxDispatcher<S, P, R>
where
    S: OutboxStore,
    P: EventPublisher,
    R: Entropy,
{
    pub const fn new(store: S, publisher: P, entropy: R, settings: DispatchSettings) -> Self {
        Self {
            store,
            publisher,
            entropy,
            settings,
            placement: None,
            rest_until: std::sync::Mutex::new(None),
        }
    }

    /// Makes the dispatcher one of several replicas: it dispatches the messages of the shards the
    /// placement says it holds, taking them fairly between tenants (`FR-010-001`, `FR-010-002`). The
    /// messages of one run are in one shard, so one dispatcher publishes them, in the order they
    /// were written. A message of a shard whose owner is gone is published by its next owner: the
    /// visibility timeout of a claim that was never confirmed has the same effect as before.
    #[must_use]
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }

    /// The next messages to publish: the oldest of any shard, or, for a replica with a placement,
    /// a fair choice among the due messages of its shards.
    async fn claim(&self) -> Result<Vec<OutboxMessage>, StoreError> {
        let Some(placement) = &self.placement else {
            return self
                .store
                .claim_outbox(self.settings.batch, self.settings.visibility)
                .await;
        };
        let snapshot = placement.snapshot();
        let shards = snapshot.shards(std::time::Instant::now());
        if shards.is_empty() {
            return Ok(Vec::new());
        }
        let limit = self.settings.batch;
        let store = &self.store;
        let discover = self
            .rest_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none_or(|until| until <= std::time::Instant::now());
        let (chosen, selection) = fairness::select(
            limit,
            discover,
            snapshot.weights(),
            |due: &sagat_application::maintenance::OutboxDue| due.tenant_id,
            |due| due.id,
            |exclude| async move {
                store
                    .due_outbox(&Window {
                        shards,
                        exclude,
                        limit,
                    })
                    .await
            },
        )
        .await?;
        let rest = fairness::rest_after(self.settings.discovery_rest, &selection);
        if discover && !rest.is_zero() {
            *self
                .rest_until
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(std::time::Instant::now() + rest);
        }
        if selection.limited > 0 {
            metrics::counter!("scheduler_fairness_limited_total", "pass" => "outbox")
                .increment(selection.limited as u64);
        }
        if selection.exhausted {
            metrics::counter!("scheduler_fairness_exhausted_total", "pass" => "outbox")
                .increment(1);
        }
        if chosen.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<Uuid> = chosen.iter().map(|due| due.id).collect();
        self.store
            .claim_outbox_ids(&ids, self.settings.visibility)
            .await
    }

    /// Claims one batch and publishes it. Never fails: what cannot be published is recorded and
    /// retried, and a store error is logged and counted as a round that did nothing.
    pub async fn run_once(&self) -> Dispatched {
        let claimed = match self.claim().await {
            Ok(claimed) => claimed,
            Err(error) => {
                tracing::warn!(%error, "could not claim outbox messages");
                return Dispatched::default();
            }
        };
        let mut delivered: Vec<Uuid> = Vec::with_capacity(claimed.len());
        let mut outcome = Dispatched::default();
        for message in &claimed {
            let published = async {
                match subject_of(message) {
                    Ok(subject) => {
                        self.publisher
                            .publish(&subject, &message.id.to_string(), &message.payload)
                            .await
                    }
                    Err(rejected) => Err(rejected),
                }
            }
            .instrument(publish_span(message))
            .await;
            match published {
                Ok(()) => {
                    delivered.push(message.id);
                    outcome.published += 1;
                    metrics::counter!("outbox_published_total", "event" => known(&message.event_type))
                        .increment(1);
                }
                Err(error) => {
                    outcome.failed += 1;
                    metrics::counter!("outbox_publish_failed_total", "code" => error.code())
                        .increment(1);
                    tracing::warn!(
                        code = error.code(),
                        attempt = message.attempt_count,
                        event = %message.event_type,
                        "could not publish an outbox message; it will be retried"
                    );
                    let retry_in = self.backoff(message.attempt_count);
                    if let Err(store) = self
                        .store
                        .mark_failed(message.id, error.code(), retry_in)
                        .await
                    {
                        // The visibility timeout makes it reappear anyway.
                        tracing::warn!(%store, "could not record a failed publish");
                    }
                }
            }
        }
        if !delivered.is_empty()
            && let Err(error) = self.store.mark_delivered(&delivered).await
        {
            // The messages reappear after the visibility timeout and are published again with the
            // same ids: a duplicate the broker drops, never a loss.
            tracing::warn!(%error, count = delivered.len(), "could not confirm delivered messages");
        }
        // The backlog is a gauge of the whole outbox, set by the monitor (one replica) and not counted
        // by every dispatcher at every round: that cost grew with the replicas and the backlog.
        outcome
    }

    /// Capped exponential backoff with equal jitter: half fixed, half random.
    fn backoff(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1).min(16);
        let uncapped = self
            .settings
            .base_backoff
            .saturating_mul(1u32 << exponent)
            .min(self.settings.max_backoff);
        let half = uncapped / 2;
        half + half.mul_f64(self.entropy.unit().clamp(0.0, 1.0))
    }
}

/// The span of publishing one message, in the trace of the run it is about when the message says
/// which (a `task.ready` hint does: it carries the run and its trace).
fn publish_span(message: &OutboxMessage) -> tracing::Span {
    let fields: Option<serde_json::Value> = serde_json::from_slice(&message.payload).ok();
    let text = |name: &str| {
        fields
            .as_ref()
            .and_then(|f| f.get(name))
            .and_then(serde_json::Value::as_str)
    };
    let parent = text("runId")
        .and_then(|run| Uuid::parse_str(run).ok())
        .map(|run| sagat_telemetry::run_span_id(run).to_string());
    tracing::info_span!(
        "outbox.publish",
        event = known(&message.event_type),
        trace_id = text("traceId"),
        parent_span_id = parent,
        otel.kind = "producer",
        attempt = message.attempt_count,
    )
}

/// The metric label of an event type: a closed set, never caller-controlled text.
fn known(event_type: &str) -> &'static str {
    match event_type {
        "task.ready" => "task.ready",
        "run.finished" => "run.finished",
        _ => "other",
    }
}

/// `sagat.v1.<event type>.<tenant>[.<routing key>]`: bounded by tenants, event types and the
/// activities a tenant declares, never by runs. Every token is checked, because a subject is a
/// wildcard-bearing pattern language and a message must never be able to address a subject outside
/// its own tenant; the routing key (an activity name) is mapped onto the characters a subject token
/// may safely use.
fn subject_of(message: &OutboxMessage) -> Result<String, PublishError> {
    let valid =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
    if message.event_type.is_empty()
        || message.event_type.starts_with('.')
        || message.event_type.ends_with('.')
        || message.event_type.contains("..")
        || !message.event_type.chars().all(valid)
    {
        return Err(PublishError::Rejected(
            "the event type is not a valid subject",
        ));
    }
    let mut subject = format!("sagat.v1.{}.{}", message.event_type, message.tenant_id);
    if let Some(key) = &message.routing_key {
        subject.push('.');
        subject.push_str(&routing_tokens(key)?);
    }
    Ok(subject)
}

/// Dot-separated tokens of `[A-Za-z0-9_-]`; anything else, and an empty token, becomes `_`. Two keys
/// that differ only in such characters share a subject, which is harmless for a hint (the payload
/// names the exact activity).
fn routing_tokens(key: &str) -> Result<String, PublishError> {
    if key.is_empty() || key.len() > MAX_ROUTING_KEY {
        return Err(PublishError::Rejected(
            "the routing key is not usable in a subject",
        ));
    }
    let safe = |c: char| {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
            c
        } else {
            '_'
        }
    };
    Ok(key
        .split('.')
        .map(|token| {
            if token.is_empty() {
                "_".to_owned()
            } else {
                token.chars().map(safe).collect()
            }
        })
        .collect::<Vec<_>>()
        .join("."))
}

/// A NATS subject may be long, but a hint's routing key is an activity name (at most this).
const MAX_ROUTING_KEY: usize = 128;

#[cfg(test)]
mod tests {
    use sagat_domain::identity::TenantId;
    use time::OffsetDateTime;

    use super::*;

    fn message(event_type: &str) -> OutboxMessage {
        OutboxMessage {
            id: Uuid::nil(),
            tenant_id: TenantId::generate(),
            aggregate_id: Uuid::nil(),
            event_type: event_type.to_owned(),
            routing_key: None,
            payload: Vec::new(),
            attempt_count: 1,
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    struct Fixed(f64);

    impl Entropy for Fixed {
        fn unit(&self) -> f64 {
            self.0
        }

        fn token(&self) -> [u8; 32] {
            [1; 32]
        }
    }

    /// A store that is never reached: only the backoff arithmetic is under test.
    struct NoStore;

    impl OutboxStore for NoStore {
        async fn due_outbox(
            &self,
            _: &Window,
        ) -> Result<
            Vec<sagat_application::maintenance::OutboxDue>,
            sagat_application::error::StoreError,
        > {
            Ok(Vec::new())
        }

        async fn claim_outbox_ids(
            &self,
            _: &[Uuid],
            _: Duration,
        ) -> Result<Vec<OutboxMessage>, sagat_application::error::StoreError> {
            Ok(Vec::new())
        }

        async fn claim_outbox(
            &self,
            _: u32,
            _: Duration,
        ) -> Result<Vec<OutboxMessage>, sagat_application::error::StoreError> {
            Ok(Vec::new())
        }

        async fn mark_delivered(
            &self,
            _: &[Uuid],
        ) -> Result<(), sagat_application::error::StoreError> {
            Ok(())
        }

        async fn mark_failed(
            &self,
            _: Uuid,
            _: &str,
            _: Duration,
        ) -> Result<(), sagat_application::error::StoreError> {
            Ok(())
        }

        async fn outbox_backlog(&self) -> Result<u64, sagat_application::error::StoreError> {
            Ok(0)
        }
    }

    struct NoBroker;

    impl EventPublisher for NoBroker {
        async fn publish(&self, _: &str, _: &str, _: &[u8]) -> Result<(), PublishError> {
            Ok(())
        }
    }

    #[test]
    fn the_backoff_doubles_with_every_attempt_up_to_its_cap_and_is_jittered_within_half() {
        let at = |unit: f64, attempt: u32| {
            OutboxDispatcher::new(NoStore, NoBroker, Fixed(unit), DispatchSettings::default())
                .backoff(attempt)
        };
        // Attempts count from 1; the delay is [half, full] of base * 2^(attempt-1), capped.
        assert_eq!(at(0.0, 1), Duration::from_millis(500));
        assert_eq!(at(1.0, 1), Duration::from_secs(1));
        assert_eq!(at(0.0, 2), Duration::from_secs(1));
        assert_eq!(at(1.0, 4), Duration::from_secs(8));
        assert_eq!(at(1.0, 6), Duration::from_secs(32));
        assert_eq!(
            at(0.0, 7),
            Duration::from_secs(30),
            "half of the capped 60 s"
        );
        assert_eq!(at(1.0, 7), Duration::from_secs(60), "capped");
        assert_eq!(at(1.0, u32::MAX), Duration::from_secs(60), "no overflow");
        assert_eq!(
            at(0.0, 0),
            Duration::from_millis(500),
            "attempt 0 is treated as the first"
        );
    }

    #[test]
    fn the_subject_is_per_event_type_and_tenant() {
        let m = message("task.ready");
        assert_eq!(
            subject_of(&m).expect("valid"),
            format!("sagat.v1.task.ready.{}", m.tenant_id)
        );
    }

    #[test]
    fn a_routing_key_narrows_the_subject_and_can_never_widen_or_split_it() {
        let m = message("task.ready");
        let with_key = |key: &str| {
            let mut keyed = m.clone();
            keyed.routing_key = Some(key.to_owned());
            subject_of(&keyed)
        };
        let base = format!("sagat.v1.task.ready.{}", m.tenant_id);
        let suffix = |key: &str| {
            with_key(key)
                .expect("usable")
                .strip_prefix(&format!("{base}."))
                .expect("under the tenant subject")
                .to_owned()
        };
        assert_eq!(suffix("payments.charge"), "payments.charge");
        assert_eq!(suffix("Payments:Charge"), "Payments_Charge");
        // Wildcards, separators and odd characters can only become underscores, never new tokens
        // beyond the dots the name has.
        assert_eq!(suffix("a*b"), "a_b");
        assert_eq!(suffix("a>"), "a_");
        assert_eq!(suffix("a b/c\nd"), "a_b_c_d");
        assert_eq!(suffix("a..b"), "a._.b");
        assert_eq!(suffix("a."), "a._");
        assert_eq!(suffix("ü"), "_");
        assert!(with_key("").is_err());
        assert!(with_key(&"a".repeat(129)).is_err());
        // However hostile the key, the subject stays under the tenant's.
        for hostile in ["*", ">", "..", "x.>", "$JS.API", "_INBOX.x"] {
            let subject = with_key(hostile).expect("sanitised");
            assert!(
                subject.starts_with(&format!("{base}.")),
                "{hostile}: {subject}"
            );
            assert!(
                !subject.contains(['*', '>', ' ', '$']),
                "{hostile}: {subject}"
            );
        }
    }

    #[test]
    fn an_event_type_that_could_escape_its_subject_is_rejected() {
        for bad in [
            "", ".x", "x.", "a..b", "a b", "a*", "a>", "A", "a/b", "a\nb", "ü",
        ] {
            assert!(subject_of(&message(bad)).is_err(), "{bad:?}");
        }
    }
}
