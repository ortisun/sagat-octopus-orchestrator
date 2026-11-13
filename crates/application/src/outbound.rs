//! Events out: subscriptions to what happens on the platform, and the deliveries made to them
//! (`FR-004-006`, US04).
//!
//! A delivery is created in the same transaction that records the event it carries (a run
//! finishing), so an event is either owed to every subscription that wanted it or was never
//! committed. A worker then takes the deliveries that are due, signs and sends each through the
//! guarded egress client and records the attempt; what happens next is decided by
//! [`sagat_domain::outbound::judge_response`]. Nothing is promised that cannot be: delivery is
//! at-least-once, and the id of the delivery (the same on every attempt) is what lets a receiver
//! drop the repeat.

use std::future::Future;
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::outbound::{
    AttemptOutcome, BrokerDestination, DeliveryId, DeliveryLimits, DeliveryStatus,
    OutboundDestination, OutboundKind, SubscriptionId, SubscriptionStatus, TargetUrl, check_kinds,
};
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::{SecretName, SecretValue};
use time::OffsetDateTime;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::egress::{EgressService, EgressStore};
use crate::error::{AppError, ConflictKind, StoreError};
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;
use crate::secrets::{SecretAccessLookup, SecretProblem, StoredSecret, check_usable};

/// The health of a subscription, as the deliveries have shown it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Health {
    /// Deliveries in a row that ended dead.
    pub consecutive_dead: u32,
    pub last_success_at: Option<OffsetDateTime>,
    pub last_failure_at: Option<OffsetDateTime>,
    /// Why the platform switched it off, when it did.
    pub disabled_reason: Option<String>,
}

/// A subscription as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionRecord {
    pub id: SubscriptionId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub name: Slug,
    pub destination: OutboundDestination,
    pub kinds: Vec<OutboundKind>,
    pub limits: DeliveryLimits,
    pub status: SubscriptionStatus,
    pub health: Health,
    pub created_by: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// What an administrator chooses about a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionSettings {
    pub url: TargetUrl,
    pub secret_name: SecretName,
    pub kinds: Vec<OutboundKind>,
    pub limits: DeliveryLimits,
    /// `Active` or `Paused`; the platform alone sets `Disabled`.
    pub status: SubscriptionStatus,
}

/// What an administrator chooses about a broker subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerSubscriptionSettings {
    pub destination: BrokerDestination,
    pub credentials_secret: Option<SecretName>,
    pub kinds: Vec<OutboundKind>,
    pub limits: DeliveryLimits,
    pub status: SubscriptionStatus,
}

/// A delivery as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRecord {
    pub id: DeliveryId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub subscription_id: SubscriptionId,
    /// The id of the event (the same for every subscription it is delivered to).
    pub event_id: String,
    pub kind: OutboundKind,
    pub payload: Vec<u8>,
    pub status: DeliveryStatus,
    /// Attempts made in all, which number them.
    pub attempts: u32,
    /// How many of them were made before the last redrive; the delivery is given a full set of
    /// attempts after those.
    pub attempts_before_redrive: u32,
    pub next_attempt_at: OffsetDateTime,
    pub last_outcome: Option<AttemptOutcome>,
    pub last_http_status: Option<u16>,
    pub last_error: Option<String>,
    pub created_at: OffsetDateTime,
    pub delivered_at: Option<OffsetDateTime>,
}

/// One try at a delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    pub attempt: u32,
    pub started_at: OffsetDateTime,
    pub duration_ms: u32,
    pub outcome: AttemptOutcome,
    pub http_status: Option<u16>,
    /// A class of failure, never a detail from the network or the receiver's body.
    pub error: Option<String>,
}

/// Storage port of the subscriptions and their deliveries. Implemented by `sagat-infra-postgres`.
pub trait OutboundStore: NamespaceDirectory + QuotaLookup + SecretAccessLookup {
    /// Stores a new subscription with its audit record, counting it against the namespace's quota
    /// under a lock on the counted scope. A name already taken is
    /// `Conflict(WebhookSubscriptionName)`.
    fn create_subscription(
        &self,
        subscription: &SubscriptionRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Replaces the settings of a subscription. A subscription that was not `Active` and is now also
    /// forgets its failures and puts its dead deliveries back in the queue (the number is returned).
    /// `None` when it does not exist.
    fn update_subscription(
        &self,
        subscription: &SubscriptionRecord,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Option<u64>, StoreError>> + Send;

    /// Removes a subscription and its deliveries. `false` when it did not exist.
    fn delete_subscription(
        &self,
        tenant: TenantId,
        id: SubscriptionId,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn subscription(
        &self,
        tenant: TenantId,
        id: SubscriptionId,
    ) -> impl Future<Output = Result<Option<SubscriptionRecord>, StoreError>> + Send;

    fn subscriptions(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        brokers: bool,
    ) -> impl Future<Output = Result<Vec<SubscriptionRecord>, StoreError>> + Send;

    /// Deliveries of a subscription, newest first, with `id < before` when given.
    fn deliveries(
        &self,
        tenant: TenantId,
        subscription: SubscriptionId,
        status: Option<DeliveryStatus>,
        before: Option<DeliveryId>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<DeliveryRecord>, StoreError>> + Send;

    /// One delivery with every attempt made at it, oldest first.
    fn delivery(
        &self,
        tenant: TenantId,
        subscription: SubscriptionId,
        id: DeliveryId,
    ) -> impl Future<Output = Result<Option<(DeliveryRecord, Vec<AttemptRecord>)>, StoreError>> + Send;

    /// Puts dead deliveries of a subscription back in the queue: the one named, or up to `limit` of
    /// them, oldest first. They get a full set of attempts again; the ones already made stay in the
    /// record. Returns how many were put back.
    fn redrive(
        &self,
        tenant: TenantId,
        subscription: SubscriptionId,
        delivery: Option<DeliveryId>,
        limit: u32,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// Administers subscriptions and reads their deliveries.
pub struct OutboundService<S, C> {
    store: S,
    clock: C,
    egress: EgressService<S, C>,
    redis_callbacks: bool,
}

impl<S, C> OutboundService<S, C>
where
    S: OutboundStore + EgressStore,
    C: Clock,
{
    /// `egress` must be over the same store (it answers whether the namespace may reach a URL).
    #[must_use]
    pub const fn new(store: S, clock: C, egress: EgressService<S, C>) -> Self {
        Self {
            store,
            clock,
            egress,
            redis_callbacks: false,
        }
    }

    /// Enable only after all outbound readers and writers have been upgraded or stopped.
    /// This switch cannot fence an old process; the operator completes the cold rollout first.
    #[must_use]
    pub const fn with_redis_callbacks(mut self, enabled: bool) -> Self {
        self.redis_callbacks = enabled;
        self
    }

    /// Creates a subscription in a namespace.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:manage`; [`AppError::NotFound`];
    /// [`AppError::NotAllowed`] for kinds or a URL the egress policy of the namespace does not allow
    /// or a status the caller may not set; [`AppError::SecretUnavailable`];
    /// [`AppError::Conflict`]; [`AppError::QuotaExceeded`].
    pub async fn create(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: Slug,
        settings: SubscriptionSettings,
        ctx: &RequestContext,
    ) -> Result<SubscriptionRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        self.check(principal, namespace, &settings, ctx).await?;
        let now = self.clock.now();
        let subscription = SubscriptionRecord {
            id: SubscriptionId::generate(),
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            name,
            destination: OutboundDestination::Webhook {
                url: settings.url,
                signing_secret: settings.secret_name,
            },
            kinds: settings.kinds,
            limits: settings.limits,
            status: settings.status,
            health: Health::default(),
            created_by: principal.subject().as_str().to_owned(),
            created_at: now,
            updated_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::WebhookSubscriptions,
            )
            .await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::WebhookSubscriptionCreated,
            ResourceType::WebhookSubscription,
            subscription.id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!("subscription '{}'", subscription.name));
        self.store
            .create_subscription(&subscription, limit, &audit)
            .await?;
        Ok(subscription)
    }

    /// Replaces the settings of a subscription. Turning a disabled one `Active` is how the
    /// administrator says the receiver is back: its failures are forgotten and what was set aside as
    /// dead is delivered again.
    ///
    /// # Errors
    /// As [`Self::create`], and [`AppError::NotFound`] for an unknown subscription.
    pub async fn update(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        settings: SubscriptionSettings,
        ctx: &RequestContext,
    ) -> Result<SubscriptionRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        let mut subscription = self.load(principal, id).await?;
        self.check(principal, subscription.namespace_id, &settings, ctx)
            .await?;
        let now = self.clock.now();
        let was_off = subscription.status != SubscriptionStatus::Active;
        subscription.destination = OutboundDestination::Webhook {
            url: settings.url,
            signing_secret: settings.secret_name,
        };
        subscription.kinds = settings.kinds;
        subscription.limits = settings.limits;
        subscription.status = settings.status;
        subscription.updated_at = now;
        let audit = AuditRecord::by(
            principal,
            AuditAction::WebhookSubscriptionChanged,
            ResourceType::WebhookSubscription,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "subscription '{}' is {}{}",
            subscription.name,
            subscription.status.as_str().to_lowercase(),
            if was_off && settings.status == SubscriptionStatus::Active {
                ", turned back on"
            } else {
                ""
            }
        ));
        if self
            .store
            .update_subscription(&subscription, &audit)
            .await?
            .is_none()
        {
            return Err(AppError::NotFound);
        }
        // What the store keeps (the health a turn-on resets) is what is shown.
        self.load(principal, id).await
    }

    /// Removes a subscription, and with it what was queued for it.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn delete(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::EventManage)?;
        let subscription = self.load(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::WebhookSubscriptionDeleted,
            ResourceType::WebhookSubscription,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("subscription '{}'", subscription.name));
        if self
            .store
            .delete_subscription(principal.tenant_id(), id, &audit)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// # Errors
    /// [`AppError::Forbidden`] without `event:read`; [`AppError::NotFound`].
    pub async fn get(
        &self,
        principal: &Principal,
        id: SubscriptionId,
    ) -> Result<SubscriptionRecord, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load(principal, id).await
    }

    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn list(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<SubscriptionRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .subscriptions(principal.tenant_id(), namespace, false)
            .await?)
    }

    /// Deliveries of a subscription, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn deliveries(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        status: Option<DeliveryStatus>,
        before: Option<DeliveryId>,
        limit: u32,
    ) -> Result<Vec<DeliveryRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load(principal, id).await?;
        Ok(self
            .store
            .deliveries(
                principal.tenant_id(),
                id,
                status,
                before,
                limit.clamp(1, 200),
            )
            .await?)
    }

    /// One delivery with all its attempts.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn delivery(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        delivery: DeliveryId,
    ) -> Result<(DeliveryRecord, Vec<AttemptRecord>), AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load(principal, id).await?;
        self.store
            .delivery(principal.tenant_id(), id, delivery)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Puts dead deliveries back in the queue: one, or up to `limit` (at most 1000) of the
    /// subscription's, oldest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:manage`; [`AppError::NotFound`].
    pub async fn redrive(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        delivery: Option<DeliveryId>,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<u64, AppError> {
        principal.authorize(Permission::EventManage)?;
        let subscription = self.load(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::WebhookDeliveryRedriven,
            ResourceType::WebhookSubscription,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(match delivery {
            Some(one) => format!("delivery {one} of '{}'", subscription.name),
            None => format!("dead deliveries of '{}'", subscription.name),
        });
        Ok(self
            .store
            .redrive(
                principal.tenant_id(),
                id,
                delivery,
                limit.clamp(1, 1000),
                &audit,
            )
            .await?)
    }

    /// Creates a subscription that publishes to a tenant-operated broker.
    pub async fn create_broker(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        name: Slug,
        settings: BrokerSubscriptionSettings,
        ctx: &RequestContext,
    ) -> Result<SubscriptionRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        self.check_broker(principal, namespace, &settings, ctx)
            .await?;
        let now = self.clock.now();
        let subscription = SubscriptionRecord {
            id: SubscriptionId::generate(),
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            name,
            destination: OutboundDestination::Broker {
                settings: settings.destination,
                credentials_secret: settings.credentials_secret,
            },
            kinds: settings.kinds,
            limits: settings.limits,
            status: settings.status,
            health: Health::default(),
            created_by: principal.subject().as_str().to_owned(),
            created_at: now,
            updated_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::BrokerSubscriptions,
            )
            .await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::BrokerSubscriptionCreated,
            ResourceType::BrokerSubscription,
            subscription.id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!("broker subscription '{}'", subscription.name));
        match self
            .store
            .create_subscription(&subscription, limit, &audit)
            .await
        {
            Err(StoreError::Conflict(ConflictKind::WebhookSubscriptionName)) => {
                Err(AppError::Conflict(ConflictKind::BrokerSubscriptionName))
            }
            Err(error) => Err(error.into()),
            Ok(()) => Ok(subscription),
        }
    }

    /// Replaces the settings of an outbound broker subscription; its broker kind does not change.
    pub async fn update_broker(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        settings: BrokerSubscriptionSettings,
        ctx: &RequestContext,
    ) -> Result<SubscriptionRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        let mut subscription = self.load_broker(principal, id).await?;
        let OutboundDestination::Broker {
            settings: current, ..
        } = &subscription.destination
        else {
            return Err(AppError::NotFound);
        };
        if current.kind() != settings.destination.kind() {
            return Err(AppError::NotAllowed(
                "the kind of a broker subscription does not change; make another",
            ));
        }
        self.check_broker(principal, subscription.namespace_id, &settings, ctx)
            .await?;
        let now = self.clock.now();
        let was_off = subscription.status != SubscriptionStatus::Active;
        subscription.destination = OutboundDestination::Broker {
            settings: settings.destination,
            credentials_secret: settings.credentials_secret,
        };
        subscription.kinds = settings.kinds;
        subscription.limits = settings.limits;
        subscription.status = settings.status;
        subscription.updated_at = now;
        let audit = AuditRecord::by(
            principal,
            AuditAction::BrokerSubscriptionChanged,
            ResourceType::BrokerSubscription,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "broker subscription '{}' is {}{}",
            subscription.name,
            subscription.status.as_str().to_lowercase(),
            if was_off && subscription.status == SubscriptionStatus::Active {
                ", turned back on"
            } else {
                ""
            }
        ));
        if self
            .store
            .update_subscription(&subscription, &audit)
            .await?
            .is_none()
        {
            return Err(AppError::NotFound);
        }
        self.load_broker(principal, id).await
    }

    pub async fn get_broker(
        &self,
        principal: &Principal,
        id: SubscriptionId,
    ) -> Result<SubscriptionRecord, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load_broker(principal, id).await
    }

    pub async fn list_brokers(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        ctx: &RequestContext,
    ) -> Result<Vec<SubscriptionRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .subscriptions(principal.tenant_id(), namespace, true)
            .await?)
    }

    pub async fn delete_broker(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::EventManage)?;
        let subscription = self.load_broker(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::BrokerSubscriptionDeleted,
            ResourceType::BrokerSubscription,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("broker subscription '{}'", subscription.name));
        if self
            .store
            .delete_subscription(principal.tenant_id(), id, &audit)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    pub async fn broker_deliveries(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        status: Option<DeliveryStatus>,
        before: Option<DeliveryId>,
        limit: u32,
    ) -> Result<Vec<DeliveryRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load_broker(principal, id).await?;
        Ok(self
            .store
            .deliveries(
                principal.tenant_id(),
                id,
                status,
                before,
                limit.clamp(1, 200),
            )
            .await?)
    }

    pub async fn broker_delivery(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        delivery: DeliveryId,
    ) -> Result<(DeliveryRecord, Vec<AttemptRecord>), AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load_broker(principal, id).await?;
        self.store
            .delivery(principal.tenant_id(), id, delivery)
            .await?
            .ok_or(AppError::NotFound)
    }

    pub async fn redrive_broker(
        &self,
        principal: &Principal,
        id: SubscriptionId,
        delivery: Option<DeliveryId>,
        limit: u32,
        ctx: &RequestContext,
    ) -> Result<u64, AppError> {
        principal.authorize(Permission::EventManage)?;
        let subscription = self.load_broker(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::BrokerDeliveryRedriven,
            ResourceType::BrokerSubscription,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(match delivery {
            Some(one) => format!("delivery {one} of '{}'", subscription.name),
            None => format!("dead deliveries of '{}'", subscription.name),
        });
        Ok(self
            .store
            .redrive(
                principal.tenant_id(),
                id,
                delivery,
                limit.clamp(1, 1000),
                &audit,
            )
            .await?)
    }

    async fn load_broker(
        &self,
        principal: &Principal,
        id: SubscriptionId,
    ) -> Result<SubscriptionRecord, AppError> {
        let found = self
            .store
            .subscription(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::NotFound)?;
        if matches!(found.destination, OutboundDestination::Broker { .. }) {
            Ok(found)
        } else {
            Err(AppError::NotFound)
        }
    }

    async fn check_broker(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        settings: &BrokerSubscriptionSettings,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        check_kinds(&settings.kinds).map_err(|_| {
            AppError::NotAllowed("a subscription names between 1 and 8 different event types")
        })?;
        if settings.status == SubscriptionStatus::Disabled {
            return Err(AppError::NotAllowed(
                "a subscription is switched off by the platform, never by a caller",
            ));
        }
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        if matches!(settings.destination, BrokerDestination::Redis { .. }) && !self.redis_callbacks
        {
            return Err(AppError::NotAllowed(
                "Redis callbacks require the completed rollout activation",
            ));
        }
        if let BrokerDestination::Redis { server, .. } = &settings.destination
            && settings.credentials_secret.is_some()
            && !server.tls()
        {
            return Err(AppError::NotAllowed(
                "Redis credentials require a TLS destination",
            ));
        }
        for server in settings.destination.servers() {
            if self
                .egress
                .check(principal.tenant_id(), namespace, &server.destination())
                .await?
                .is_err()
            {
                return Err(AppError::NotAllowed(
                    "the egress policy of the namespace does not allow that broker",
                ));
            }
        }
        if let Some(secret) = &settings.credentials_secret {
            let states = self
                .store
                .secret_states(
                    principal.tenant_id(),
                    namespace,
                    std::slice::from_ref(secret),
                    principal.subject().as_str(),
                )
                .await?;
            check_usable(Some(principal.role()), &states).map_err(|SecretProblem(name)| {
                AppError::SecretUnavailable {
                    name: name.to_string(),
                }
            })?;
        }
        Ok(())
    }

    async fn load(
        &self,
        principal: &Principal,
        id: SubscriptionId,
    ) -> Result<SubscriptionRecord, AppError> {
        let found = self
            .store
            .subscription(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::NotFound)?;
        if matches!(found.destination, OutboundDestination::Webhook { .. }) {
            Ok(found)
        } else {
            Err(AppError::NotFound)
        }
    }

    /// What every create and update has to satisfy.
    async fn check(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        settings: &SubscriptionSettings,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        check_kinds(&settings.kinds).map_err(|_| {
            AppError::NotAllowed("a subscription names between 1 and 8 different event types")
        })?;
        if settings.status == SubscriptionStatus::Disabled {
            return Err(AppError::NotAllowed(
                "a subscription is switched off by the platform, never by a caller",
            ));
        }
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        // The administrator learns now, and not from a dead delivery tomorrow, that the namespace
        // may not reach that address. (The policy is enforced again for every attempt.)
        match self
            .egress
            .check(
                principal.tenant_id(),
                namespace,
                &settings.url.destination(),
            )
            .await?
        {
            Ok(()) => {}
            Err(_) => {
                return Err(AppError::NotAllowed(
                    "the egress policy of the namespace does not allow that destination",
                ));
            }
        }
        let states = self
            .store
            .secret_states(
                principal.tenant_id(),
                namespace,
                std::slice::from_ref(&settings.secret_name),
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

// ----------------------------------------------------------------------------- the delivery worker

/// A delivery that is due, with what is needed to make it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueDelivery {
    pub delivery: DeliveryRecord,
    pub destination: OutboundDestination,
    pub limits: DeliveryLimits,
}

impl DueDelivery {
    #[must_use]
    pub const fn webhook(&self) -> Option<(&TargetUrl, &SecretName)> {
        match &self.destination {
            OutboundDestination::Webhook {
                url,
                signing_secret,
            } => Some((url, signing_secret)),
            OutboundDestination::Broker { .. } => None,
        }
    }
}

/// What the worker decided after an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Delivered,
    /// Another attempt after `delay`.
    Retry {
        delay: Duration,
    },
    /// Set aside.
    Dead,
}

/// Storage port of the delivery worker. Implemented by `sagat-infra-postgres`.
pub trait DeliveryStore: Send + Sync {
    /// Takes deliveries that are due, keeping them for `lease`: another worker does not see them
    /// meanwhile. Only those of active subscriptions are taken, and the rate of each subscription is
    /// enforced here: a claim takes the subscription's next slot, so across replicas a subscription
    /// is offered at most `rate_per_second` deliveries a second (in small bursts). At most `limit`
    /// subscriptions are served by one claim.
    fn claim_deliveries(
        &self,
        limit: u32,
        lease: Duration,
    ) -> impl Future<Output = Result<Vec<DueDelivery>, StoreError>> + Send;

    /// Records an attempt and what comes of it, in one transaction: the attempt, the new state of
    /// the delivery, the health of the subscription and, when the subscription is switched off by
    /// it, the audit record that says so. `true` when the subscription was switched off.
    fn record_attempt(
        &self,
        due: &DueDelivery,
        attempt: &AttemptRecord,
        next: &Next,
        switched_off: Option<&AuditRecord>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// The newest `limit` versions of the signing secret, newest first.
    fn signing_secrets(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StoredSecret>, StoreError>> + Send;

    /// Removes delivered and dead deliveries older than `retention` (and their attempts), at most
    /// `limit`.
    fn purge_deliveries(
        &self,
        retention: Duration,
        limit: u32,
    ) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Deliveries waiting to be made (for the backlog gauge).
    fn pending_deliveries(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// A request to a subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundRequest {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub url: TargetUrl,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

/// What a subscriber answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundResponse {
    pub status: u16,
    pub retry_after: Option<Duration>,
}

/// Why no answer came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFailure {
    /// The egress policy of the namespace refused the destination.
    Denied,
    Timeout,
    /// The connection failed or broke, or the name did not resolve.
    Network,
    /// The URL cannot be a destination at all.
    Invalid,
}

/// Sends a request through the guarded egress client of the namespace. Implemented by
/// `sagat-infra-connectors`; a test implements it over a local server.
pub trait WebhookSender: Send + Sync {
    fn send(
        &self,
        request: &OutboundRequest,
    ) -> impl Future<Output = Result<OutboundResponse, SendFailure>> + Send;
}

/// A broker publish request. The credentials value is redacted and zeroized by its domain type.
#[derive(Clone)]
pub struct BrokerRequest {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    /// Stable FIFO ordering group for destinations that support ordered delivery.
    pub subscription_id: SubscriptionId,
    pub destination: BrokerDestination,
    pub credentials: Option<SecretValue>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

impl std::fmt::Debug for BrokerRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerRequest")
            .field("tenant_id", &self.tenant_id)
            .field("namespace_id", &self.namespace_id)
            .field("subscription_id", &self.subscription_id)
            .field("destination", &self.destination)
            .field(
                "credentials",
                &self.credentials.as_ref().map(|_| "<redacted>"),
            )
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerFailure {
    Denied,
    Timeout,
    Network,
    Rejected,
}

pub trait BrokerPublisher: Send + Sync {
    fn publish(
        &self,
        request: &BrokerRequest,
    ) -> impl Future<Output = Result<(), BrokerFailure>> + Send;
}

/// Safe default used by tests and instances that have no outbound broker publisher.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBrokerPublisher;

impl BrokerPublisher for NoBrokerPublisher {
    async fn publish(&self, _: &BrokerRequest) -> Result<(), BrokerFailure> {
        Err(BrokerFailure::Network)
    }
}

/// Maps a conflict raised by the store.
#[must_use]
pub const fn subscription_name_conflict() -> AppError {
    AppError::Conflict(ConflictKind::WebhookSubscriptionName)
}
