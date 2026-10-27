//! Deciding what an admitted event wakes or starts (`FR-004-001`, `FR-004-003`, `FR-004-008`).
//!
//! The inbox is the source of truth and the dispatcher is a loop over it: it claims events whose
//! turn has come, finds the triggers and the waiting steps each one is about, and delivers it to
//! them. Every delivery is recorded in the same transaction that wakes the step or creates the run,
//! under a key of (event, consumer), so a dispatch that is repeated after a crash, a lost claim or
//! a race delivers nothing twice, and one that fails half way is retried with backoff until it
//! succeeds or the event is set aside as dead.
//!
//! A step is found from an event (the event arrived while it waited) and an event from a step (it
//! arrived before the wait began, within the wait's look-back, or while the matcher was looking):
//! the second is what makes the first race harmless.

use std::future::Future;
use std::time::Duration;

use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::events::EventEnvelope;
use sagat_domain::execution::{CorrelationKey, ExecutionInstanceId, RunId};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::triggers::{CronSpec, Trigger, TriggerKind};
use sagat_domain::workflow::{EventType, Expression, NodeId, WorkflowDefinition};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;

/// How long a claimed event stays claimed. A dispatcher that dies holding one leaves it to be
/// claimed again after this; the work is idempotent, so the cost of a long lease is only latency.
pub const CLAIM_LEASE: Duration = Duration::from_secs(60);

/// A trigger of a published version, as it is registered for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerRegistration {
    /// Stable across the versions of a workflow that keep the trigger: a hash of the workflow and
    /// what the trigger is about (its filter is not part of it).
    pub id: Uuid,
    pub kind: TriggerKind,
    pub source: Option<sagat_domain::identity::Slug>,
    pub event_type: Option<EventType>,
    pub filter: Option<Expression>,
    pub schedule: Option<CronSpec>,
}

/// The triggers of `definition` that something other than a caller starts, ready to register.
#[must_use]
pub fn triggers_of(
    workflow: WorkflowId,
    definition: &WorkflowDefinition,
) -> Vec<TriggerRegistration> {
    definition
        .triggers()
        .iter()
        .filter(|trigger| trigger.kind() != TriggerKind::Manual)
        .map(|trigger| TriggerRegistration {
            id: trigger_id(workflow, trigger),
            kind: trigger.kind(),
            source: trigger.source().cloned(),
            event_type: trigger.event_type().cloned(),
            filter: trigger.filter().cloned(),
            schedule: trigger.schedule().cloned(),
        })
        .collect()
}

fn trigger_id(workflow: WorkflowId, trigger: &Trigger) -> Uuid {
    let mut hash = Sha256::new();
    for part in [
        b"sagat/trigger/v1".as_slice(),
        workflow.as_uuid().as_bytes().as_slice(),
        trigger.kind().as_str().as_bytes(),
        trigger.source().map_or("", |s| s.as_str()).as_bytes(),
        trigger
            .event_type()
            .map_or("", EventType::as_str)
            .as_bytes(),
        trigger
            .schedule()
            .map_or("", CronSpec::expression)
            .as_bytes(),
        trigger.schedule().map_or("", CronSpec::timezone).as_bytes(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // A name-based identifier (version 8, RFC 4122 variant).
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// A registered trigger with the place it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTrigger {
    pub registration: TriggerRegistration,
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub workflow_id: WorkflowId,
    pub version_id: WorkflowVersionId,
    /// For a cron trigger: the next time it fires, or `None` when it has not been scheduled yet.
    pub next_fire_at: Option<OffsetDateTime>,
    /// The database's clock when the trigger was read. A trigger is due by that clock, so what
    /// follows from its being due (the next time, how late it is) is worked out from it too: a
    /// replica whose own clock is off must not skip or repeat a tick.
    pub db_now: OffsetDateTime,
}

/// An event taken from the inbox for dispatching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedEvent {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub envelope: EventEnvelope,
    pub occurred_at: Option<OffsetDateTime>,
    pub payload: Vec<u8>,
    pub received_at: OffsetDateTime,
    /// Dispatches that failed before this one.
    pub attempts: u32,
    /// The trace the event was published in, when its publisher sent one.
    pub trace: Option<crate::identity::TraceLink>,
}

/// An event as expressions and runs see it: the envelope and the data, with the instants already
/// written as RFC 3339 text so the adapter that builds the document needs no calendar.
#[derive(Debug, Clone, Copy)]
pub struct EventView<'a> {
    pub id: &'a str,
    pub source: &'a str,
    pub event_type: &'a str,
    pub subject: Option<&'a str>,
    pub correlation_key: Option<&'a str>,
    pub occurred_at: Option<&'a str>,
    pub received_at: &'a str,
    /// The data: a JSON object.
    pub data: &'a [u8],
}

impl ClaimedEvent {
    /// Writes an instant the way the document shows it.
    #[must_use]
    pub fn rfc3339(at: OffsetDateTime) -> String {
        at.format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default()
    }
}

/// The idempotency key under which `trigger` starts the run for `event`, and the fingerprint that
/// tells a repeat of the same start from a different request: starting twice for one admission of
/// an event, from one trigger, is one run.
///
/// The instant of the admission is part of it. An id is a new event once its window has ended, and
/// the provider may well use it again (a nightly job's id, a debounced event's); without the
/// instant the second admission would find the first one's key and start nothing. The instant is
/// the database's, in microseconds, and does not change however often the event is dispatched.
#[must_use]
pub fn start_key(event: &ClaimedEvent) -> (String, Vec<u8>) {
    let admitted = i64::try_from(event.received_at.unix_timestamp_nanos() / 1_000).unwrap_or(0);
    let mut hash = Sha256::new();
    for part in [
        event.namespace_id.as_uuid().as_bytes().as_slice(),
        event.envelope.key.source.as_str().as_bytes(),
        event.envelope.key.event_id.as_str().as_bytes(),
        admitted.to_be_bytes().as_slice(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let digest = hash.finalize();
    // 8 to 160 characters, like every idempotency key.
    let key: String = std::iter::once("event-".to_owned())
        .chain(digest[..20].iter().map(|b| format!("{b:02x}")))
        .collect();
    (key, digest.to_vec())
}

/// A step that waits for an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingStep {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub run_id: RunId,
    pub instance: ExecutionInstanceId,
    pub step_id: Uuid,
    pub event_type: EventType,
    pub correlation_key: CorrelationKey,
    /// The wait accepts events received at or after this instant.
    pub since: OffsetDateTime,
}

impl WaitingStep {
    /// A root-only receiver may use this candidate only with its pinned physical identity.
    /// This check grants no authority to deliver; the caller still owns the current run lock.
    #[must_use]
    pub fn root_node(&self, version: WorkflowVersionId) -> Option<&NodeId> {
        (self.instance.path().is_root()
            && self.step_id
                == crate::execution::instance_step_id(version, self.run_id, &self.instance))
        .then_some(self.instance.node())
    }
}

/// What recording a delivery needs to know about its consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRef {
    pub namespace_id: NamespaceId,
    pub source: String,
    pub event_id: String,
    /// `wait:{run}/{node}` or `trigger:{id}`.
    pub consumer: String,
    pub run_id: Option<RunId>,
    pub node: Option<String>,
    pub trigger_id: Option<Uuid>,
}

impl DeliveryRef {
    /// The legacy root delivery of `event` to `step`; scoped delivery has no native receiver yet.
    #[must_use]
    pub fn to_wait(event: &ClaimedEvent, step: &WaitingStep) -> Option<Self> {
        if !step.instance.path().is_root() {
            return None;
        }
        Some(Self {
            namespace_id: event.namespace_id,
            source: event.envelope.key.source.to_string(),
            event_id: event.envelope.key.event_id.to_string(),
            consumer: format!("wait:{}/{}", step.run_id, step.instance.node()),
            run_id: Some(step.run_id),
            node: Some(step.instance.node().as_str().to_owned()),
            trigger_id: None,
        })
    }

    /// The delivery of `event` to the trigger `id`, which starts `run`.
    #[must_use]
    pub fn to_trigger(event: &ClaimedEvent, id: Uuid, run: RunId) -> Self {
        Self {
            namespace_id: event.namespace_id,
            source: event.envelope.key.source.to_string(),
            event_id: event.envelope.key.event_id.to_string(),
            consumer: format!("trigger:{id}"),
            run_id: Some(run),
            node: None,
            trigger_id: Some(id),
        }
    }
}

/// Storage port of the dispatcher. Implemented by `sagat-infra-postgres`; every instant it compares
/// is the database's.
pub trait EventDispatchStore: Send + Sync {
    /// Takes up to `limit` events that are due, oldest first, and keeps them for `lease`: another
    /// dispatcher does not see them until the lease ends. Several dispatchers share the inbox.
    fn claim_events(
        &self,
        limit: u32,
        lease: Duration,
    ) -> impl Future<Output = Result<Vec<ClaimedEvent>, StoreError>> + Send;

    /// The event has been delivered to everything it was due to reach.
    fn finish_event(
        &self,
        event: &ClaimedEvent,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Dispatching failed: try again after `delay`, or, when `dead`, set it aside for inspection.
    /// The attempt is counted against the event only when `counted`: an event that waits for room
    /// (a quota, a store that is down) has not failed in a way a retry would not fix.
    fn retry_event(
        &self,
        event: &ClaimedEvent,
        error: &str,
        delay: Duration,
        dead: bool,
        counted: bool,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The registered event triggers of the namespace that could be about this source and type.
    fn triggers_matching(
        &self,
        event: &ClaimedEvent,
    ) -> impl Future<Output = Result<Vec<StoredTrigger>, StoreError>> + Send;

    /// Steps waiting for exactly this event: same namespace, type and correlation key, and a wait
    /// that began early enough to accept it. In SQL tuple order of (run, path nodes, path
    /// indices, node), after `after`, at most
    /// `limit`: a fan-out larger than a page is walked page by page. This is not the derived
    /// Rust instance order. Discovery stays root-only until qualified delivery is implemented.
    fn waits_for_event(
        &self,
        event: &ClaimedEvent,
        after: Option<(RunId, &ExecutionInstanceId)>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<WaitingStep>, StoreError>> + Send;

    /// Event waits whose inbox has not been searched yet, oldest first.
    fn unscanned_waits(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<WaitingStep>, StoreError>> + Send;

    /// The earliest event the wait should have received and has not: a match by type and
    /// correlation key received since the wait accepts them, not yet delivered to it.
    fn undelivered_event_for(
        &self,
        step: &WaitingStep,
    ) -> impl Future<Output = Result<Option<ClaimedEvent>, StoreError>> + Send;

    /// Records that the inbox was searched for this wait.
    fn mark_wait_scanned(
        &self,
        step: &WaitingStep,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Removes events whose deduplication window ended (and their deliveries), at most `limit`.
    fn purge_events(&self, limit: u32) -> impl Future<Output = Result<u64, StoreError>> + Send;

    /// Cron triggers whose time has come (or that were never scheduled), at most `limit`.
    fn due_cron_triggers(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<StoredTrigger>, StoreError>> + Send;

    /// Moves a cron trigger to its next time, provided it is still at `expected` (another
    /// dispatcher may have fired it): `true` when this call moved it and so owns the firing.
    fn advance_cron_trigger(
        &self,
        id: Uuid,
        expected: Option<OffsetDateTime>,
        next: OffsetDateTime,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// A tick that could not be started is not owed any less: it stays at `scheduled` and is tried
    /// again from `retry_at`, under the same key. `true` when the trigger was still at `scheduled`.
    fn delay_cron_trigger(
        &self,
        id: Uuid,
        scheduled: OffsetDateTime,
        retry_at: OffsetDateTime,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// How many events are waiting to be dispatched (for the backlog gauge).
    fn pending_events(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;
}

/// Why a schedule could not be evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CronFailure {
    #[error("the cron expression is not valid")]
    Expression,
    #[error("the time zone is not a known IANA name")]
    TimeZone,
    #[error("the schedule has no next time")]
    Exhausted,
}

/// The calendar a cron schedule is evaluated against. Implemented by `sagat-infra-ir`, which holds
/// the time-zone database; the domain keeps only the text.
pub trait CronSchedule: Send + Sync {
    /// Whether the schedule means something: five valid fields, a known time zone.
    ///
    /// # Errors
    /// [`CronFailure`] naming what is wrong.
    fn validate(&self, spec: &CronSpec) -> Result<(), CronFailure>;

    /// The first time strictly after `after` at which the schedule fires, with daylight-saving
    /// changes of its time zone taken into account.
    ///
    /// # Errors
    /// [`CronFailure`].
    fn next_after(
        &self,
        spec: &CronSpec,
        after: OffsetDateTime,
    ) -> Result<OffsetDateTime, CronFailure>;
}
