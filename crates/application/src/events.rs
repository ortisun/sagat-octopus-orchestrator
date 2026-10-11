//! Events in: the endpoints that receive signed webhooks, the API that publishes events, and the
//! inbox both admit them to (`FR-004-001`..`FR-004-004`, `FR-004-008`).
//!
//! Admitting an event is one insert and nothing else. What the event wakes or starts is decided
//! afterwards, from the inbox, by the dispatcher of the scheduler, so the answer to the sender
//! never waits for that work (the ingress promise) and a crash between the answer and the work
//! loses nothing: the row is the work. The primary key of the inbox is the deduplication, so a
//! provider that retries gets the first answer, and a provider that reuses an id for something
//! else is told so instead of being ignored.
//!
//! A webhook endpoint is reachable by anybody who can open a connection, so everything on that
//! path is ordered by what an attacker can cost us: the cheapest checks come first, the signature
//! is verified before the body is looked at, and every refusal looks the same from outside (one
//! `401`), with the reason only in the metrics and in a collapsed audit record.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::events::{
    EndpointId, EndpointLimits, EndpointStatus, EventEnvelope, EventError, EventId, EventKey,
    InboxStatus,
};
use sagat_domain::execution::{CorrelationKey, RunId};
use sagat_domain::identity::{Namespace, NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::{SecretCipher, SecretName, associated_data};
use sagat_domain::workflow::EventType;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::error::{AppError, ConflictKind, StoreError};
use crate::execution::ExpressionEvaluator;
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;
use crate::secrets::{SecretAccessLookup, SecretProblem, StoredSecret, check_usable};
use crate::signing::{self, SignatureError};

/// The source of every event published through the API. An endpoint cannot be given this slug, so
/// the two never share a deduplication scope.
pub const API_SOURCE: &str = "api";

/// Events waiting to be dispatched, for one tenant, above which admitting more is refused with a
/// retryable `429` (`FR-004-008`): the sender is told to slow down while the matcher catches up,
/// and nothing already admitted is lost.
pub const BACKLOG_LIMIT: u64 = 10_000;

/// What the refusal for a full backlog tells the sender to wait.
pub const BACKLOG_RETRY_AFTER_SECS: u32 = 5;

/// Versions of a signing secret that are tried: the current one and the two before it, so a
/// provider that has not yet moved to a rotated secret keeps being accepted for a while.
pub const SIGNING_VERSIONS: u32 = 3;

/// An endpoint as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointRecord {
    pub id: EndpointId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub slug: Slug,
    /// The secret of the namespace that signs the requests: a name, never a value.
    pub secret_name: SecretName,
    /// The type given to a request that does not say one.
    pub default_event_type: Option<EventType>,
    pub limits: EndpointLimits,
    pub status: EndpointStatus,
    pub created_by: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// What an administrator chooses about an endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointSettings {
    pub secret_name: SecretName,
    pub default_event_type: Option<EventType>,
    pub limits: EndpointLimits,
    pub status: EndpointStatus,
}

/// An event to admit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEvent {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub envelope: EventEnvelope,
    pub occurred_at: Option<OffsetDateTime>,
    /// The data: a JSON object.
    pub payload: Vec<u8>,
    /// What identifies the content, for telling a retry from a reuse of the id.
    pub digest: Vec<u8>,
    /// How long the event is remembered for deduplication, counted from the moment the database
    /// admits it: every instant that decides what an event may wake is the database's, so
    /// instances with skewed clocks agree.
    pub dedupe_ttl: Duration,
    /// The trace the event was published in, when its publisher sent one: the run it starts
    /// continues it (G79).
    pub trace: Option<crate::identity::TraceLink>,
}

/// What admitting an event came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// New: it is in the inbox.
    Accepted,
    /// The same event (same id, same content) was admitted before and is still remembered.
    Duplicate,
    /// The id is remembered, for different content.
    Reused,
    /// The tenant's backlog is full; nothing was written.
    Backlogged,
}

/// An event as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub envelope: EventEnvelope,
    pub occurred_at: Option<OffsetDateTime>,
    pub payload: Vec<u8>,
    pub received_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub status: InboxStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub dispatched_at: Option<OffsetDateTime>,
    /// What the event woke or started.
    pub deliveries: Vec<Delivery>,
}

/// One consumer an event reached: the evidence that it was delivered, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// `wait:{run}/{node}` for a waiting step, `trigger:{id}` for a trigger.
    pub consumer: String,
    pub run_id: Option<RunId>,
    pub node: Option<String>,
    pub trigger_id: Option<Uuid>,
    pub delivered_at: OffsetDateTime,
}

/// Storage port of the endpoints and the inbox. Implemented by `sagat-infra-postgres`.
pub trait EventStore: NamespaceDirectory + QuotaLookup + SecretAccessLookup {
    /// A namespace of the tenant by its slug.
    fn namespace_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> impl Future<Output = Result<Option<Namespace>, StoreError>> + Send;

    /// Stores a new endpoint with its audit record, counting it against the namespace's quota under
    /// a lock on the counted scope. A slug already taken is `Conflict(EventEndpointSlug)`.
    fn create_endpoint(
        &self,
        endpoint: &EndpointRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Replaces the settings of an endpoint. `false` when it does not exist.
    fn update_endpoint(
        &self,
        endpoint: &EndpointRecord,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Removes an endpoint. Events it admitted stay until they expire. `false` when it did not
    /// exist.
    fn delete_endpoint(
        &self,
        tenant: TenantId,
        id: EndpointId,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn endpoint(
        &self,
        tenant: TenantId,
        id: EndpointId,
    ) -> impl Future<Output = Result<Option<EndpointRecord>, StoreError>> + Send;

    fn endpoints(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<EndpointRecord>, StoreError>> + Send;

    /// An endpoint by the id its URL carries. The only lookup that is not tenant-scoped: the caller
    /// of a webhook has no tenant until the endpoint says which one it belongs to.
    fn endpoint_by_id(
        &self,
        id: EndpointId,
    ) -> impl Future<Output = Result<Option<EndpointRecord>, StoreError>> + Send;

    /// The newest `limit` versions of a secret of the namespace, newest first.
    fn signing_secrets(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StoredSecret>, StoreError>> + Send;

    /// Admits the event, unless the id is remembered or the tenant's backlog has reached
    /// `backlog_limit`. An id whose window has ended is a new event.
    ///
    /// `audit`, when given, is written in the same transaction if (and only if) the event is
    /// admitted: a repeat adds nothing to the trail.
    fn admit_event(
        &self,
        event: &NewEvent,
        backlog_limit: u64,
        audit: Option<&AuditRecord>,
    ) -> impl Future<Output = Result<Admission, StoreError>> + Send;

    fn event(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        key: &EventKey,
    ) -> impl Future<Output = Result<Option<StoredEvent>, StoreError>> + Send;
}

/// A request to a webhook endpoint, as the HTTP adapter read it. Nothing here is trusted.
#[derive(Debug, Clone, Copy)]
pub struct WebhookRequest<'a> {
    pub signature: Option<&'a str>,
    pub timestamp: Option<&'a str>,
    pub event_id: Option<&'a str>,
    pub event_type: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub correlation_key: Option<&'a str>,
    pub body: &'a [u8],
    /// The trace of the request that carries the webhook.
    pub trace: Option<&'a crate::identity::TraceLink>,
}

/// An event to publish through the API.
#[derive(Debug, Clone)]
pub struct PublishRequest {
    /// Slug of the namespace.
    pub namespace: String,
    /// Absent: the platform makes one up, and the event is then never a duplicate of another.
    pub event_id: Option<EventId>,
    pub event_type: EventType,
    pub subject: Option<String>,
    pub correlation_key: Option<CorrelationKey>,
    pub occurred_at: Option<OffsetDateTime>,
    pub data: Vec<u8>,
    pub dedupe_ttl: Option<Duration>,
}

/// What an admitted request came to, for the sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub key: EventKey,
    /// Already admitted earlier: nothing was written.
    pub duplicate: bool,
}

/// Why a webhook request is refused, as far as the metrics and the audit trail are concerned. The
/// sender never sees which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    UnknownEndpoint,
    Malformed,
    Stale,
    BadSignature,
    NoSecret,
}

impl Refusal {
    const fn label(self) -> &'static str {
        match self {
            Self::UnknownEndpoint => "unknown_endpoint",
            Self::Malformed => "malformed",
            Self::Stale => "stale_timestamp",
            Self::BadSignature => "bad_signature",
            Self::NoSecret => "no_signing_secret",
        }
    }
}

/// Why a request was not verified: it did not verify, or nothing could be said about it.
enum Unverified {
    Refused(Refusal),
    Store(AppError),
}

impl From<Refusal> for Unverified {
    fn from(reason: Refusal) -> Self {
        Self::Refused(reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Stage {
    /// Every request that reaches a known endpoint, signed or not.
    Attempt,
    /// Requests whose signature verified.
    Verified,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: OffsetDateTime,
}

/// Token buckets per endpoint and stage. A bucket is created only for an endpoint that exists, so
/// the table is bounded by the endpoints the quota allows, and idle entries are swept.
#[derive(Debug, Default)]
struct Gate {
    buckets: Mutex<HashMap<(EndpointId, Stage), Bucket>>,
}

impl Gate {
    const SWEEP_ABOVE: usize = 4096;

    /// Takes a token, or says how long until there is one.
    fn take(
        &self,
        key: (EndpointId, Stage),
        per_second: f64,
        burst: f64,
        now: OffsetDateTime,
    ) -> Result<(), Duration> {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if buckets.len() > Self::SWEEP_ABOVE {
            // An idle bucket is full again and carries nothing worth keeping.
            buckets.retain(|_, b| (now - b.at).as_seconds_f64() < 600.0);
        }
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: burst,
            at: now,
        });
        let elapsed = (now - bucket.at).as_seconds_f64().max(0.0);
        bucket.tokens = elapsed.mul_add(per_second, bucket.tokens).min(burst);
        bucket.at = bucket.at.max(now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let wait = (1.0 - bucket.tokens) / per_second;
            Err(Duration::try_from_secs_f64(wait)
                .unwrap_or(Duration::from_secs(3600))
                .min(Duration::from_secs(3600)))
        }
    }
}

/// Manages endpoints, publishes events and receives webhooks.
pub struct EventService<S, C> {
    store: S,
    clock: C,
    cipher: Arc<dyn SecretCipher>,
    evaluator: Arc<dyn ExpressionEvaluator>,
    gate: Gate,
}

impl<S, C> EventService<S, C>
where
    S: EventStore,
    C: Clock,
{
    #[must_use]
    pub fn new(
        store: S,
        clock: C,
        cipher: Arc<dyn SecretCipher>,
        evaluator: Arc<dyn ExpressionEvaluator>,
    ) -> Self {
        Self {
            store,
            clock,
            cipher,
            evaluator,
            gate: Gate::default(),
        }
    }

    /// Creates an endpoint in a namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:manage`; [`AppError::NotFound`] for a namespace the
    /// tenant does not have; [`AppError::NotAllowed`] for the reserved slug;
    /// [`AppError::SecretUnavailable`] when the signing secret does not exist or the caller may not
    /// use it; [`AppError::Conflict`] for a slug already taken in the namespace;
    /// [`AppError::QuotaExceeded`].
    pub async fn create_endpoint(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        slug: Slug,
        settings: EndpointSettings,
        ctx: &RequestContext,
    ) -> Result<EndpointRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        if sagat_domain::connectors::is_reserved(&slug) {
            return Err(AppError::NotAllowed(
                "the slugs 'api' and 'cron' are reserved for events published through the API and \
                 for schedules",
            ));
        }
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        self.require_usable_secret(principal, namespace, &settings.secret_name)
            .await?;
        let now = self.clock.now();
        let endpoint = EndpointRecord {
            id: EndpointId::random(),
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            slug,
            secret_name: settings.secret_name,
            default_event_type: settings.default_event_type,
            limits: settings.limits,
            status: settings.status,
            created_by: principal.subject().as_str().to_owned(),
            created_at: now,
            updated_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::EventEndpoints,
            )
            .await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::EventEndpointCreated,
            ResourceType::EventEndpoint,
            endpoint.id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!("endpoint '{}'", endpoint.slug));
        self.store.create_endpoint(&endpoint, limit, &audit).await?;
        Ok(endpoint)
    }

    /// Replaces the settings of an endpoint. The slug and the id never change: workflows name the
    /// first and providers hold the second.
    ///
    /// # Errors
    /// As [`Self::create_endpoint`], and [`AppError::NotFound`] for an unknown endpoint.
    pub async fn update_endpoint(
        &self,
        principal: &Principal,
        id: EndpointId,
        settings: EndpointSettings,
        ctx: &RequestContext,
    ) -> Result<EndpointRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        let mut endpoint = self.load(principal, id).await?;
        self.require_usable_secret(principal, endpoint.namespace_id, &settings.secret_name)
            .await?;
        let now = self.clock.now();
        endpoint.secret_name = settings.secret_name;
        endpoint.default_event_type = settings.default_event_type;
        endpoint.limits = settings.limits;
        endpoint.status = settings.status;
        endpoint.updated_at = now;
        let audit = AuditRecord::by(
            principal,
            AuditAction::EventEndpointChanged,
            ResourceType::EventEndpoint,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "endpoint '{}' is {}",
            endpoint.slug,
            endpoint.status.as_str().to_lowercase()
        ));
        if !self.store.update_endpoint(&endpoint, &audit).await? {
            return Err(AppError::NotFound);
        }
        Ok(endpoint)
    }

    /// Removes an endpoint: its URL answers `401` from now on.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn delete_endpoint(
        &self,
        principal: &Principal,
        id: EndpointId,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::EventManage)?;
        let endpoint = self.load(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::EventEndpointDeleted,
            ResourceType::EventEndpoint,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("endpoint '{}'", endpoint.slug));
        if self
            .store
            .delete_endpoint(principal.tenant_id(), id, &audit)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// One endpoint of the caller's tenant.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:read`; [`AppError::NotFound`].
    pub async fn get_endpoint(
        &self,
        principal: &Principal,
        id: EndpointId,
    ) -> Result<EndpointRecord, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load(principal, id).await
    }

    /// The endpoints of a namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn list_endpoints(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<EndpointRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .endpoints(principal.tenant_id(), namespace)
            .await?)
    }

    /// Publishes an event through the API (source `api`).
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:publish`; [`AppError::NotFound`] for an unknown
    /// namespace; [`AppError::NotAllowed`] for data that is not a JSON object or is too large;
    /// [`AppError::Conflict`] with `EventIdReused`; [`AppError::Busy`] when the tenant's backlog is
    /// full.
    pub async fn publish(
        &self,
        principal: &Principal,
        request: PublishRequest,
        ctx: &RequestContext,
    ) -> Result<Received, AppError> {
        principal.authorize(Permission::EventPublish)?;
        if request.data.len() > sagat_domain::events::DEFAULT_MAX_BODY {
            return Err(AppError::PayloadTooLarge("event data"));
        }
        if !self.evaluator.is_json_object(&request.data) {
            return Err(AppError::NotAllowed("the event data must be a JSON object"));
        }
        let slug = Slug::parse(&request.namespace).map_err(|_| AppError::NotFound)?;
        let namespace = self
            .store
            .namespace_by_slug(principal.tenant_id(), &slug)
            .await?
            .ok_or(AppError::NotFound)?;
        let ttl = match request.dedupe_ttl {
            None => sagat_domain::events::DEFAULT_DEDUPE_TTL,
            Some(ttl) => sagat_domain::events::checked_dedupe_ttl(ttl).map_err(event_error)?,
        };
        let event_id = match request.event_id {
            Some(id) => id,
            None => EventId::parse(&Uuid::new_v4().to_string()).map_err(event_error)?,
        };
        let key = EventKey {
            source: Slug::parse(API_SOURCE).map_err(|_| AppError::Internal)?,
            event_id,
        };
        let envelope = EventEnvelope::new(
            key.clone(),
            request.event_type,
            request.subject.as_deref(),
            request.correlation_key,
        )
        .map_err(event_error)?;
        let mut event = new_event(
            principal.tenant_id(),
            namespace.id(),
            envelope,
            request.occurred_at,
            request.data,
            ttl,
        );
        event.trace = ctx.link();
        let audit = AuditRecord::by(
            principal,
            AuditAction::EventPublished,
            ResourceType::Event,
            format!("{}/{}", key.source, key.event_id),
            ctx.trace_id.clone(),
            self.clock.now(),
        );
        match self
            .store
            .admit_event(&event, BACKLOG_LIMIT, Some(&audit))
            .await?
        {
            Admission::Accepted => {
                metrics::counter!("events_received_total", "kind" => "api", "outcome" => "accepted")
                    .increment(1);
                Ok(Received {
                    key,
                    duplicate: false,
                })
            }
            Admission::Duplicate => {
                metrics::counter!("events_received_total", "kind" => "api", "outcome" => "duplicate")
                    .increment(1);
                Ok(Received {
                    key,
                    duplicate: true,
                })
            }
            Admission::Reused => {
                metrics::counter!("events_received_total", "kind" => "api", "outcome" => "reused")
                    .increment(1);
                Err(AppError::Conflict(ConflictKind::EventIdReused))
            }
            Admission::Backlogged => {
                metrics::counter!("events_received_total", "kind" => "api", "outcome" => "backlogged")
                    .increment(1);
                tracing::warn!(tenant = %principal.tenant_id(), "event backlog is full");
                Err(AppError::Busy {
                    retry_after_secs: BACKLOG_RETRY_AFTER_SECS,
                })
            }
        }
    }

    /// An event of the inbox, with what it reached.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:read`; [`AppError::NotFound`].
    pub async fn event(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        key: &EventKey,
        ctx: &RequestContext,
    ) -> Result<StoredEvent, AppError> {
        principal.authorize(Permission::EventRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        self.store
            .event(principal.tenant_id(), namespace, key)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Receives a request on the public URL of an endpoint.
    ///
    /// The caller has no tenant, no token and no say in the order of the checks: unknown endpoint,
    /// bad or stale signature, missing secret and malformed headers all end in
    /// [`AppError::Unauthenticated`], which the adapter answers with one plain `401`.
    ///
    /// # Errors
    /// [`AppError::Unauthenticated`] for anything that does not verify; [`AppError::Unavailable`]
    /// for a paused endpoint; [`AppError::PayloadTooLarge`]; [`AppError::Busy`] for a sender over
    /// its rate (or a full backlog); [`AppError::NotAllowed`] for a verified request that is not an
    /// event; [`AppError::Conflict`] with `EventIdReused`.
    pub async fn receive(
        &self,
        endpoint: EndpointId,
        request: &WebhookRequest<'_>,
    ) -> Result<Received, AppError> {
        let now = self.clock.now();
        let Some(record) = self.store.endpoint_by_id(endpoint).await? else {
            return Err(refuse(Refusal::UnknownEndpoint));
        };
        if record.status == EndpointStatus::Paused {
            metrics::counter!("webhook_rejected_total", "reason" => "paused").increment(1);
            return Err(AppError::Unavailable);
        }
        if request.body.len() > record.limits.max_body() {
            metrics::counter!("webhook_rejected_total", "reason" => "too_large").increment(1);
            return Err(AppError::PayloadTooLarge("webhook body"));
        }
        // Before any work that costs: what the endpoint can be made to spend by a caller who has
        // not proved anything is bounded far above its real rate and far below the database's.
        let rate = f64::from(record.limits.rate_per_second());
        self.gate
            .take((record.id, Stage::Attempt), rate * 10.0, rate * 20.0, now)
            .map_err(|wait| busy(wait, "attempts"))?;

        let verified = self.verify(&record, request, now).await;
        let (event_id, event_type) = match verified {
            Ok(parts) => parts,
            // The database could not be read: not a verdict on the request. The provider is told to
            // try again (a 401 would teach it that its secret is wrong).
            Err(Unverified::Store(error)) => return Err(error),
            Err(Unverified::Refused(Refusal::NoSecret)) => {
                // Not the sender's fault and not theirs to learn: an operator has to see it.
                tracing::error!(endpoint = %record.id, "the signing secret of an endpoint cannot be read");
                self.audit_refusal(&record, Refusal::NoSecret, now).await;
                return Err(refuse(Refusal::NoSecret));
            }
            Err(Unverified::Refused(reason)) => {
                self.audit_refusal(&record, reason, now).await;
                return Err(refuse(reason));
            }
        };

        self.gate
            .take(
                (record.id, Stage::Verified),
                rate,
                (rate * 2.0).max(2.0),
                now,
            )
            .map_err(|wait| busy(wait, "rate"))?;
        if !self.evaluator.is_json_object(request.body) {
            return Err(AppError::NotAllowed("the body must be a JSON object"));
        }
        let key = EventKey {
            source: record.slug.clone(),
            event_id,
        };
        let correlation = request
            .correlation_key
            .map(CorrelationKey::parse)
            .transpose()
            .map_err(|_| AppError::NotAllowed("the correlation key is not usable"))?;
        let envelope = EventEnvelope::new(key.clone(), event_type, request.subject, correlation)
            .map_err(event_error)?;
        let mut event = new_event(
            record.tenant_id,
            record.namespace_id,
            envelope,
            None,
            request.body.to_vec(),
            record.limits.dedupe_ttl(),
        );
        event.trace = request.trace.cloned();
        match self.store.admit_event(&event, BACKLOG_LIMIT, None).await? {
            Admission::Accepted => {
                metrics::counter!("events_received_total", "kind" => "webhook", "outcome" => "accepted")
                    .increment(1);
                Ok(Received {
                    key,
                    duplicate: false,
                })
            }
            Admission::Duplicate => {
                metrics::counter!("events_received_total", "kind" => "webhook", "outcome" => "duplicate")
                    .increment(1);
                Ok(Received {
                    key,
                    duplicate: true,
                })
            }
            Admission::Reused => {
                metrics::counter!("events_received_total", "kind" => "webhook", "outcome" => "reused")
                    .increment(1);
                self.audit_reuse(&record, now).await;
                Err(AppError::Conflict(ConflictKind::EventIdReused))
            }
            Admission::Backlogged => {
                metrics::counter!("events_received_total", "kind" => "webhook", "outcome" => "backlogged")
                    .increment(1);
                tracing::warn!(tenant = %record.tenant_id, "event backlog is full");
                Err(AppError::Busy {
                    retry_after_secs: BACKLOG_RETRY_AFTER_SECS,
                })
            }
        }
    }

    /// Checks the headers and the signature. On success: the event id and type.
    async fn verify(
        &self,
        record: &EndpointRecord,
        request: &WebhookRequest<'_>,
        now: OffsetDateTime,
    ) -> Result<(EventId, EventType), Unverified> {
        let (Some(signature), Some(timestamp), Some(event_id)) =
            (request.signature, request.timestamp, request.event_id)
        else {
            return Err(Refusal::Malformed.into());
        };
        let Ok(timestamp_seconds) = timestamp.parse::<i64>() else {
            return Err(Refusal::Malformed.into());
        };
        let Ok(event_id) = EventId::parse(event_id) else {
            return Err(Refusal::Malformed.into());
        };
        // What costs nothing comes first: a header that is too long, too full or not a signature at
        // all, and a timestamp outside the window, are refused before a secret is read or opened or
        // a single hash is computed. The header is written by whoever knows the URL.
        signing::precheck(
            signature,
            timestamp_seconds,
            now.unix_timestamp(),
            record.limits.skew(),
        )
        .map_err(refusal_of)?;

        let versions = self
            .store
            .signing_secrets(
                record.tenant_id,
                record.namespace_id,
                &record.secret_name,
                SIGNING_VERSIONS,
            )
            .await
            .map_err(|error| {
                tracing::error!(%error, "reading the signing secret failed");
                Unverified::Store(error.into())
            })?;
        let mut values = Vec::with_capacity(versions.len());
        for stored in &versions {
            let aad = associated_data(
                record.tenant_id,
                record.namespace_id,
                &stored.meta.name,
                stored.meta.id,
                &stored.meta.key_id,
            );
            // A version no key opens is skipped, and so is one too short to mean anything: the
            // others may still verify the request.
            if let Ok(value) = self.cipher.open(&stored.sealed, &aad)
                && signing::usable(value.expose())
            {
                values.push(value);
            }
        }
        if values.is_empty() {
            return Err(Refusal::NoSecret.into());
        }
        let candidates: Vec<&[u8]> = values.iter().map(|v| v.expose()).collect();
        // What the sender says the event is, as it said it: the signature covers the headers, not
        // the endpoint's default.
        let covered = signing::Covered {
            timestamp: timestamp_seconds,
            event_id: event_id.as_str(),
            event_type: request.event_type.unwrap_or(""),
            subject: request.subject.unwrap_or(""),
            correlation_key: request.correlation_key.unwrap_or(""),
        };
        signing::verify(
            signing::Direction::Inbound,
            &candidates,
            signature,
            &covered,
            request.body,
            now.unix_timestamp(),
            record.limits.skew(),
        )
        .map_err(refusal_of)?;

        // Only after the signature: what the sender says about the event is theirs to say.
        let event_type = match request.event_type {
            Some(text) => EventType::parse(text).ok(),
            None => record.default_event_type.clone(),
        };
        let Some(event_type) = event_type else {
            return Err(Refusal::Malformed.into());
        };
        Ok((event_id, event_type))
    }

    async fn audit_refusal(&self, record: &EndpointRecord, reason: Refusal, now: OffsetDateTime) {
        let audit = AuditRecord::gateway(
            record.tenant_id,
            AuditAction::EventRejected,
            ResourceType::EventEndpoint,
            format!("{}/{}", record.id, reason.label()),
            format!("webhook refused: {}", reason.label()),
            now,
        );
        // Best effort and off the response path: the store drops the record when too many are
        // pending, and collapses repeats.
        if let Err(error) = NamespaceDirectory::record_denial(&self.store, &audit).await {
            tracing::error!(%error, "failed to record audit entry");
        }
    }

    async fn audit_reuse(&self, record: &EndpointRecord, now: OffsetDateTime) {
        let audit = AuditRecord::gateway(
            record.tenant_id,
            AuditAction::EventRejected,
            ResourceType::EventEndpoint,
            format!("{}/event_id_reused", record.id),
            "an event id was used again for a different payload",
            now,
        );
        if let Err(error) = NamespaceDirectory::record_denial(&self.store, &audit).await {
            tracing::error!(%error, "failed to record audit entry");
        }
    }

    async fn load(
        &self,
        principal: &Principal,
        id: EndpointId,
    ) -> Result<EndpointRecord, AppError> {
        // Scoped to the tenant: an endpoint of another tenant is a missing one.
        self.store
            .endpoint(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::NotFound)
    }

    async fn require_usable_secret(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: &SecretName,
    ) -> Result<(), AppError> {
        let states = self
            .store
            .secret_states(
                principal.tenant_id(),
                namespace,
                std::slice::from_ref(name),
                principal.subject().as_str(),
            )
            .await?;
        check_usable(Some(principal.role()), &states).map_err(|SecretProblem(name)| {
            AppError::SecretUnavailable {
                name: name.to_string(),
            }
        })
    }
}

fn event_error(error: EventError) -> AppError {
    AppError::NotAllowed(match error {
        EventError::InvalidEventId => "the event id must be 1 to 200 printable characters",
        EventError::InvalidSubject => {
            "the subject must be at most 256 characters without control characters"
        }
        EventError::InvalidDedupeTtl => {
            "the deduplication window must be between 1 minute and 30 days"
        }
        EventError::InvalidSkew => "the allowed clock skew must be between 1 second and 1 hour",
        EventError::DedupeShorterThanReplayWindow => {
            "the deduplication window must be at least twice the allowed clock skew"
        }
        EventError::InvalidBodyLimit => "the body limit must be between 1 KiB and 240 KiB",
        EventError::InvalidRate => "the rate must be between 1 and 10000 requests per second",
    })
}

/// Counts a refusal and gives the one answer every refusal gets.
fn refusal_of(error: SignatureError) -> Unverified {
    Unverified::Refused(match error {
        SignatureError::Malformed => Refusal::Malformed,
        SignatureError::Stale => Refusal::Stale,
        SignatureError::Mismatch => Refusal::BadSignature,
    })
}

fn refuse(reason: Refusal) -> AppError {
    metrics::counter!("webhook_rejected_total", "reason" => reason.label()).increment(1);
    AppError::Unauthenticated
}

fn busy(wait: Duration, why: &'static str) -> AppError {
    metrics::counter!("webhook_rejected_total", "reason" => why).increment(1);
    AppError::Busy {
        retry_after_secs: u32::try_from(wait.as_secs_f64().ceil() as u64)
            .unwrap_or(u32::MAX)
            .max(1),
    }
}

pub fn new_event(
    tenant: TenantId,
    namespace: NamespaceId,
    envelope: EventEnvelope,
    occurred_at: Option<OffsetDateTime>,
    payload: Vec<u8>,
    ttl: Duration,
) -> NewEvent {
    let digest = digest_of(&envelope, &payload);
    NewEvent {
        tenant_id: tenant,
        namespace_id: namespace,
        envelope,
        occurred_at,
        payload,
        digest,
        dedupe_ttl: ttl,
        trace: None,
    }
}

/// What makes two admissions of one event id "the same event": its type, subject, correlation key
/// and data. Length-prefixed, so no two different tuples concatenate to the same bytes.
fn digest_of(envelope: &EventEnvelope, payload: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    for part in [
        envelope.event_type.as_str().as_bytes(),
        envelope.subject.as_deref().unwrap_or("").as_bytes(),
        envelope
            .correlation_key
            .as_ref()
            .map_or("", CorrelationKey::as_str)
            .as_bytes(),
        payload,
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}
