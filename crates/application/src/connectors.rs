//! Connectors to the brokers of a tenant: what is read from them becomes events (`FR-004-005`,
//! `FR-004-008`, US03).
//!
//! The rule that matters is the order of two steps. A message is acknowledged to its broker (offset
//! committed, message deleted, ack sent) only after the event it became is in the inbox, and for a
//! broker whose position the platform keeps itself (Kafka) the position moves in the same
//! transaction that admits the event. A crash can then repeat a message but never lose one, and the
//! inbox deduplicates the repeat. A message that cannot become an event (no type, not a JSON
//! object, too large) is not retried for ever: it is set aside, with the reason and what it
//! carried, where an operator can look at it, and the connector moves on.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::connectors::{
    ConnectorId, ConnectorSettings, ConnectorStatus, DEFAULT_INGEST_RATE, DeadLetter, check_rate,
    is_reserved,
};
use sagat_domain::events::{DEFAULT_MAX_BODY, EventEnvelope, EventId, EventKey, MAX_SUBJECT_LEN};
use sagat_domain::execution::CorrelationKey;
use sagat_domain::identity::{NamespaceId, Permission, Principal, Slug, TenantId};
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::authz::{NamespaceDirectory, require_namespace};
use crate::clock::Clock;
use crate::egress::{EgressService, EgressStore};
use crate::error::{AppError, StoreError};
use crate::events::{Admission, NewEvent};
use crate::execution::ExpressionEvaluator;
use crate::identity::RequestContext;
use crate::quotas::QuotaLookup;
use crate::secrets::{SecretAccessLookup, SecretProblem, StoredSecret, check_usable};

/// What came of setting a message aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetAside {
    /// It is a dead letter now.
    Kept,
    /// It already was one (the broker delivered it again): nothing was added.
    AlreadyKept,
    /// The connector already keeps [`MAX_DEAD_LETTERS`] of them, the first ones, and this one was
    /// not kept: the broker is told it is done with it, and it is counted. Somebody has to look at
    /// the ones that are kept (or discard them) before more are.
    Full,
}

/// Dead letters kept per connector; the oldest are dropped beyond this.
pub const MAX_DEAD_LETTERS: u64 = 1_000;
/// Longest body kept with a dead letter (more is cut, and the record says so).
pub const MAX_DEAD_BODY: usize = 64 * 1024;
/// Longest header value kept with a dead letter.
const MAX_HEADER_VALUE: usize = 256;
/// Headers kept with a dead letter.
const MAX_HEADERS: usize = 16;

/// What a connector has shown about itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectorHealth {
    /// The class of the last failure to read the broker; cleared by the next successful read.
    pub last_error: Option<String>,
    pub last_message_at: Option<OffsetDateTime>,
    /// Messages turned into events (including repeats the inbox recognised).
    pub consumed: u64,
    pub dead_lettered: u64,
}

/// A connector as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorRecord {
    pub id: ConnectorId,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    /// The source of the events it admits.
    pub slug: Slug,
    pub settings: ConnectorSettings,
    /// Credentials for the broker, a secret of the namespace; absent for a broker that needs none.
    pub secret_name: Option<SecretName>,
    /// The type of a message that does not carry one.
    pub default_event_type: Option<EventType>,
    pub rate_per_second: u32,
    pub status: ConnectorStatus,
    pub health: ConnectorHealth,
    pub created_by: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// What an administrator chooses about a connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorChoices {
    pub settings: ConnectorSettings,
    pub secret_name: Option<SecretName>,
    pub default_event_type: Option<EventType>,
    pub rate_per_second: Option<u32>,
    pub status: ConnectorStatus,
}

/// A message set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterRecord {
    pub id: Uuid,
    pub connector_id: ConnectorId,
    /// Where it was: `topic/partition/offset`, a stream sequence, a message id.
    pub position: String,
    pub reason: DeadLetter,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    /// The body was cut to [`MAX_DEAD_BODY`].
    pub truncated: bool,
    pub received_at: OffsetDateTime,
}

/// A message as a broker gave it, before anyone has judged it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
    /// Where it was read from, for the dead-letter record.
    pub position: String,
    /// The id of the event when the message does not carry one: the broker's own identity of it
    /// (`topic/partition/offset`, a message id), stable across redeliveries.
    pub fallback_id: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

/// A message that is an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub envelope: EventEnvelope,
    pub occurred_at: Option<OffsetDateTime>,
    pub body: Vec<u8>,
}

/// Looks a header up without regard to case.
fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

/// Turns a message into an event, or says why it cannot be one.
///
/// # Errors
/// The [`DeadLetter`] reason.
pub fn decode(
    connector: &ConnectorRecord,
    message: &RawMessage,
    evaluator: &dyn ExpressionEvaluator,
) -> Result<Decoded, DeadLetter> {
    if message.body.len() > DEFAULT_MAX_BODY {
        return Err(DeadLetter::TooLarge);
    }
    if !evaluator.is_json_object(&message.body) {
        return Err(DeadLetter::NotAnObject);
    }
    let event_type = match header(&message.headers, "eventType") {
        Some(text) => EventType::parse(text).map_err(|_| DeadLetter::BadEnvelope)?,
        None => connector
            .default_event_type
            .clone()
            .ok_or(DeadLetter::NoType)?,
    };
    let event_id =
        EventId::parse(header(&message.headers, "eventId").unwrap_or(&message.fallback_id))
            .map_err(|_| DeadLetter::BadEnvelope)?;
    let subject = header(&message.headers, "subject");
    if subject.is_some_and(|s| s.len() > MAX_SUBJECT_LEN) {
        return Err(DeadLetter::BadEnvelope);
    }
    let correlation = header(&message.headers, "correlationKey")
        .map(CorrelationKey::parse)
        .transpose()
        .map_err(|_| DeadLetter::BadEnvelope)?;
    let envelope = EventEnvelope::new(
        EventKey {
            source: connector.slug.clone(),
            event_id,
        },
        event_type,
        subject,
        correlation,
    )
    .map_err(|_| DeadLetter::BadEnvelope)?;
    let occurred_at = header(&message.headers, "occurredAt").and_then(|t| {
        OffsetDateTime::parse(t, &time::format_description::well_known::Rfc3339).ok()
    });
    Ok(Decoded {
        envelope,
        occurred_at,
        body: message.body.clone(),
    })
}

/// The headers worth keeping with a dead letter: a few, short, and only text.
#[must_use]
pub fn keep_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .take(MAX_HEADERS)
        .map(|(k, v)| {
            (
                k.chars().take(64).collect(),
                v.chars().take(MAX_HEADER_VALUE).collect(),
            )
        })
        .collect()
}

/// Where a broker's position is kept by the platform, and what it becomes once a message is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// `topic/partition`.
    pub stream: String,
    /// The offset the consumer reads next; stored in the transaction that admits the message.
    pub next_offset: i64,
}

/// What a consumer reports about a stretch of reading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    pub consumed: u64,
    pub dead_lettered: u64,
    pub last_message_at: Option<OffsetDateTime>,
    /// `Some(None)` clears the last error, `Some(Some(class))` sets it, `None` leaves it.
    pub error: Option<Option<String>>,
}

/// Storage port of the connectors. Implemented by `sagat-infra-postgres`.
pub trait ConnectorStore: NamespaceDirectory + QuotaLookup + SecretAccessLookup {
    /// Stores a new connector with its audit record, counting it against the namespace's quota. The
    /// slug is taken by one source of events per namespace, an endpoint or a connector:
    /// `Conflict(ConnectorSlug)` when it is not free.
    fn create_connector(
        &self,
        connector: &ConnectorRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Replaces the choices of a connector. `false` when it does not exist.
    fn update_connector(
        &self,
        connector: &ConnectorRecord,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Removes a connector with its offsets and dead letters. `false` when it did not exist.
    fn delete_connector(
        &self,
        tenant: TenantId,
        id: ConnectorId,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    fn connector(
        &self,
        tenant: TenantId,
        id: ConnectorId,
    ) -> impl Future<Output = Result<Option<ConnectorRecord>, StoreError>> + Send;

    fn connectors(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> impl Future<Output = Result<Vec<ConnectorRecord>, StoreError>> + Send;

    /// Dead letters of a connector, newest first, with `id < before` when given.
    fn dead_letters(
        &self,
        tenant: TenantId,
        connector: ConnectorId,
        before: Option<Uuid>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<DeadLetterRecord>, StoreError>> + Send;

    fn dead_letter(
        &self,
        tenant: TenantId,
        connector: ConnectorId,
        id: Uuid,
    ) -> impl Future<Output = Result<Option<DeadLetterRecord>, StoreError>> + Send;

    /// Forgets a dead letter. `false` when it did not exist.
    fn discard_dead_letter(
        &self,
        tenant: TenantId,
        connector: ConnectorId,
        id: Uuid,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Admits the decoded content of a dead letter and forgets it, in one transaction, when the
    /// event is admitted or was admitted before.
    fn admit_dead_letter(
        &self,
        connector: &ConnectorRecord,
        dead_letter: Uuid,
        event: &NewEvent,
        backlog_limit: u64,
        audit: &AuditRecord,
    ) -> impl Future<Output = Result<Admission, StoreError>> + Send;
}

/// Storage port of the consumers. Implemented by `sagat-infra-postgres`.
pub trait ConnectorRuntime: Send + Sync {
    /// Every connector that is to be read, in every tenant.
    fn active_connectors(
        &self,
    ) -> impl Future<Output = Result<Vec<ConnectorRecord>, StoreError>> + Send;

    /// The newest version of the credentials secret of a connector, still sealed.
    fn credentials(
        &self,
        connector: &ConnectorRecord,
    ) -> impl Future<Output = Result<Option<StoredSecret>, StoreError>> + Send;

    /// Admits the event and, in the same transaction, moves the checkpoint (if the broker's position
    /// is kept here). `expected` is the offset the checkpoint must be at: another consumer that
    /// moved it first makes this a [`StoreError::RevisionConflict`], and nothing is written.
    fn admit(
        &self,
        connector: &ConnectorRecord,
        event: &NewEvent,
        backlog_limit: u64,
        checkpoint: Option<(&Checkpoint, i64)>,
    ) -> impl Future<Output = Result<Admission, StoreError>> + Send;

    /// Sets a message aside and, in the same transaction, moves the checkpoint.
    fn set_aside(
        &self,
        connector: &ConnectorRecord,
        message: &RawMessage,
        reason: DeadLetter,
        checkpoint: Option<(&Checkpoint, i64)>,
    ) -> impl Future<Output = Result<SetAside, StoreError>> + Send;

    /// Where a stream's consumer reads next, if it has read before.
    fn checkpoint(
        &self,
        connector: ConnectorId,
        stream: &str,
    ) -> impl Future<Output = Result<Option<i64>, StoreError>> + Send;

    /// Takes or renews the lease on a stream for `holder`: only the holder reads it. `true` when
    /// `holder` has it.
    fn lease(
        &self,
        connector: ConnectorId,
        stream: &str,
        holder: &str,
        ttl: Duration,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Records what reading has shown.
    fn progress(
        &self,
        connector: ConnectorId,
        progress: &Progress,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// Administers connectors and their dead letters.
pub struct ConnectorService<S, C> {
    store: S,
    clock: C,
    egress: EgressService<S, C>,
    evaluator: std::sync::Arc<dyn ExpressionEvaluator>,
}

impl<S, C> ConnectorService<S, C>
where
    S: ConnectorStore + EgressStore,
    C: Clock,
{
    /// `egress` must be over the same store (it answers whether the namespace may reach a broker).
    #[must_use]
    pub fn new(
        store: S,
        clock: C,
        egress: EgressService<S, C>,
        evaluator: std::sync::Arc<dyn ExpressionEvaluator>,
    ) -> Self {
        Self {
            store,
            clock,
            egress,
            evaluator,
        }
    }

    /// Creates a connector.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:manage`; [`AppError::NotFound`];
    /// [`AppError::NotAllowed`] for a reserved slug, a rate out of range or a broker the egress
    /// policy of the namespace does not allow; [`AppError::SecretUnavailable`];
    /// [`AppError::Conflict`]; [`AppError::QuotaExceeded`].
    pub async fn create(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        slug: Slug,
        choices: ConnectorChoices,
        ctx: &RequestContext,
    ) -> Result<ConnectorRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        if is_reserved(&slug) {
            return Err(AppError::NotAllowed(
                "that slug is reserved for events that do not come through a connector",
            ));
        }
        let rate = self.check(principal, namespace, &choices, ctx).await?;
        let now = self.clock.now();
        let connector = ConnectorRecord {
            id: ConnectorId::generate(),
            tenant_id: principal.tenant_id(),
            namespace_id: namespace,
            slug,
            settings: choices.settings,
            secret_name: choices.secret_name,
            default_event_type: choices.default_event_type,
            rate_per_second: rate,
            status: choices.status,
            health: ConnectorHealth::default(),
            created_by: principal.subject().as_str().to_owned(),
            created_at: now,
            updated_at: now,
        };
        let limit = self
            .store
            .effective_limit(
                principal.tenant_id(),
                Some(namespace),
                QuotaResource::Connectors,
            )
            .await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::ConnectorCreated,
            ResourceType::Connector,
            connector.id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "{} connector '{}'",
            connector.settings.kind(),
            connector.slug
        ));
        self.store
            .create_connector(&connector, limit, &audit)
            .await?;
        Ok(connector)
    }

    /// Replaces the choices of a connector (its slug and kind never change).
    ///
    /// # Errors
    /// As [`Self::create`], and [`AppError::NotFound`].
    pub async fn update(
        &self,
        principal: &Principal,
        id: ConnectorId,
        choices: ConnectorChoices,
        ctx: &RequestContext,
    ) -> Result<ConnectorRecord, AppError> {
        principal.authorize(Permission::EventManage)?;
        let mut connector = self.load(principal, id).await?;
        if choices.settings.kind() != connector.settings.kind() {
            return Err(AppError::NotAllowed(
                "the kind of a connector does not change; make another",
            ));
        }
        let rate = self
            .check(principal, connector.namespace_id, &choices, ctx)
            .await?;
        let now = self.clock.now();
        connector.settings = choices.settings;
        connector.secret_name = choices.secret_name;
        connector.default_event_type = choices.default_event_type;
        connector.rate_per_second = rate;
        connector.status = choices.status;
        connector.updated_at = now;
        let audit = AuditRecord::by(
            principal,
            AuditAction::ConnectorChanged,
            ResourceType::Connector,
            id.to_string(),
            ctx.trace_id.clone(),
            now,
        )
        .with_reason(format!(
            "connector '{}' is {}",
            connector.slug,
            connector.status.as_str().to_lowercase()
        ));
        if !self.store.update_connector(&connector, &audit).await? {
            return Err(AppError::NotFound);
        }
        self.load(principal, id).await
    }

    /// Removes a connector, its checkpoints and its dead letters. The events it admitted stay until
    /// their window ends.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn delete(
        &self,
        principal: &Principal,
        id: ConnectorId,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::EventManage)?;
        let connector = self.load(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::ConnectorDeleted,
            ResourceType::Connector,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("connector '{}'", connector.slug));
        if self
            .store
            .delete_connector(principal.tenant_id(), id, &audit)
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
        id: ConnectorId,
    ) -> Result<ConnectorRecord, AppError> {
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
    ) -> Result<Vec<ConnectorRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        Ok(self
            .store
            .connectors(principal.tenant_id(), namespace)
            .await?)
    }

    /// Dead letters of a connector, newest first.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn dead_letters(
        &self,
        principal: &Principal,
        id: ConnectorId,
        before: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<DeadLetterRecord>, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load(principal, id).await?;
        Ok(self
            .store
            .dead_letters(principal.tenant_id(), id, before, limit.clamp(1, 200))
            .await?)
    }

    /// One dead letter.
    ///
    /// # Errors
    /// [`AppError::Forbidden`]; [`AppError::NotFound`].
    pub async fn dead_letter(
        &self,
        principal: &Principal,
        id: ConnectorId,
        letter: Uuid,
    ) -> Result<DeadLetterRecord, AppError> {
        principal.authorize(Permission::EventRead)?;
        self.load(principal, id).await?;
        self.store
            .dead_letter(principal.tenant_id(), id, letter)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// Forgets a dead letter.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:manage`; [`AppError::NotFound`].
    pub async fn discard(
        &self,
        principal: &Principal,
        id: ConnectorId,
        letter: Uuid,
        ctx: &RequestContext,
    ) -> Result<(), AppError> {
        principal.authorize(Permission::EventManage)?;
        self.load(principal, id).await?;
        let audit = AuditRecord::by(
            principal,
            AuditAction::ConnectorMessageDiscarded,
            ResourceType::Connector,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("dead letter {letter}"));
        if self
            .store
            .discard_dead_letter(principal.tenant_id(), id, letter, &audit)
            .await?
        {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }

    /// Tries a dead letter again, with what the connector says now (a default type it did not have
    /// when the message came). Admitted, or already admitted before: the dead letter is gone and the
    /// event is returned. Still not an event: it stays, with the reason it fails for now.
    ///
    /// # Errors
    /// [`AppError::Forbidden`] without `event:manage`; [`AppError::NotFound`];
    /// [`AppError::NotAllowed`] naming why the message still cannot become an event;
    /// [`AppError::Busy`] when the tenant's backlog is full.
    pub async fn redrive(
        &self,
        principal: &Principal,
        id: ConnectorId,
        letter: Uuid,
        ctx: &RequestContext,
    ) -> Result<EventKey, AppError> {
        principal.authorize(Permission::EventManage)?;
        let connector = self.load(principal, id).await?;
        let stored = self
            .store
            .dead_letter(principal.tenant_id(), id, letter)
            .await?
            .ok_or(AppError::NotFound)?;
        if stored.truncated {
            return Err(AppError::NotAllowed(
                "the message was cut when it was set aside, so it cannot be tried again",
            ));
        }
        let raw = RawMessage {
            position: stored.position.clone(),
            fallback_id: stored.position.clone(),
            headers: stored.headers.clone(),
            body: stored.body.clone(),
        };
        let decoded = decode(&connector, &raw, self.evaluator.as_ref()).map_err(|reason| {
            AppError::NotAllowed(match reason {
                DeadLetter::NoType => {
                    "the message names no event type and the connector has no default"
                }
                DeadLetter::BadEnvelope => {
                    "the message's id, type, subject or correlation key is not acceptable"
                }
                DeadLetter::NotAnObject => "the message is not a JSON object",
                DeadLetter::TooLarge => "the message is larger than an event may be",
                DeadLetter::IdReused => "the id was used before for a different message",
            })
        })?;
        let key = decoded.envelope.key.clone();
        let mut event = crate::events::new_event(
            principal.tenant_id(),
            connector.namespace_id,
            decoded.envelope,
            decoded.occurred_at,
            decoded.body,
            sagat_domain::events::DEFAULT_DEDUPE_TTL,
        );
        event.trace = ctx.link();
        let audit = AuditRecord::by(
            principal,
            AuditAction::ConnectorMessageRedriven,
            ResourceType::Connector,
            id.to_string(),
            ctx.trace_id.clone(),
            self.clock.now(),
        )
        .with_reason(format!("dead letter {letter}"));
        match self
            .store
            .admit_dead_letter(
                &connector,
                letter,
                &event,
                crate::events::BACKLOG_LIMIT,
                &audit,
            )
            .await?
        {
            Admission::Accepted | Admission::Duplicate => Ok(key),
            Admission::Reused => Err(AppError::Conflict(
                crate::error::ConflictKind::EventIdReused,
            )),
            Admission::Backlogged => Err(AppError::Busy {
                retry_after_secs: crate::events::BACKLOG_RETRY_AFTER_SECS,
            }),
        }
    }

    async fn load(
        &self,
        principal: &Principal,
        id: ConnectorId,
    ) -> Result<ConnectorRecord, AppError> {
        self.store
            .connector(principal.tenant_id(), id)
            .await?
            .ok_or(AppError::NotFound)
    }

    /// What every create and update has to satisfy; returns the rate to use.
    async fn check(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        choices: &ConnectorChoices,
        ctx: &RequestContext,
    ) -> Result<u32, AppError> {
        let rate =
            check_rate(choices.rate_per_second.unwrap_or(DEFAULT_INGEST_RATE)).map_err(|_| {
                AppError::NotAllowed("the ingest rate is between 1 and 10000 messages a second")
            })?;
        require_namespace(&self.store, &self.clock, principal, namespace, ctx).await?;
        // The brokers are destinations like any other: the namespace may reach only what its egress
        // policy allows. A broker that speaks without encryption needs the policy to allow plain
        // connections, which an administrator decides on purpose.
        for server in choices.settings.servers() {
            if self
                .egress
                .check(principal.tenant_id(), namespace, &server.destination())
                .await?
                .is_err()
            {
                return Err(AppError::NotAllowed(
                    "the egress policy of the namespace does not allow a broker of the connector",
                ));
            }
        }
        if let Some(secret) = &choices.secret_name {
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
        Ok(rate)
    }
}
