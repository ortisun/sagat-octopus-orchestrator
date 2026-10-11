//! What the journal says about a run, as spans and as measurements (FR-008-001, FR-008-002).
//!
//! The write path adds nothing to produce telemetry beyond remembering, in the transaction that
//! changes a run, that it changed: the spans of a run and of its attempts, and the measurements
//! of how long they took, are **derived from the journal afterwards**, by [`derive`], a pure function
//! of the run's facts and its journal. That is what makes them survive a crash of the process that
//! produced the event, a collector that is down for an hour, and a restart of the projector: the
//! source is durable, deriving again gives the same spans (their ids are computed, not drawn), and
//! progress moves only when the spans have been taken (D160, D162).

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use sagat_domain::execution::{RunEvent, RunId, RunStatus};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_telemetry::{
    AttrValue, SpanData, SpanEvent, SpanId, SpanKind, SpanStatus, TraceId, attempt_span_id,
    event_span_id, run_span_id,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;

/// One entry of a run's journal, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub seq: u64,
    pub event: RunEvent,
    /// Who or what caused it: `system:scheduler`, a worker's subject, a person's subject.
    pub actor: String,
    pub occurred_at: OffsetDateTime,
}

/// What is known about a run apart from its journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFacts {
    pub tenant_id: TenantId,
    pub namespace_id: NamespaceId,
    pub run_id: RunId,
    pub workflow: String,
    pub version: u32,
    pub trace_id: TraceId,
    /// The span the run's own span hangs from, when it was started by a request or an event.
    pub parent_span: Option<SpanId>,
    /// The run that started this one, when it is a child: its span is the parent.
    pub parent_run: Option<RunId>,
    pub created_at: OffsetDateTime,
    /// The identity of each step (by node), which an attempt's span id is computed from.
    pub steps: HashMap<String, Uuid>,
}

/// How an attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// Leased and not over: nothing to export yet.
    Open,
    Succeeded,
    /// Failed for good.
    Failed,
    /// Failed and another attempt was scheduled (or the lease was lost and another is due).
    Retry,
    /// Failed and waits for a person.
    Review,
    /// Ended without a known outcome: a reconciliation was asked for.
    UnknownOutcome,
    /// The run was stopped while it held the lease.
    Cancelled,
}

impl AttemptOutcome {
    /// The name shown and counted: a closed set.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Retry => "retry",
            Self::Review => "review",
            Self::UnknownOutcome => "unknown_outcome",
            Self::Cancelled => "cancelled",
        }
    }

    const fn is_error(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Retry | Self::Review | Self::UnknownOutcome
        )
    }
}

/// One attempt of one step, as the journal tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub node: String,
    pub attempt: u32,
    pub generation: u64,
    /// When the step became ready for this attempt, when the journal says.
    pub ready_at: Option<OffsetDateTime>,
    pub leased_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
    /// The journal sequence of the entry that ended it.
    pub ended_seq: Option<u64>,
    pub outcome: AttemptOutcome,
    pub code: Option<String>,
    /// For a retry: not before.
    pub retry_not_before: Option<OffsetDateTime>,
    pub span: SpanId,
}

/// A measurement the projector makes once the spans it derived have been taken.
#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    /// An attempt ended.
    AttemptEnded {
        outcome: &'static str,
        seconds: f64,
        /// From ready to leased, when known.
        schedule_latency: Option<f64>,
    },
    /// A run reached a terminal status.
    RunFinished { status: &'static str, seconds: f64 },
}

/// What [`derive`] made of the entries beyond the ones already projected.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Derived {
    pub spans: Vec<SpanData>,
    pub observations: Vec<Observation>,
}

fn nanos(at: OffsetDateTime) -> u64 {
    u64::try_from(at.unix_timestamp_nanos()).unwrap_or(0)
}

fn seconds_between(from: OffsetDateTime, to: OffsetDateTime) -> f64 {
    ((to - from).as_seconds_f64()).max(0.0)
}

/// The attempts the journal tells of, in the order they were leased. `entries` must be in journal
/// order and may be a subset of the journal as long as it holds, for every attempt it wants to
/// close, the entry that leased it and the entries that ended attempts before it (see
/// [`CONTEXT_EVENTS`]).
#[must_use]
pub fn attempts(facts: &RunFacts, entries: &[JournalEntry]) -> Vec<Attempt> {
    let mut done: Vec<Attempt> = Vec::new();
    let mut open: HashMap<String, Attempt> = HashMap::new();
    let mut ready: HashMap<String, OffsetDateTime> = HashMap::new();
    let mut close = |open: &mut HashMap<String, Attempt>,
                     node: &str,
                     attempt: u32,
                     seq: u64,
                     at: OffsetDateTime,
                     outcome: AttemptOutcome,
                     code: Option<String>,
                     not_before: Option<OffsetDateTime>| {
        if open.get(node).is_some_and(|a| a.attempt == attempt)
            && let Some(mut a) = open.remove(node)
        {
            a.ended_at = Some(at);
            a.ended_seq = Some(seq);
            a.outcome = outcome;
            a.code = code;
            a.retry_not_before = not_before;
            done.push(a);
        }
    };
    for entry in entries {
        let at = entry.occurred_at;
        match &entry.event {
            RunEvent::StepReleased { node } | RunEvent::StepRetryDue { node } => {
                ready.insert(node.as_str().to_owned(), at);
            }
            RunEvent::StepLeased {
                node,
                attempt,
                generation,
            } => {
                let name = node.as_str().to_owned();
                let step = facts.steps.get(&name).copied().unwrap_or_else(Uuid::nil);
                open.insert(
                    name.clone(),
                    Attempt {
                        node: name.clone(),
                        attempt: *attempt,
                        generation: *generation,
                        ready_at: ready.remove(&name),
                        leased_at: at,
                        ended_at: None,
                        ended_seq: None,
                        outcome: AttemptOutcome::Open,
                        code: None,
                        retry_not_before: None,
                        span: attempt_span_id(step, *attempt),
                    },
                );
            }
            RunEvent::StepCompleted { node, attempt, .. } => close(
                &mut open,
                node.as_str(),
                *attempt,
                entry.seq,
                at,
                AttemptOutcome::Succeeded,
                None,
                None,
            ),
            RunEvent::StepRetryScheduled {
                node,
                attempt,
                code,
                not_before,
                ..
            } => close(
                &mut open,
                node.as_str(),
                *attempt,
                entry.seq,
                at,
                AttemptOutcome::Retry,
                Some(code.as_str().to_owned()),
                Some(*not_before),
            ),
            RunEvent::StepFailed {
                node,
                code,
                fence: Some(fence),
            } => close(
                &mut open,
                node.as_str(),
                fence.attempt,
                entry.seq,
                at,
                AttemptOutcome::Failed,
                Some(code.as_str().to_owned()),
                None,
            ),
            RunEvent::StepNeedsReview {
                node,
                code,
                fence: Some(fence),
            } => close(
                &mut open,
                node.as_str(),
                fence.attempt,
                entry.seq,
                at,
                AttemptOutcome::Review,
                Some(code.as_str().to_owned()),
                None,
            ),
            RunEvent::ReconciliationRequested { node, fence, cause } => close(
                &mut open,
                node.as_str(),
                fence.attempt,
                entry.seq,
                at,
                AttemptOutcome::UnknownOutcome,
                Some(cause.as_str().to_owned()),
                None,
            ),
            RunEvent::StepCancelled { node } => {
                let attempt = open.get(node.as_str()).map(|a| a.attempt);
                if let Some(attempt) = attempt {
                    close(
                        &mut open,
                        node.as_str(),
                        attempt,
                        entry.seq,
                        at,
                        AttemptOutcome::Cancelled,
                        None,
                        None,
                    );
                }
            }
            _ => {}
        }
    }
    done.extend(open.into_values());
    done.sort_by_key(|a| (a.leased_at, a.attempt));
    done
}

/// The event types [`derive`] needs from the part of the journal that was projected already: the
/// ones that close an attempt leased in an earlier batch, and the starts of the timers and waits
/// that end in this one.
pub const CONTEXT_EVENTS: [&str; 11] = [
    "step.released",
    "step.retry_due",
    "step.leased",
    "step.completed",
    "step.retry_scheduled",
    "step.failed",
    "step.needs_review",
    "reconciliation.requested",
    "step.cancelled",
    "timer.started",
    "wait.started",
];

/// Whether `actor` is the platform itself or one of its workers, as opposed to a person: the only
/// thing a span says about who did something (a subject can be personal data).
fn actor_kind(actor: &str) -> &'static str {
    if actor.starts_with("system:") || actor.starts_with("worker:") {
        "system"
    } else {
        "user"
    }
}

fn base_attributes(facts: &RunFacts) -> Vec<(String, AttrValue)> {
    vec![
        (
            "sagat.tenant.id".to_owned(),
            AttrValue::from(facts.tenant_id.to_string()),
        ),
        (
            "sagat.namespace.id".to_owned(),
            AttrValue::from(facts.namespace_id.to_string()),
        ),
        (
            "sagat.run.id".to_owned(),
            AttrValue::from(facts.run_id.to_string()),
        ),
        (
            "sagat.workflow".to_owned(),
            AttrValue::from(facts.workflow.as_str()),
        ),
        (
            "sagat.workflow.version".to_owned(),
            AttrValue::from(facts.version),
        ),
    ]
}

/// The spans and measurements of the entries after `applied_seq`. Entries at or before it are only
/// context: they are never turned into a span again.
#[must_use]
pub fn derive(facts: &RunFacts, entries: &[JournalEntry], applied_seq: u64) -> Derived {
    let run_uuid = facts.run_id.as_uuid();
    let run_span = run_span_id(run_uuid);
    let mut out = Derived::default();

    for attempt in attempts(facts, entries) {
        let (Some(ended_at), Some(ended_seq)) = (attempt.ended_at, attempt.ended_seq) else {
            continue;
        };
        if ended_seq <= applied_seq {
            continue;
        }
        let duration = seconds_between(attempt.leased_at, ended_at);
        let latency = attempt
            .ready_at
            .map(|ready| seconds_between(ready, attempt.leased_at));
        let mut attributes = base_attributes(facts);
        attributes.push((
            "sagat.node".to_owned(),
            AttrValue::from(attempt.node.as_str()),
        ));
        attributes.push(("sagat.attempt".to_owned(), AttrValue::from(attempt.attempt)));
        attributes.push((
            "sagat.generation".to_owned(),
            AttrValue::Int(i64::try_from(attempt.generation).unwrap_or(i64::MAX)),
        ));
        attributes.push((
            "sagat.outcome".to_owned(),
            AttrValue::from(attempt.outcome.as_str()),
        ));
        if let Some(code) = &attempt.code {
            attributes.push((
                "sagat.failure.code".to_owned(),
                AttrValue::from(code.as_str()),
            ));
        }
        if let Some(latency) = latency {
            attributes.push((
                "sagat.schedule_latency_ms".to_owned(),
                AttrValue::Double(latency * 1000.0),
            ));
        }
        let mut events = Vec::new();
        if let (AttemptOutcome::Retry, Some(not_before)) =
            (attempt.outcome, attempt.retry_not_before)
        {
            events.push(SpanEvent {
                name: "retry scheduled".to_owned(),
                unix_nano: nanos(ended_at),
                attributes: vec![(
                    "sagat.retry.not_before".to_owned(),
                    AttrValue::from(not_before.unix_timestamp()),
                )],
            });
        }
        out.spans.push(SpanData {
            trace_id: facts.trace_id,
            span_id: attempt.span,
            parent: Some(run_span),
            name: "step.attempt".to_owned(),
            kind: SpanKind::Internal,
            start_unix_nano: nanos(attempt.leased_at),
            end_unix_nano: nanos(ended_at),
            attributes,
            events,
            links: Vec::new(),
            status: match attempt.outcome {
                AttemptOutcome::Succeeded => SpanStatus::Ok,
                o if o.is_error() => SpanStatus::Error(attempt.code.clone().unwrap_or_default()),
                _ => SpanStatus::Unset,
            },
        });
        out.observations.push(Observation::AttemptEnded {
            outcome: attempt.outcome.as_str(),
            seconds: duration,
            schedule_latency: latency,
        });
    }

    // Timers and waits: where the time of a run goes when no worker holds it.
    let mut timers: HashMap<String, (u64, OffsetDateTime)> = HashMap::new();
    let mut waits: HashMap<String, (u64, OffsetDateTime)> = HashMap::new();
    for entry in entries {
        let at = entry.occurred_at;
        let mut finished =
            |kind: &str, started: Option<(u64, OffsetDateTime)>, node: &str, outcome: &str| {
                if let Some((started_seq, started_at)) = started
                    && entry.seq > applied_seq
                {
                    let mut attributes = base_attributes(facts);
                    attributes.push(("sagat.node".to_owned(), AttrValue::from(node)));
                    attributes.push(("sagat.outcome".to_owned(), AttrValue::from(outcome)));
                    out.spans.push(SpanData {
                        trace_id: facts.trace_id,
                        span_id: event_span_id(run_uuid, started_seq),
                        parent: Some(run_span),
                        name: kind.to_owned(),
                        kind: SpanKind::Internal,
                        start_unix_nano: nanos(started_at),
                        end_unix_nano: nanos(at),
                        attributes,
                        events: Vec::new(),
                        links: Vec::new(),
                        status: if outcome == "timed_out" {
                            SpanStatus::Error("WAIT_TIMED_OUT".to_owned())
                        } else {
                            SpanStatus::Unset
                        },
                    });
                }
            };
        match &entry.event {
            RunEvent::TimerStarted { node, .. } => {
                timers.insert(node.as_str().to_owned(), (entry.seq, at));
            }
            RunEvent::TimerFired { node } => {
                finished(
                    "timer",
                    timers.remove(node.as_str()),
                    node.as_str(),
                    "fired",
                );
            }
            RunEvent::WaitStarted { node, .. } => {
                waits.insert(node.as_str().to_owned(), (entry.seq, at));
            }
            RunEvent::WaitSatisfied { node } => {
                finished(
                    "wait.event",
                    waits.remove(node.as_str()),
                    node.as_str(),
                    "satisfied",
                );
            }
            RunEvent::WaitTimedOut { node, .. } => {
                finished(
                    "wait.event",
                    waits.remove(node.as_str()),
                    node.as_str(),
                    "timed_out",
                );
            }
            _ => {}
        }
    }

    // The run: one span from its creation to its end, with what was done to it on the way.
    let finished = entries.iter().find_map(|entry| match &entry.event {
        RunEvent::RunFinished { status } if entry.seq > applied_seq => {
            Some((*status, entry.occurred_at))
        }
        _ => None,
    });
    if let Some((status, ended_at)) = finished {
        let mut attributes = base_attributes(facts);
        attributes.push((
            "sagat.run.status".to_owned(),
            AttrValue::from(status.as_str()),
        ));
        let events = entries
            .iter()
            .filter_map(|entry| {
                let notable = matches!(
                    entry.event,
                    RunEvent::RunPaused
                        | RunEvent::RunResumed
                        | RunEvent::RunStopRequested { .. }
                        | RunEvent::StepRetryRequested { .. }
                        | RunEvent::StepSkipped { .. }
                        | RunEvent::StepResolved { .. }
                        | RunEvent::StepNeedsReview { .. }
                        | RunEvent::SagaAborting { .. }
                        | RunEvent::CompensationPlanned { .. }
                        | RunEvent::SagaCompensated { .. }
                        | RunEvent::ReconciliationRequested { .. }
                        | RunEvent::ReconciliationConcluded { .. }
                );
                notable.then(|| {
                    let mut attributes = vec![(
                        "sagat.actor.kind".to_owned(),
                        AttrValue::from(actor_kind(&entry.actor)),
                    )];
                    if let Some(node) = entry.event.node() {
                        attributes.push(("sagat.node".to_owned(), AttrValue::from(node.as_str())));
                    }
                    if let Some(cause) = entry.event.cause() {
                        attributes
                            .push(("sagat.cause.code".to_owned(), AttrValue::from(cause.code)));
                    }
                    SpanEvent {
                        name: entry.event.name().to_owned(),
                        unix_nano: nanos(entry.occurred_at),
                        attributes,
                    }
                })
            })
            .collect();
        out.spans.push(SpanData {
            trace_id: facts.trace_id,
            span_id: run_span,
            parent: facts
                .parent_run
                .map(|parent| run_span_id(parent.as_uuid()))
                .or(facts.parent_span),
            name: "workflow.run".to_owned(),
            kind: SpanKind::Internal,
            start_unix_nano: nanos(facts.created_at),
            end_unix_nano: nanos(ended_at),
            attributes,
            events,
            links: Vec::new(),
            status: match status {
                RunStatus::Succeeded | RunStatus::Compensated => SpanStatus::Ok,
                RunStatus::Failed
                | RunStatus::TimedOut
                | RunStatus::Partial
                | RunStatus::CompensationFailed => SpanStatus::Error(status.as_str().to_owned()),
                _ => SpanStatus::Unset,
            },
        });
        out.observations.push(Observation::RunFinished {
            status: status.as_str(),
            seconds: seconds_between(facts.created_at, ended_at),
        });
    }
    out
}

/// A run with changes the projector has not taken yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub tenant_id: TenantId,
    pub run_id: RunId,
    /// The last journal sequence of the run.
    pub head_seq: u64,
    /// The last sequence whose spans were taken.
    pub applied_seq: u64,
    /// Since when the run has had changes that are not projected.
    pub behind_since: OffsetDateTime,
}

/// How far behind the projection is, as the database sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backlog {
    /// Runs with changes not projected.
    pub runs: u64,
    /// The age of the oldest of them, in seconds; zero when there are none.
    pub oldest_seconds: f64,
}

/// What a run needs to be projected: its facts and the part of its journal that matters.
#[derive(Debug, Clone, PartialEq)]
pub struct ToProject {
    pub facts: RunFacts,
    pub entries: Vec<JournalEntry>,
    pub head_seq: u64,
    pub applied_seq: u64,
}

/// How far one run's telemetry has got, as the database sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunTelemetry {
    /// The run belongs to a trace (runs created before traces were recorded do not).
    pub traced: bool,
    pub finished: bool,
    /// The last journal sequence written for the run and the last one projected, when the run has a
    /// note; none for a run created before notes were kept, or one whose note was purged.
    pub head_seq: Option<u64>,
    pub applied_seq: Option<u64>,
    /// For how long the run has had changes that are not projected, in seconds; zero when it has not.
    pub behind_seconds: f64,
}

/// How a run's spans and measurements stand in relation to its journal (the timeline's
/// "projection state"). The journal itself, and the graph, are as current as the last commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryState {
    /// Everything in the journal has been turned into spans.
    Current,
    /// Behind, by less than the budget: it is being worked on.
    CatchingUp,
    /// Behind by more than the budget: the collector, the database or the projector has a problem
    /// (the journal is intact; the trace may be missing steps).
    Behind,
    /// The run has no trace to belong to: nothing is exported for it.
    NoTrace,
    /// The run was created before telemetry was projected and has not changed since.
    Untracked,
}

impl TelemetryState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "CURRENT",
            Self::CatchingUp => "CATCHING_UP",
            Self::Behind => "BEHIND",
            Self::NoTrace => "NO_TRACE",
            Self::Untracked => "UNTRACKED",
        }
    }
}

impl RunTelemetry {
    /// The state, given how late is too late.
    #[must_use]
    pub fn state(&self, budget: Duration) -> TelemetryState {
        if !self.traced {
            return TelemetryState::NoTrace;
        }
        match (self.head_seq, self.applied_seq) {
            (Some(head), Some(applied)) if applied < head => {
                if self.behind_seconds <= budget.as_secs_f64() {
                    TelemetryState::CatchingUp
                } else {
                    TelemetryState::Behind
                }
            }
            (Some(_), Some(_)) => TelemetryState::Current,
            // No note: a run that finished has had its note purged after it was projected; one that
            // is still live was created before notes were kept.
            _ if self.finished => TelemetryState::Current,
            _ => TelemetryState::Untracked,
        }
    }
}

/// The store of the telemetry projection.
pub trait TelemetryStore: Send + Sync {
    /// Takes the right to project: only one process does at a time. `false` when another has it.
    fn acquire_projector(&self) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// Runs that changed since they were last projected, the oldest change first.
    fn pending_runs(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Pending>, StoreError>> + Send;

    /// A run's facts and the journal to project for it: the entries after `applied_seq`, and the
    /// context [`CONTEXT_EVENTS`] needs (the whole journal when the run finished in this batch).
    /// `None` if the run is gone, or has no trace to belong to.
    fn load_for_projection(
        &self,
        pending: &Pending,
    ) -> impl Future<Output = Result<Option<ToProject>, StoreError>> + Send;

    /// Records that the spans of the run up to `upto` were taken. The note stays after the run is
    /// over (and is purged later): removing it would let a late change start the run's projection
    /// again from the beginning.
    fn mark_projected(
        &self,
        pending: &Pending,
        upto: u64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// How far one run's telemetry has got. `None` if the run does not exist.
    fn run_telemetry(
        &self,
        tenant: TenantId,
        run: RunId,
    ) -> impl Future<Output = Result<Option<RunTelemetry>, StoreError>> + Send;

    /// How far behind the projection is.
    fn backlog(&self) -> impl Future<Output = Result<Backlog, StoreError>> + Send;
}
