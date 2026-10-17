//! Value-safe reads of inbox events and connector dead letters (ADR-0033, US08).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::connectors::{ConnectorId, DeadLetter};
use sagat_domain::events::{EventEnvelope, EventKey, InboxStatus};
use sagat_domain::identity::{NamespaceId, Permission, Principal, Role, TenantId};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{AppError, StoreError};
use crate::events::Delivery;
use crate::identity::RequestContext;
use crate::payloads::{PayloadProjection, PayloadProjector, StoredPayloadBytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventReveal {
    Structure,
    Pii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventPayloadScope {
    pub namespace_id: NamespaceId,
}

#[derive(Debug, Clone)]
pub struct PreparedInboxEvent {
    principal: Principal,
    namespace_id: NamespaceId,
    key: EventKey,
    reveal: EventReveal,
    trace_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedDeadLetter {
    principal: Principal,
    namespace_id: NamespaceId,
    connector_id: ConnectorId,
    letter_id: Option<Uuid>,
    reveal: EventReveal,
    trace_id: Option<String>,
}

macro_rules! prepared_accessors {
    ($name:ty) => {
        impl $name {
            #[must_use]
            pub const fn tenant_id(&self) -> TenantId {
                self.principal.tenant_id()
            }
            #[must_use]
            pub const fn namespace_id(&self) -> NamespaceId {
                self.namespace_id
            }
            #[must_use]
            pub fn subject(&self) -> &sagat_domain::identity::Subject {
                self.principal.subject()
            }
            #[must_use]
            pub const fn reveal(&self) -> EventReveal {
                self.reveal
            }
        }
    };
}
prepared_accessors!(PreparedInboxEvent);
prepared_accessors!(PreparedDeadLetter);

impl PreparedInboxEvent {
    #[must_use]
    pub const fn key(&self) -> &EventKey {
        &self.key
    }
}
impl PreparedDeadLetter {
    #[must_use]
    pub const fn connector_id(&self) -> ConnectorId {
        self.connector_id
    }
    #[must_use]
    pub const fn letter_id(&self) -> Option<Uuid> {
        self.letter_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedInboxEvent {
    pub envelope: EventEnvelope,
    pub occurred_at: Option<OffsetDateTime>,
    pub payload: StoredPayloadBytes,
    pub received_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub status: InboxStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub dispatched_at: Option<OffsetDateTime>,
    pub deliveries: Vec<Delivery>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SelectedDeadLetter {
    pub id: Uuid,
    pub connector_id: ConnectorId,
    pub position: String,
    pub reason: DeadLetter,
    pub headers: BTreeMap<String, String>,
    pub body: StoredPayloadBytes,
    pub truncated: bool,
    pub received_at: OffsetDateTime,
}

impl std::fmt::Debug for SelectedDeadLetter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectedDeadLetter")
            .field("id", &self.id)
            .field("connector_id", &self.connector_id)
            .field("position", &self.position)
            .field("reason", &self.reason)
            .field("header_count", &self.headers.len())
            .field("body", &self.body)
            .field("truncated", &self.truncated)
            .field("received_at", &self.received_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventData {
    Structure(PayloadProjection),
    Revealed(StoredPayloadBytes),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxEventRead {
    pub event: SelectedInboxEvent,
    pub data_size_bytes: usize,
    pub data: EventData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadLetterStructure {
    Json(PayloadProjection),
    Opaque,
}

#[derive(Clone, PartialEq, Eq)]
pub enum DeadLetterData {
    Structure(DeadLetterStructure),
    Revealed {
        headers: BTreeMap<String, String>,
        body: StoredPayloadBytes,
    },
}
impl std::fmt::Debug for DeadLetterData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structure(value) => f.debug_tuple("Structure").field(value).finish(),
            Self::Revealed { headers, body } => f
                .debug_struct("Revealed")
                .field("header_count", &headers.len())
                .field("body", body)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterRead {
    pub letter: SelectedDeadLetter,
    pub body_size_bytes: usize,
    pub header_count: usize,
    pub data: DeadLetterData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPayloadFailure {
    EventReadNotGranted,
    PiiRevealNotGranted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPayloadRead<T> {
    Missing,
    Refused(EventPayloadFailure),
    Ready(T),
}

#[derive(Debug, Clone)]
pub struct EventPayloadAudits {
    pub revealed: AuditRecord,
    pub denied: AuditRecord,
}

pub trait EventPayloadEvaluator: Send + Sync {
    fn event(
        &self,
        selected: SelectedInboxEvent,
        role: Role,
        grant: bool,
    ) -> Result<InboxEventRead, EventPayloadFailure>;
    fn dead_letter(
        &self,
        selected: SelectedDeadLetter,
        role: Role,
        grant: bool,
    ) -> Result<DeadLetterRead, EventPayloadFailure>;
}

pub trait EventPayloadStore: Send + Sync {
    fn event_scope(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        key: &EventKey,
    ) -> impl Future<Output = Result<Option<EventPayloadScope>, StoreError>> + Send;
    fn dead_letter_scope(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        connector: ConnectorId,
        letter: Option<Uuid>,
    ) -> impl Future<Output = Result<Option<EventPayloadScope>, StoreError>> + Send;
    fn read_event(
        &self,
        prepared: &PreparedInboxEvent,
        evaluator: &dyn EventPayloadEvaluator,
        audits: &EventPayloadAudits,
    ) -> impl Future<Output = Result<EventPayloadRead<InboxEventRead>, StoreError>> + Send;
    fn read_dead_letter(
        &self,
        prepared: &PreparedDeadLetter,
        evaluator: &dyn EventPayloadEvaluator,
        audits: &EventPayloadAudits,
    ) -> impl Future<Output = Result<EventPayloadRead<DeadLetterRead>, StoreError>> + Send;
    fn list_dead_letters(
        &self,
        prepared: &PreparedDeadLetter,
        before: Option<Uuid>,
        limit: u32,
        evaluator: &dyn EventPayloadEvaluator,
    ) -> impl Future<Output = Result<EventPayloadRead<Vec<DeadLetterRead>>, StoreError>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventPayloadError {
    #[error(transparent)]
    Application(#[from] AppError),
    #[error("PII reveal is not granted in this namespace")]
    PiiRevealNotGranted,
}

pub type EventPrepareFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;
pub type EventReadFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, EventPayloadError>> + Send + 'a>>;

pub trait EventPayloadUseCase: Send + Sync {
    fn prepare_event<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        key: EventKey,
        reveal: EventReveal,
        ctx: &'a RequestContext,
    ) -> EventPrepareFuture<'a, PreparedInboxEvent>;
    fn prepare_dead_letter<'a>(
        &'a self,
        principal: &'a Principal,
        namespace: NamespaceId,
        connector: ConnectorId,
        letter: Option<Uuid>,
        reveal: EventReveal,
        ctx: &'a RequestContext,
    ) -> EventPrepareFuture<'a, PreparedDeadLetter>;
    fn event<'a>(&'a self, prepared: PreparedInboxEvent) -> EventReadFuture<'a, InboxEventRead>;
    fn dead_letter<'a>(
        &'a self,
        prepared: PreparedDeadLetter,
    ) -> EventReadFuture<'a, DeadLetterRead>;
    fn dead_letters<'a>(
        &'a self,
        prepared: PreparedDeadLetter,
        before: Option<Uuid>,
        limit: u32,
    ) -> EventReadFuture<'a, Vec<DeadLetterRead>>;
}

pub struct EventPayloadService<S, C, P> {
    store: S,
    clock: C,
    projector: P,
}
impl<S, C, P> EventPayloadService<S, C, P> {
    #[must_use]
    pub const fn new(store: S, clock: C, projector: P) -> Self {
        Self {
            store,
            clock,
            projector,
        }
    }
}

impl<S: EventPayloadStore, C: Clock, P: PayloadProjector> EventPayloadService<S, C, P> {
    async fn prepare_event_inner(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        key: EventKey,
        reveal: EventReveal,
        ctx: &RequestContext,
    ) -> Result<PreparedInboxEvent, AppError> {
        principal.authorize(Permission::EventRead)?;
        if self
            .store
            .event_scope(principal.tenant_id(), namespace, &key)
            .await?
            .is_none()
        {
            return Err(AppError::NotFound);
        }
        Ok(PreparedInboxEvent {
            principal: principal.clone(),
            namespace_id: namespace,
            key,
            reveal,
            trace_id: ctx.trace_id.clone(),
        })
    }
    async fn prepare_dead_letter_inner(
        &self,
        principal: &Principal,
        namespace: NamespaceId,
        connector: ConnectorId,
        letter: Option<Uuid>,
        reveal: EventReveal,
        ctx: &RequestContext,
    ) -> Result<PreparedDeadLetter, AppError> {
        principal.authorize(Permission::EventRead)?;
        if self
            .store
            .dead_letter_scope(principal.tenant_id(), namespace, connector, letter)
            .await?
            .is_none()
        {
            return Err(AppError::NotFound);
        }
        Ok(PreparedDeadLetter {
            principal: principal.clone(),
            namespace_id: namespace,
            connector_id: connector,
            letter_id: letter,
            reveal,
            trace_id: ctx.trace_id.clone(),
        })
    }
    fn audits_event(&self, p: &PreparedInboxEvent) -> EventPayloadAudits {
        let id = format!("{}/{}", p.key.source, p.key.event_id);
        let make = |action, reason: String| {
            AuditRecord::by(
                &p.principal,
                action,
                ResourceType::Event,
                id.clone(),
                p.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason(reason)
        };
        EventPayloadAudits {
            revealed: make(
                AuditAction::PayloadPiiRevealed,
                "event-data:reveal=pii".into(),
            ),
            denied: make(AuditAction::PayloadRevealDenied, "permission".into()),
        }
    }
    fn audits_letter(&self, p: &PreparedDeadLetter) -> Result<EventPayloadAudits, AppError> {
        let id = p.connector_id.to_string();
        let letter = p.letter_id.ok_or(AppError::NotFound)?;
        let make = |action, reason: String| {
            AuditRecord::by(
                &p.principal,
                action,
                ResourceType::Connector,
                id.clone(),
                p.trace_id.clone(),
                self.clock.now(),
            )
            .with_reason(reason)
        };
        Ok(EventPayloadAudits {
            revealed: make(
                AuditAction::PayloadPiiRevealed,
                format!("dead-letter:{letter}:reveal=pii"),
            ),
            denied: make(AuditAction::PayloadRevealDenied, "permission".into()),
        })
    }
    fn evaluator<'a>(&'a self, reveal: EventReveal) -> Evaluator<'a, P> {
        Evaluator {
            reveal,
            projector: &self.projector,
        }
    }
    fn finish<T>(read: EventPayloadRead<T>) -> Result<T, EventPayloadError> {
        match read {
            EventPayloadRead::Missing => Err(AppError::NotFound.into()),
            EventPayloadRead::Refused(EventPayloadFailure::EventReadNotGranted) => {
                Err(AppError::Forbidden {
                    permission: Permission::EventRead,
                }
                .into())
            }
            EventPayloadRead::Refused(EventPayloadFailure::PiiRevealNotGranted) => {
                Err(EventPayloadError::PiiRevealNotGranted)
            }
            EventPayloadRead::Ready(v) => Ok(v),
        }
    }
}

impl<S: EventPayloadStore + 'static, C: Clock + 'static, P: PayloadProjector + 'static>
    EventPayloadUseCase for EventPayloadService<S, C, P>
{
    fn prepare_event<'a>(
        &'a self,
        p: &'a Principal,
        n: NamespaceId,
        k: EventKey,
        r: EventReveal,
        c: &'a RequestContext,
    ) -> EventPrepareFuture<'a, PreparedInboxEvent> {
        Box::pin(self.prepare_event_inner(p, n, k, r, c))
    }
    fn prepare_dead_letter<'a>(
        &'a self,
        p: &'a Principal,
        n: NamespaceId,
        id: ConnectorId,
        l: Option<Uuid>,
        r: EventReveal,
        c: &'a RequestContext,
    ) -> EventPrepareFuture<'a, PreparedDeadLetter> {
        Box::pin(self.prepare_dead_letter_inner(p, n, id, l, r, c))
    }
    fn event<'a>(&'a self, p: PreparedInboxEvent) -> EventReadFuture<'a, InboxEventRead> {
        Box::pin(async move {
            let a = self.audits_event(&p);
            let read = self
                .store
                .read_event(&p, &self.evaluator(p.reveal), &a)
                .await
                .map_err(AppError::from)?;
            let result = Self::finish(read);
            record_read(
                "event",
                match &result {
                    Ok(read) if matches!(read.data, EventData::Revealed(_)) => "revealed",
                    Ok(_) => "structure_only",
                    Err(_) => "denied",
                },
            );
            result
        })
    }
    fn dead_letter<'a>(&'a self, p: PreparedDeadLetter) -> EventReadFuture<'a, DeadLetterRead> {
        Box::pin(async move {
            let a = self.audits_letter(&p)?;
            let read = self
                .store
                .read_dead_letter(&p, &self.evaluator(p.reveal), &a)
                .await
                .map_err(AppError::from)?;
            let result = Self::finish(read);
            record_read(
                "dead_letter",
                match &result {
                    Ok(read) if matches!(read.data, DeadLetterData::Revealed { .. }) => "revealed",
                    Ok(_) => "structure_only",
                    Err(_) => "denied",
                },
            );
            result
        })
    }
    fn dead_letters<'a>(
        &'a self,
        p: PreparedDeadLetter,
        b: Option<Uuid>,
        l: u32,
    ) -> EventReadFuture<'a, Vec<DeadLetterRead>> {
        Box::pin(async move {
            let read = self
                .store
                .list_dead_letters(&p, b, l, &self.evaluator(EventReveal::Structure))
                .await
                .map_err(AppError::from)?;
            let result = Self::finish(read);
            record_read(
                "dead_letter",
                if result.is_ok() {
                    "structure_only"
                } else {
                    "denied"
                },
            );
            result
        })
    }
}

struct Evaluator<'a, P> {
    reveal: EventReveal,
    projector: &'a P,
}
impl<P: PayloadProjector> EventPayloadEvaluator for Evaluator<'_, P> {
    fn event(
        &self,
        mut selected: SelectedInboxEvent,
        role: Role,
        grant: bool,
    ) -> Result<InboxEventRead, EventPayloadFailure> {
        authorize(role, self.reveal, grant)?;
        let payload = std::mem::replace(&mut selected.payload, StoredPayloadBytes::new(Vec::new()));
        let data_size_bytes = payload.as_slice().len();
        let data = match self.reveal {
            EventReveal::Pii => EventData::Revealed(payload),
            EventReveal::Structure => EventData::Structure(
                self.projector
                    .project(payload.as_slice())
                    .map_err(|_| EventPayloadFailure::EventReadNotGranted)?,
            ),
        };
        Ok(InboxEventRead {
            event: selected,
            data_size_bytes,
            data,
        })
    }
    fn dead_letter(
        &self,
        mut selected: SelectedDeadLetter,
        role: Role,
        grant: bool,
    ) -> Result<DeadLetterRead, EventPayloadFailure> {
        authorize(role, self.reveal, grant)?;
        let headers = std::mem::take(&mut selected.headers);
        let body = std::mem::replace(&mut selected.body, StoredPayloadBytes::new(Vec::new()));
        let body_size_bytes = body.as_slice().len();
        let header_count = headers.len();
        let data = match self.reveal {
            EventReveal::Pii => DeadLetterData::Revealed { headers, body },
            EventReveal::Structure => {
                let s = if selected.truncated {
                    DeadLetterStructure::Opaque
                } else {
                    self.projector
                        .project(body.as_slice())
                        .map(DeadLetterStructure::Json)
                        .unwrap_or(DeadLetterStructure::Opaque)
                };
                DeadLetterData::Structure(s)
            }
        };
        Ok(DeadLetterRead {
            letter: selected,
            body_size_bytes,
            header_count,
            data,
        })
    }
}
fn authorize(role: Role, reveal: EventReveal, grant: bool) -> Result<(), EventPayloadFailure> {
    if !role.grants(Permission::EventRead) {
        return Err(EventPayloadFailure::EventReadNotGranted);
    }
    if reveal == EventReveal::Pii && (!role.grants(Permission::PayloadReadInternal) || !grant) {
        return Err(EventPayloadFailure::PiiRevealNotGranted);
    }
    Ok(())
}

fn record_read(surface: &'static str, outcome: &'static str) {
    metrics::counter!("event_value_reads_total", "surface" => surface, "outcome" => outcome)
        .increment(1);
}
