//! The delivery worker: sends what subscriptions are owed, and decides what each answer means
//! (`FR-004-006`, US04).
//!
//! It is a loop over the deliveries that are due, safe to run in several replicas: a claim keeps a
//! delivery for a short lease, and the rate of each subscription is enforced by the claim itself.
//! Delivery is at-least-once. A worker that dies after the receiver got the event and before the
//! attempt was recorded sends it again when the lease runs out, with the same delivery id, which is
//! what lets a receiver drop the repeat; nothing here can promise more than that, and nothing says
//! otherwise.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use sagat_application::clock::{Clock, SystemClock};
use sagat_application::entropy::Entropy;
use sagat_application::outbound::{
    AttemptRecord, BrokerFailure, BrokerPublisher, BrokerRequest, DeliveryStore, DueDelivery, Next,
    NoBrokerPublisher, OutboundRequest, SendFailure, WebhookSender,
};
use sagat_application::signing;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::outbound::{
    AttemptOutcome, OutboundDestination, Verdict, delivery_backoff, judge_response, out_of_attempts,
};
use sagat_domain::secrets::{SecretCipher, SecretName, SecretValue};
use sagat_telemetry::{SpanId, TraceId};
use tracing::Instrument;

/// The version of the shape of what is delivered (see `events.asyncapi.yaml`).
const SCHEMA_VERSION: &str = "1";

/// Versions of the signing secret a request is signed with: the current one and the one before, so
/// a receiver that has not yet moved to a rotated secret still verifies.
const SIGNING_VERSIONS: u32 = 2;

/// Tunable limits of the worker.
#[derive(Debug, Clone, Copy)]
pub struct DeliverySettings {
    /// Subscriptions served per claim.
    pub batch: u32,
    /// How long a claimed delivery stays claimed; longer than the longest timeout of a request.
    pub lease: Duration,
    /// Delivered and dead deliveries are kept this long.
    pub retention: Duration,
    /// Deliveries removed per housekeeping round.
    pub purge_batch: u32,
}

impl Default for DeliverySettings {
    fn default() -> Self {
        Self {
            batch: 32,
            lease: Duration::from_secs(90),
            retention: Duration::from_secs(14 * 24 * 3600),
            purge_batch: 5_000,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeliveryRound {
    pub attempted: u32,
    pub delivered: u32,
    pub retried: u32,
    pub dead: u32,
    /// Subscriptions switched off by what this round found.
    pub switched_off: u32,
    /// Attempts that could not be recorded (they are repeated when the lease runs out).
    pub failed: u32,
}

impl DeliveryRound {
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.attempted + self.failed > 0
    }
}

/// The delivery worker over a store, a sender, a source of jitter and the key ring.
pub struct DeliveryWorker<S, T, R, P = NoBrokerPublisher> {
    store: S,
    sender: T,
    broker: P,
    entropy: R,
    cipher: sagat_application::secrets::SecretReader,
    clock: SystemClock,
    settings: DeliverySettings,
}

impl<S, T, R, P> std::fmt::Debug for DeliveryWorker<S, T, R, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryWorker").finish_non_exhaustive()
    }
}

impl<S, T, R> DeliveryWorker<S, T, R, NoBrokerPublisher>
where
    S: DeliveryStore,
    T: WebhookSender,
    R: Entropy,
{
    pub fn new(
        store: S,
        sender: T,
        entropy: R,
        cipher: Arc<dyn SecretCipher>,
        settings: DeliverySettings,
    ) -> Self {
        Self {
            store,
            sender,
            broker: NoBrokerPublisher,
            entropy,
            cipher: sagat_application::secrets::SecretReader::local(cipher),
            clock: SystemClock,
            settings,
        }
    }
}

impl<S, T, R, P> DeliveryWorker<S, T, R, P>
where
    S: DeliveryStore,
    T: WebhookSender,
    R: Entropy,
    P: BrokerPublisher,
{
    pub fn with_broker(
        store: S,
        sender: T,
        broker: P,
        entropy: R,
        cipher: Arc<dyn SecretCipher>,
        settings: DeliverySettings,
    ) -> Self {
        Self::with_broker_reader(
            store,
            sender,
            broker,
            entropy,
            sagat_application::secrets::SecretReader::local(cipher),
            settings,
        )
    }

    pub fn with_broker_reader(
        store: S,
        sender: T,
        broker: P,
        entropy: R,
        reader: sagat_application::secrets::SecretReader,
        settings: DeliverySettings,
    ) -> Self {
        Self {
            store,
            sender,
            broker,
            entropy,
            cipher: reader,
            clock: SystemClock,
            settings,
        }
    }

    pub fn with_secret_provider(
        mut self,
        provider: Arc<dyn sagat_application::secret_provider::SecretProvider>,
    ) -> Self {
        self.cipher = self.cipher.with_provider(provider);
        self
    }

    /// One round: claims what is due and attempts it, concurrently.
    pub async fn tick(&self) -> DeliveryRound {
        let mut round = DeliveryRound::default();
        let due = match self
            .store
            .claim_deliveries(self.settings.batch, self.settings.lease)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "claiming deliveries failed");
                return round;
            }
        };
        for outcome in join_all(due.iter().map(|d| self.attempt(d))).await {
            round.attempted += 1;
            match outcome {
                Ok(Settled::Delivered) => round.delivered += 1,
                Ok(Settled::Retry) => round.retried += 1,
                Ok(Settled::Dead { switched_off }) => {
                    round.dead += 1;
                    round.switched_off += u32::from(switched_off);
                }
                Err(()) => round.failed += 1,
            }
        }
        if let Ok(pending) = self.store.pending_deliveries().await {
            #[allow(clippy::cast_precision_loss)] // a gauge
            metrics::gauge!("outbound_pending_deliveries").set(pending as f64);
            #[allow(clippy::cast_precision_loss)] // compatibility gauge
            metrics::gauge!("webhook_pending_deliveries").set(pending as f64);
        }
        round
    }

    /// Removes deliveries past their retention. Run it rarely.
    pub async fn housekeeping(&self) {
        let mut removed = 0u64;
        for _ in 0..20 {
            match self
                .store
                .purge_deliveries(self.settings.retention, self.settings.purge_batch)
                .await
            {
                Ok(n) => {
                    removed += n;
                    if n < u64::from(self.settings.purge_batch) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not purge old deliveries");
                    break;
                }
            }
        }
        if removed > 0 {
            tracing::info!(removed, "purged old deliveries");
        }
    }

    /// Makes one attempt at one delivery and records it, in the trace of the run the event is about
    /// when the event says which: the receiver gets a `traceparent` whose parent is this attempt.
    async fn attempt(&self, due: &DueDelivery) -> Result<Settled, ()> {
        let link = delivery_trace(&due.delivery.payload);
        let span = tracing::info_span!(
            "webhook.delivery",
            trace_id = link.as_ref().map(|l| l.trace.as_str()),
            span_id = %link.as_ref().map_or_else(SpanId::generate, |l| l.span),
            parent_span_id = link.as_ref().map(|l| l.run_span.to_string()),
            otel.kind = "client",
            attempt = due.delivery.attempts + 1,
        );
        let traceparent = link.as_ref().and_then(|l| {
            TraceId::parse(&l.trace).map(|trace| sagat_telemetry::traceparent(trace, l.span, true))
        });
        self.attempt_traced(due, traceparent).instrument(span).await
    }

    async fn attempt_traced(
        &self,
        due: &DueDelivery,
        traceparent: Option<String>,
    ) -> Result<Settled, ()> {
        let delivery = &due.delivery;
        let number = delivery.attempts + 1;
        let started_at = self.clock.now();
        let started = Instant::now();

        let result = match &due.destination {
            OutboundDestination::Webhook { url, .. } => match self.sign(due).await {
                Some(signature) => {
                    let request = OutboundRequest {
                        tenant_id: delivery.tenant_id,
                        namespace_id: delivery.namespace_id,
                        url: url.clone(),
                        headers: vec![
                            ("content-type".into(), "application/json".into()),
                            ("user-agent".into(), "sagat-octopus-orchestrator/1".into()),
                            ("x-octopus-signature".into(), signature.header),
                            (
                                "x-octopus-timestamp".into(),
                                signature.timestamp.to_string(),
                            ),
                            ("x-octopus-event-id".into(), delivery.event_id.clone()),
                            ("x-octopus-event-type".into(), delivery.kind.to_string()),
                            // The same on every attempt: the receiver's idempotency key.
                            ("x-octopus-delivery-id".into(), delivery.id.to_string()),
                            ("x-octopus-attempt".into(), number.to_string()),
                            ("x-octopus-schema-version".into(), SCHEMA_VERSION.into()),
                        ]
                        .into_iter()
                        .chain(traceparent.map(|value| ("traceparent".to_owned(), value)))
                        .collect(),
                        body: delivery.payload.clone(),
                        timeout: due.limits.timeout(),
                    };
                    match self.sender.send(&request).await {
                        Ok(response) => {
                            let (outcome, verdict) =
                                judge_response(response.status, response.retry_after);
                            (outcome, Some(response.status), verdict)
                        }
                        Err(SendFailure::Timeout) => (
                            AttemptOutcome::Timeout,
                            None,
                            Verdict::Retry { after: None },
                        ),
                        Err(SendFailure::Network) => (
                            AttemptOutcome::Network,
                            None,
                            Verdict::Retry { after: None },
                        ),
                        // The administrator may widen the policy: worth trying again later.
                        Err(SendFailure::Denied) => {
                            (AttemptOutcome::Denied, None, Verdict::Retry { after: None })
                        }
                        // A URL that cannot be a destination will not become one.
                        Err(SendFailure::Invalid) => {
                            (AttemptOutcome::Rejected, None, Verdict::Dead)
                        }
                    }
                }
                // Nothing to sign with (the secret was deleted, or no key opens it): the
                // administrator can fix that, so the delivery waits like a receiver that is down.
                None => (
                    AttemptOutcome::Unsigned,
                    None,
                    Verdict::Retry { after: None },
                ),
            },
            OutboundDestination::Broker {
                settings,
                credentials_secret,
            } => {
                let credentials = match credentials_secret {
                    Some(name) => match self.credentials(due, name).await {
                        Some(value) => Some(value),
                        None => {
                            return self
                                .settle(
                                    due,
                                    started_at,
                                    started,
                                    number,
                                    AttemptResult {
                                        outcome: AttemptOutcome::CredentialUnavailable,
                                        http_status: None,
                                        verdict: Verdict::Retry { after: None },
                                    },
                                )
                                .await;
                        }
                    },
                    None => None,
                };
                let request = BrokerRequest {
                    tenant_id: delivery.tenant_id,
                    namespace_id: delivery.namespace_id,
                    subscription_id: delivery.subscription_id,
                    destination: settings.clone(),
                    credentials,
                    headers: vec![
                        ("content-type".into(), "application/json".into()),
                        ("event-id".into(), delivery.event_id.clone()),
                        ("event-type".into(), delivery.kind.to_string()),
                        // Stable across attempts: adapters use it as their idempotency metadata.
                        ("delivery-id".into(), delivery.id.to_string()),
                        ("attempt".into(), number.to_string()),
                        ("schema-version".into(), SCHEMA_VERSION.into()),
                    ]
                    .into_iter()
                    .chain(traceparent.map(|value| ("traceparent".to_owned(), value)))
                    .collect(),
                    body: delivery.payload.clone(),
                    timeout: due.limits.timeout(),
                };
                match self.broker.publish(&request).await {
                    Ok(()) => (AttemptOutcome::Delivered, None, Verdict::Done),
                    Err(BrokerFailure::Timeout) => (
                        AttemptOutcome::Timeout,
                        None,
                        Verdict::Retry { after: None },
                    ),
                    Err(BrokerFailure::Network) => (
                        AttemptOutcome::Network,
                        None,
                        Verdict::Retry { after: None },
                    ),
                    Err(BrokerFailure::Denied) => {
                        (AttemptOutcome::Denied, None, Verdict::Retry { after: None })
                    }
                    // Invalid settings are rejected at management time. A broker rejection can be
                    // repaired out of band, so preserve the durable retry/DLQ path.
                    Err(BrokerFailure::Rejected) => (
                        AttemptOutcome::Rejected,
                        None,
                        Verdict::Retry { after: None },
                    ),
                }
            }
        };
        let (outcome, http_status, verdict) = result;

        self.settle(
            due,
            started_at,
            started,
            number,
            AttemptResult {
                outcome,
                http_status,
                verdict,
            },
        )
        .await
    }

    async fn settle(
        &self,
        due: &DueDelivery,
        started_at: time::OffsetDateTime,
        started: Instant,
        number: u32,
        result: AttemptResult,
    ) -> Result<Settled, ()> {
        let delivery = &due.delivery;
        let AttemptResult {
            outcome,
            http_status,
            verdict,
        } = result;

        let next = match verdict {
            Verdict::Done => Next::Delivered,
            Verdict::Dead => Next::Dead,
            Verdict::Retry { after } => {
                // Failures since the delivery was last given a full set of attempts.
                let failed = number.saturating_sub(delivery.attempts_before_redrive);
                if out_of_attempts(failed) {
                    Next::Dead
                } else {
                    Next::Retry {
                        delay: delivery_backoff(failed, self.entropy.unit(), after),
                    }
                }
            }
        };
        let record = AttemptRecord {
            attempt: number,
            started_at,
            duration_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            outcome,
            http_status,
            error: outcome_note(outcome),
        };
        let (disabled_action, resource_type, metric_prefix) = match due.destination {
            OutboundDestination::Webhook { .. } => (
                AuditAction::WebhookSubscriptionDisabled,
                ResourceType::WebhookSubscription,
                "webhook",
            ),
            OutboundDestination::Broker { .. } => (
                AuditAction::BrokerSubscriptionDisabled,
                ResourceType::BrokerSubscription,
                "broker",
            ),
        };
        let off = AuditRecord::engine(
            delivery.tenant_id,
            disabled_action,
            resource_type,
            delivery.subscription_id.to_string(),
            "too many deliveries in a row were set aside",
            None,
            started_at,
        );
        metrics::histogram!("outbound_attempt_seconds", "destination" => metric_prefix)
            .record(started.elapsed().as_secs_f64());
        metrics::counter!("outbound_attempts_total", "destination" => metric_prefix, "outcome" => outcome.as_str())
            .increment(1);
        if metric_prefix == "webhook" {
            metrics::histogram!("webhook_attempt_seconds").record(started.elapsed().as_secs_f64());
            metrics::counter!("webhook_attempts_total", "outcome" => outcome.as_str()).increment(1);
        }
        match self
            .store
            .record_attempt(due, &record, &next, Some(&off))
            .await
        {
            Ok(switched_off) => match &next {
                Next::Delivered => {
                    let lag = (self.clock.now() - delivery.created_at).as_seconds_f64();
                    metrics::histogram!("outbound_delivery_lag_seconds", "destination" => metric_prefix)
                        .record(lag.max(0.0));
                    metrics::counter!("outbound_deliveries_total", "destination" => metric_prefix, "outcome" => "delivered")
                        .increment(1);
                    if metric_prefix == "webhook" {
                        metrics::histogram!("webhook_delivery_lag_seconds").record(lag.max(0.0));
                        metrics::counter!("webhook_deliveries_total", "outcome" => "delivered")
                            .increment(1);
                    }
                    Ok(Settled::Delivered)
                }
                Next::Retry { .. } => Ok(Settled::Retry),
                Next::Dead => {
                    metrics::counter!("outbound_deliveries_total", "destination" => metric_prefix, "outcome" => "dead").increment(1);
                    if metric_prefix == "webhook" {
                        metrics::counter!("webhook_deliveries_total", "outcome" => "dead")
                            .increment(1);
                    }
                    if switched_off {
                        tracing::warn!(
                            subscription = %delivery.subscription_id,
                            "a subscription was switched off after too many dead deliveries"
                        );
                        metrics::counter!("outbound_subscriptions_switched_off_total", "destination" => metric_prefix).increment(1);
                        if metric_prefix == "webhook" {
                            metrics::counter!("webhook_subscriptions_switched_off_total")
                                .increment(1);
                        }
                    }
                    Ok(Settled::Dead { switched_off })
                }
            },
            Err(error) => {
                // The lease runs out and the delivery is made again with the same id.
                tracing::error!(%error, delivery = %delivery.id, "could not record a delivery attempt");
                Err(())
            }
        }
    }

    /// The signature header for the delivery, or `None` when nothing can sign it. Every version of
    /// the secret that is still kept (the current and the one before) signs, so a receiver that
    /// verifies with either accepts the request.
    async fn sign(&self, due: &DueDelivery) -> Option<Signature> {
        let delivery = &due.delivery;
        let (_, secret_name) = due.webhook()?;
        let versions = self
            .store
            .signing_secrets(
                delivery.tenant_id,
                delivery.namespace_id,
                secret_name,
                SIGNING_VERSIONS,
            )
            .await
            .map_err(|error| tracing::error!(%error, "reading the signing secret failed"))
            .ok()?;
        let timestamp = self.clock.now().unix_timestamp();
        let mut parts = Vec::with_capacity(versions.len());
        for stored in &versions {
            if let Ok(value) = self.cipher.resolve(stored).await {
                // The receiver is told the type in a header, so the signature covers it.
                let covered = signing::Covered {
                    timestamp,
                    event_id: &delivery.event_id,
                    event_type: delivery.kind.as_str(),
                    subject: "",
                    correlation_key: "",
                };
                // A secret too short to sign anything signs nothing; the others may.
                parts.extend(signing::sign(
                    signing::Direction::Outbound,
                    value.expose(),
                    &covered,
                    &delivery.payload,
                ));
            }
        }
        if parts.is_empty() {
            return None;
        }
        Some(Signature {
            header: parts.join(","),
            timestamp,
        })
    }

    async fn credentials(&self, due: &DueDelivery, name: &SecretName) -> Option<SecretValue> {
        let delivery = &due.delivery;
        let stored = self
            .store
            .signing_secrets(delivery.tenant_id, delivery.namespace_id, name, 1)
            .await
            .map_err(|error| tracing::error!(%error, "reading broker credentials failed"))
            .ok()?
            .into_iter()
            .next()?;
        self.cipher.resolve(&stored).await.ok()
    }
}

struct Signature {
    header: String,
    timestamp: i64,
}

enum Settled {
    Delivered,
    Retry,
    Dead { switched_off: bool },
}

struct AttemptResult {
    outcome: AttemptOutcome,
    http_status: Option<u16>,
    verdict: Verdict,
}

/// What is recorded about a failed attempt: a class, never a detail from the network or from the
/// receiver (either can carry what the receiver would rather not have in a log).
fn outcome_note(outcome: AttemptOutcome) -> Option<String> {
    match outcome {
        AttemptOutcome::Delivered => None,
        AttemptOutcome::Rejected => Some("the receiver refused the event".to_owned()),
        AttemptOutcome::ServerError => Some("the receiver answered with an error".to_owned()),
        AttemptOutcome::Throttled => Some("the receiver asked for fewer requests".to_owned()),
        AttemptOutcome::Timeout => Some("no answer in time".to_owned()),
        AttemptOutcome::Network => Some("the connection failed".to_owned()),
        AttemptOutcome::Denied => {
            Some("the egress policy of the namespace does not allow the destination".to_owned())
        }
        AttemptOutcome::Unsigned => Some("there is no signing secret to sign with".to_owned()),
        AttemptOutcome::CredentialUnavailable => {
            Some("there are no usable broker credentials".to_owned())
        }
    }
}

/// Where a delivery belongs in a trace: the trace of the run its event is about, the span of the run
/// (what the delivery hangs from), and the span of this delivery.
struct DeliveryTrace {
    trace: String,
    run_span: SpanId,
    span: SpanId,
}

/// Reads the run and its trace from the body of a `run.finished` event (`data.runId`, `data.traceId`);
/// any other event, or one with no trace, is delivered with none.
fn delivery_trace(payload: &[u8]) -> Option<DeliveryTrace> {
    let body: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let data = body.get("data")?;
    let trace = data.get("traceId")?.as_str()?;
    TraceId::parse(trace)?;
    let run = uuid::Uuid::parse_str(data.get("runId")?.as_str()?).ok()?;
    Some(DeliveryTrace {
        trace: trace.to_owned(),
        run_span: sagat_telemetry::run_span_id(run),
        span: SpanId::generate(),
    })
}
