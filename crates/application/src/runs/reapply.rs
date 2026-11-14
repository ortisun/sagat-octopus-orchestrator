//! Reapply after a reset (feature 018, US06, `FR-018-039`): which signals and Updates of the source a reset
//! delivers again to the run it creates, in which order, and what the report of the reset says about each.
//!
//! Everything here is a function of the journal of the source, the definition it ran and the policy, so the
//! preview and the reset cannot disagree about the candidates. What is delivered, and under which key, is the
//! run service's (`runs.rs`); what is stored is [`ReapplyEntry`] as JSON.

use sagat_domain::execution::{
    OperationError, ResetId, ResetReapply, RunEvent, UpdateReceiptStatus, UpdateRejection, WaitVia,
};
use sagat_domain::workflow::{NodeId, NodeKind, WorkflowDefinition};
use serde_json::{Value, json};
use std::collections::HashMap;
use time::OffsetDateTime;

use super::{Retained, SourceEvent};
use crate::declared_operations::DeclaredOperationStoreError;
use crate::operation_evaluation::DeclaredOperationArguments;

/// The most entries a reapply holds, whatever the policy (the contract's `maxItems`, and the bound the register
/// enforces on `run_resets.reapply_report`). More candidates than this refuse the reset
/// (`RESET_REAPPLY_TOO_MANY`): nothing is dropped without being reported.
pub const REAPPLY_CEILING: usize = 256;

/// What a candidate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapplyKind {
    /// A signal sent to the source (`wait.satisfied` with `via: signal`).
    Signal,
    /// A declared Update whose arguments the source retained (`state.updated` with `retained: true`).
    Update,
}

impl ReapplyKind {
    /// The closed name the register stores and the API answers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Signal => "SIGNAL",
            Self::Update => "UPDATE",
        }
    }

    /// The kind a stored name stands for; `None` for a name this version does not know.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "SIGNAL" => Some(Self::Signal),
            "UPDATE" => Some(Self::Update),
            _ => None,
        }
    }
}

/// What came of a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapplyOutcome {
    /// The preview only: the reset will try it.
    WillTry,
    /// Delivered to the new run.
    Reapplied,
    /// Tried, and the new run said no (nothing waits for the signal; the Update was refused).
    Rejected,
    /// Not tried, for the reason the entry gives.
    NotReapplied,
}

impl ReapplyOutcome {
    /// The closed name the register stores and the API answers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WillTry => "WILL_TRY",
            Self::Reapplied => "REAPPLIED",
            Self::Rejected => "REJECTED",
            Self::NotReapplied => "NOT_REAPPLIED",
        }
    }

    /// The outcome a stored name stands for; `None` for a name this version does not know.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "WILL_TRY" => Some(Self::WillTry),
            "REAPPLIED" => Some(Self::Reapplied),
            "REJECTED" => Some(Self::Rejected),
            "NOT_REAPPLIED" => Some(Self::NotReapplied),
            _ => None,
        }
    }
}

/// Why a candidate was not reapplied or was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapplyReason {
    /// The row was written before the journal said how the wait was satisfied or whether the Update kept its
    /// arguments, so it cannot be told apart from what must not be delivered again.
    LegacyEvent,
    /// The payload of the signal, or the arguments of the Update, are no longer kept.
    NotRetained,
    /// An earlier signal was not delivered, so the later ones are not: they would reach the wait that earlier
    /// one was meant for.
    PreviousSignalNotDelivered,
    /// No step of the new run waited for the signal (it ends the reapply of signals).
    NotWaiting,
    /// The validator of the Update declined the arguments.
    UpdateRejected,
    /// The declared state of the new run was not at the revision the Update was applied to in the source.
    RevisionConflict,
    /// The declaration no longer accepts the arguments or the operation.
    UpdateNotAccepted,
}

impl ReapplyReason {
    /// The closed name the register stores and the API answers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyEvent => "LEGACY_EVENT",
            Self::NotRetained => "NOT_RETAINED",
            Self::PreviousSignalNotDelivered => "PREVIOUS_SIGNAL_NOT_DELIVERED",
            Self::NotWaiting => "NOT_WAITING",
            Self::UpdateRejected => "UPDATE_REJECTED",
            Self::RevisionConflict => "REVISION_CONFLICT",
            Self::UpdateNotAccepted => "UPDATE_NOT_ACCEPTED",
        }
    }

    /// The reason a stored name stands for; `None` for a name this version does not know.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "LEGACY_EVENT" => Some(Self::LegacyEvent),
            "NOT_RETAINED" => Some(Self::NotRetained),
            "PREVIOUS_SIGNAL_NOT_DELIVERED" => Some(Self::PreviousSignalNotDelivered),
            "NOT_WAITING" => Some(Self::NotWaiting),
            "UPDATE_REJECTED" => Some(Self::UpdateRejected),
            "REVISION_CONFLICT" => Some(Self::RevisionConflict),
            "UPDATE_NOT_ACCEPTED" => Some(Self::UpdateNotAccepted),
            _ => None,
        }
    }
}

/// One line of the preview and of the report of a reset: the candidate, and what came (or will come) of it.
/// It names the signal or the operation, never a payload, an argument or a digest of them: those are content
/// of the tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReapplyEntry {
    /// The sequence of the row of the source that is the candidate.
    pub sequence: u64,
    pub kind: ReapplyKind,
    /// The name of the signal, or of the operation of the Update.
    pub name: String,
    pub outcome: ReapplyOutcome,
    pub reason: Option<ReapplyReason>,
}

impl ReapplyEntry {
    /// The entry as the register stores it and the API answers it: a signal has a `name`, an Update an
    /// `operation`; `reason` is absent when there is none.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut entry = json!({
            "sequence": self.sequence,
            "kind": self.kind.as_str(),
            "outcome": self.outcome.as_str(),
        });
        if let Some(object) = entry.as_object_mut() {
            let key = match self.kind {
                ReapplyKind::Signal => "name",
                ReapplyKind::Update => "operation",
            };
            object.insert(key.to_owned(), json!(self.name));
            if let Some(reason) = self.reason {
                object.insert("reason".to_owned(), json!(reason.as_str()));
            }
        }
        entry
    }
}

/// The report as the register stores it: a JSON array of entries.
#[must_use]
pub fn report_json(entries: &[ReapplyEntry]) -> Value {
    Value::Array(entries.iter().map(ReapplyEntry::to_json).collect())
}

/// A line of a stored report, as it is read: the names are the strings the register holds, because a version that
/// is not this one may have written a kind, an outcome or a reason this one does not know, and reading a report must
/// never fail on it. A field this version does not know is left out; so is a line that is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEntry {
    /// The row of the journal of the source that this is.
    pub sequence: u64,
    pub kind: String,
    /// The signal, for a line of kind `SIGNAL`.
    pub name: Option<String>,
    /// The declared operation, for a line of kind `UPDATE`.
    pub operation: Option<String>,
    pub outcome: String,
    pub reason: Option<String>,
}

impl ReportEntry {
    /// The line a stored value stands for; `None` when it has no sequence, kind or outcome.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            sequence: value.get("sequence")?.as_u64()?,
            kind: text("kind")?,
            name: text("name"),
            operation: text("operation"),
            outcome: text("outcome")?,
            reason: text("reason"),
        })
    }

    /// What the line is about: the signal or the operation.
    #[must_use]
    pub fn subject(&self) -> Option<&str> {
        self.name.as_deref().or(self.operation.as_deref())
    }
}

impl From<&ReapplyEntry> for ReportEntry {
    fn from(entry: &ReapplyEntry) -> Self {
        let (name, operation) = match entry.kind {
            ReapplyKind::Signal => (Some(entry.name.clone()), None),
            ReapplyKind::Update => (None, Some(entry.name.clone())),
        };
        Self {
            sequence: entry.sequence,
            kind: entry.kind.as_str().to_owned(),
            name,
            operation,
            outcome: entry.outcome.as_str().to_owned(),
            reason: entry.reason.map(|reason| reason.as_str().to_owned()),
        }
    }
}

/// The lines a stored report holds. It never fails: what is not a list is an empty report, and a line that is not a line
/// is left out.
#[must_use]
pub fn report_from_json(value: &Value) -> Vec<ReportEntry> {
    value
        .as_array()
        .map(|lines| lines.iter().filter_map(ReportEntry::from_json).collect())
        .unwrap_or_default()
}

/// What a candidate needs in order to be tried. Its `Debug` says what kind it is and never the digest of the content
/// it names.
#[derive(Clone, PartialEq, Eq)]
pub enum Material {
    /// Not tried, for this reason.
    Skip(ReapplyReason),
    /// A signal: the digest of the payload the row names (the output of the step the signal woke).
    Signal { payload_digest: Vec<u8> },
    /// An Update: the revision of the declared state it was applied to in the source.
    Update { before_revision: u64 },
}

impl std::fmt::Debug for Material {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skip(reason) => formatter.debug_tuple("Skip").field(reason).finish(),
            Self::Signal { .. } => formatter.debug_struct("Signal").finish_non_exhaustive(),
            Self::Update { before_revision } => formatter
                .debug_struct("Update")
                .field("before_revision", before_revision)
                .finish(),
        }
    }
}

/// A signal or an Update of the source that a reset may deliver again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The sequence of the row of the source.
    pub sequence: u64,
    pub kind: ReapplyKind,
    /// The name of the signal, or of the operation.
    pub name: String,
    pub material: Material,
}

/// More candidates than a reapply holds ([`REAPPLY_CEILING`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooMany;

/// The signals and Updates of the source after sequence `after`, in journal order, that `policy` delivers
/// again (`FR-018-039`).
///
/// A signal is a `wait.satisfied` of a root step recorded `via: signal`, named by the `eventType` of the wait in
/// the pinned definition. A row that does not say how the wait was satisfied is a [`ReapplyReason::LegacyEvent`]
/// candidate; one satisfied by a published event or by the signal of a Signal-with-Start is not a candidate (a
/// reset must not deliver again what the caller asked for in one request that also started the run, nor what an
/// event did). Several steps woken by one delivery (the same name, the same payload, the same instant) are one
/// signal: two equal rows of one instant are one delivery only when the later step was already waiting when the first
/// row was written (a transaction that delivers two equal signals one after the other, as a reset does, writes rows of
/// one instant, and a step that began to wait after the first is another delivery). When that cannot be shown they
/// are two, so nothing is dropped without being reported. An Update is a `state.updated` that kept its arguments (`retained: true`); one that did not
/// (`retained: false`) is not a candidate, and one written before the marker existed is a legacy candidate.
///
/// Which of them cannot be tried (the payload is gone) is known only once their content is read; the first
/// signal that cannot be delivered ends the reapply of signals ([`end_signals_at_first_skip`]).
///
/// # Errors
/// [`TooMany`] when there are more than [`REAPPLY_CEILING`] candidates.
pub fn candidates(
    def: &WorkflowDefinition,
    journal: &[SourceEvent],
    after: u64,
    policy: ResetReapply,
) -> Result<Vec<Candidate>, TooMany> {
    let mut found: Vec<Candidate> = Vec::new();
    // where each step of the root began to wait, to tell what was waiting when a signal came
    let mut began: HashMap<&NodeId, Vec<u64>> = HashMap::new();
    for row in journal {
        if let RunEvent::WaitStarted { node, .. } = row.event.event()
            && row.event.path().is_root()
        {
            began.entry(node).or_default().push(row.sequence);
        }
    }
    // one delivery of a signal can wake several steps; they are one signal. The value is the first row of the delivery.
    let mut delivered: HashMap<(&str, Option<&[u8]>, OffsetDateTime), u64> = HashMap::new();
    for row in journal.iter().filter(|row| row.sequence > after) {
        if !row.event.path().is_root() {
            continue;
        }
        match row.event.event() {
            RunEvent::WaitSatisfied { node, via } if policy.signals() => {
                let Some(NodeKind::WaitEvent { event_type, .. }) =
                    def.node(node).map(sagat_domain::workflow::Node::kind)
                else {
                    continue;
                };
                let material = match via {
                    Some(WaitVia::Signal) => match &row.payload_digest {
                        Some(digest) => Material::Signal {
                            payload_digest: digest.clone(),
                        },
                        None => Material::Skip(ReapplyReason::NotRetained),
                    },
                    None => Material::Skip(ReapplyReason::LegacyEvent),
                    Some(WaitVia::Event | WaitVia::SignalWithStart) => continue,
                };
                let name = event_type.as_str();
                let key = (name, row.payload_digest.as_deref(), row.occurred_at);
                if let Some(first) = delivered.get(&key) {
                    // the latest time this step began to wait before it was woken
                    let waiting_since = began.get(node).and_then(|starts| {
                        starts.iter().rev().find(|start| **start < row.sequence)
                    });
                    if waiting_since.is_some_and(|start| start < first) {
                        continue;
                    }
                }
                delivered.insert(key, row.sequence);
                found.push(Candidate {
                    sequence: row.sequence,
                    kind: ReapplyKind::Signal,
                    name: name.to_owned(),
                    material,
                });
                if found.len() > REAPPLY_CEILING {
                    return Err(TooMany);
                }
            }
            RunEvent::StateUpdated {
                operation,
                before_revision,
                arguments_retained,
                ..
            } if policy.updates() => {
                let material = match arguments_retained {
                    Some(true) => Material::Update {
                        before_revision: *before_revision,
                    },
                    Some(false) => continue,
                    None => Material::Skip(ReapplyReason::LegacyEvent),
                };
                found.push(Candidate {
                    sequence: row.sequence,
                    kind: ReapplyKind::Update,
                    name: operation.as_str().to_owned(),
                    material,
                });
                if found.len() > REAPPLY_CEILING {
                    return Err(TooMany);
                }
            }
            _ => {}
        }
    }
    Ok(found)
}

/// What a candidate that can be tried is delivered with. Not read for a preview, which only says what will be
/// tried. Its `Debug` says the size and never the content: a payload is content of the tenant.
#[derive(Clone)]
pub enum Delivery {
    /// The payload of the signal, as the source's step kept it.
    Signal { payload: Vec<u8> },
    /// The Update, with the arguments the source kept and the revision of the declared state it was applied to
    /// there: the new run applies it only to a state at that revision.
    Update {
        before_revision: u64,
        arguments: DeclaredOperationArguments,
    },
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Signal { payload } => formatter
                .debug_struct("Signal")
                .field("payload_bytes", &payload.len())
                .finish_non_exhaustive(),
            Self::Update {
                before_revision,
                arguments,
            } => formatter
                .debug_struct("Update")
                .field("before_revision", before_revision)
                .field("arguments", arguments)
                .finish(),
        }
    }
}

/// A candidate with what the reset needs to try it, or the reason it will not.
#[derive(Debug, Clone)]
pub struct PlannedReapply {
    /// `WILL_TRY`, or `NOT_REAPPLIED` with its reason.
    pub entry: ReapplyEntry,
    /// Present for the entries of a reset that will be tried.
    pub delivery: Option<Delivery>,
}

impl PlannedReapply {
    /// A signal with what the tenant keeps of its payload (`keep`: the bytes were asked for, which a reset does and
    /// a preview does not): delivered with it, or the reason it cannot be.
    #[must_use]
    pub fn of_signal(candidate: &Candidate, kept: Option<Retained>, keep: bool) -> Self {
        match kept {
            None => Self::skipped(candidate, ReapplyReason::NotRetained),
            Some(Retained { content, .. }) => match (keep, content) {
                (false, _) => Self::will_try(candidate, None),
                (true, Some(payload)) => {
                    Self::will_try(candidate, Some(Delivery::Signal { payload }))
                }
                (true, None) => Self::skipped(candidate, ReapplyReason::NotRetained),
            },
        }
    }

    /// An Update with what the tenant keeps of its arguments. A preview reads only that they are kept; a reset
    /// reads them, and arguments that do not parse are not retained.
    #[must_use]
    pub fn of_update(
        candidate: &Candidate,
        before_revision: u64,
        kept: Option<Retained>,
        keep: bool,
    ) -> Self {
        match kept {
            None => Self::skipped(candidate, ReapplyReason::NotRetained),
            Some(Retained { content, .. }) => match (keep, content) {
                (false, _) => Self::will_try(candidate, None),
                (true, content) => {
                    match content
                        .and_then(|content| DeclaredOperationArguments::parse(&content).ok())
                    {
                        Some(arguments) => Self::will_try(
                            candidate,
                            Some(Delivery::Update {
                                before_revision,
                                arguments,
                            }),
                        ),
                        None => Self::skipped(candidate, ReapplyReason::NotRetained),
                    }
                }
            },
        }
    }

    /// A candidate that will not be tried.
    #[must_use]
    pub fn skipped(candidate: &Candidate, reason: ReapplyReason) -> Self {
        Self {
            entry: ReapplyEntry {
                sequence: candidate.sequence,
                kind: candidate.kind,
                name: candidate.name.clone(),
                outcome: ReapplyOutcome::NotReapplied,
                reason: Some(reason),
            },
            delivery: None,
        }
    }

    /// A candidate that will be tried; `delivery` is what a reset delivers it with (none for a preview).
    #[must_use]
    pub fn will_try(candidate: &Candidate, delivery: Option<Delivery>) -> Self {
        Self {
            entry: ReapplyEntry {
                sequence: candidate.sequence,
                kind: candidate.kind,
                name: candidate.name.clone(),
                outcome: ReapplyOutcome::WillTry,
                reason: None,
            },
            delivery,
        }
    }
}

/// The first signal that cannot be delivered ends the reapply of signals: every later signal that would be
/// tried is [`ReapplyReason::PreviousSignalNotDelivered`] instead, because delivering it would reach the wait
/// the earlier one was meant for. A signal that has a reason of its own keeps it; Updates are not affected (the
/// revision they were applied to protects them).
pub fn end_signals_at_first_skip(planned: &mut [PlannedReapply]) {
    let mut stopped = false;
    for item in planned
        .iter_mut()
        .filter(|item| item.entry.kind == ReapplyKind::Signal)
    {
        if item.entry.outcome == ReapplyOutcome::NotReapplied {
            stopped = true;
        } else if stopped {
            item.entry.outcome = ReapplyOutcome::NotReapplied;
            item.entry.reason = Some(ReapplyReason::PreviousSignalNotDelivered);
            item.delivery = None;
        }
    }
}

/// What the register says about an Update the new run resolved: applied, or refused for the reason its receipt
/// keeps. `None` for a receipt that contradicts itself (a rejected Update without a rejection), which is a
/// defect and not an answer.
#[must_use]
pub fn update_answer(
    status: UpdateReceiptStatus,
    rejection: Option<&UpdateRejection>,
) -> Option<(ReapplyOutcome, Option<ReapplyReason>)> {
    match (status, rejection) {
        (UpdateReceiptStatus::Applied, _) => Some((ReapplyOutcome::Reapplied, None)),
        (UpdateReceiptStatus::Rejected, Some(UpdateRejection::RevisionConflict { .. })) => Some((
            ReapplyOutcome::Rejected,
            Some(ReapplyReason::RevisionConflict),
        )),
        (UpdateReceiptStatus::Rejected, Some(UpdateRejection::ValidatorRejected)) => Some((
            ReapplyOutcome::Rejected,
            Some(ReapplyReason::UpdateRejected),
        )),
        (UpdateReceiptStatus::Rejected, None) => None,
    }
}

/// The reason an Update was refused before it could be decided, when that is a business answer (the declaration
/// does not accept the arguments or the state they would make); `None` for a failure of the platform, which
/// aborts the whole reset. The Updates the `updateRun` route answers `422` are the first kind.
#[must_use]
pub fn update_refusal(error: &DeclaredOperationStoreError) -> Option<ReapplyReason> {
    match error {
        DeclaredOperationStoreError::Operation(
            OperationError::InvalidArguments
            | OperationError::EvaluationFailed
            | OperationError::ValueTooLarge
            | OperationError::StateSchemaChanged
            | OperationError::UnsupportedDisclosure,
        ) => Some(ReapplyReason::UpdateNotAccepted),
        _ => None,
    }
}

/// Counts what a reset delivered again, once the reset is committed: `workflow_reset_reapply_total{kind,outcome,reason}`.
/// The labels are the closed names of the report (`reason` is `none` for an entry without one); none is a tenant, a run,
/// a signal or an operation. Nothing is counted for a reset that was not made.
pub fn count_reapply(report: &[ReapplyEntry]) {
    for entry in report {
        metrics::counter!(
            "workflow_reset_reapply_total",
            "kind" => entry.kind.as_str(),
            "outcome" => entry.outcome.as_str(),
            "reason" => entry.reason.map_or("none", ReapplyReason::as_str)
        )
        .increment(1);
    }
}

/// What the platform's own keys of a reset start with, as an `Idempotency-Key` and as an update key: the API
/// refuses it to callers, the way it refuses `batch:` and `schedule:`.
pub const RESET_KEY_PREFIX: &str = "reset:";

/// The key under which the platform delivers the candidate at `sequence` of the source again, for the reset
/// `reset`: stable for the reset, distinct for every candidate, and in the prefix callers cannot use.
#[must_use]
pub fn reapply_key(reset: ResetId, sequence: u64) -> String {
    format!("{RESET_KEY_PREFIX}{reset}:{sequence}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use sagat_domain::execution::{AddressedRunEvent, RunEvent};
    use sagat_domain::identity::Slug;
    use sagat_domain::workflow::{
        ActivityName, Digest, EventType, Expression, Node, NodeId, RetryPolicy,
    };
    use std::time::Duration as StdDuration;
    use time::Duration;

    use super::*;

    fn entry(
        kind: ReapplyKind,
        outcome: ReapplyOutcome,
        reason: Option<ReapplyReason>,
    ) -> ReapplyEntry {
        ReapplyEntry {
            sequence: 7,
            kind,
            name: "payment.confirmed".to_owned(),
            outcome,
            reason,
        }
    }

    #[test]
    fn every_closed_name_round_trips_and_an_unknown_one_is_none() {
        for kind in [ReapplyKind::Signal, ReapplyKind::Update] {
            assert_eq!(ReapplyKind::parse(kind.as_str()), Some(kind));
        }
        for outcome in [
            ReapplyOutcome::WillTry,
            ReapplyOutcome::Reapplied,
            ReapplyOutcome::Rejected,
            ReapplyOutcome::NotReapplied,
        ] {
            assert_eq!(ReapplyOutcome::parse(outcome.as_str()), Some(outcome));
        }
        for reason in [
            ReapplyReason::LegacyEvent,
            ReapplyReason::NotRetained,
            ReapplyReason::PreviousSignalNotDelivered,
            ReapplyReason::NotWaiting,
            ReapplyReason::UpdateRejected,
            ReapplyReason::RevisionConflict,
            ReapplyReason::UpdateNotAccepted,
        ] {
            assert_eq!(ReapplyReason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(ReapplyKind::parse("signal"), None);
        assert_eq!(ReapplyOutcome::parse("DONE"), None);
        assert_eq!(ReapplyReason::parse(""), None);
        // the names are the contract's
        assert_eq!(
            [
                ReapplyKind::Signal.as_str(),
                ReapplyKind::Update.as_str(),
                ReapplyOutcome::WillTry.as_str(),
                ReapplyOutcome::Reapplied.as_str(),
                ReapplyOutcome::Rejected.as_str(),
                ReapplyOutcome::NotReapplied.as_str(),
                ReapplyReason::LegacyEvent.as_str(),
                ReapplyReason::NotRetained.as_str(),
                ReapplyReason::PreviousSignalNotDelivered.as_str(),
                ReapplyReason::NotWaiting.as_str(),
                ReapplyReason::UpdateRejected.as_str(),
                ReapplyReason::RevisionConflict.as_str(),
                ReapplyReason::UpdateNotAccepted.as_str(),
            ],
            [
                "SIGNAL",
                "UPDATE",
                "WILL_TRY",
                "REAPPLIED",
                "REJECTED",
                "NOT_REAPPLIED",
                "LEGACY_EVENT",
                "NOT_RETAINED",
                "PREVIOUS_SIGNAL_NOT_DELIVERED",
                "NOT_WAITING",
                "UPDATE_REJECTED",
                "REVISION_CONFLICT",
                "UPDATE_NOT_ACCEPTED",
            ]
        );
    }

    #[test]
    fn a_signal_is_named_and_an_update_has_an_operation_and_a_reason_is_there_only_when_there_is_one()
     {
        assert_eq!(
            entry(ReapplyKind::Signal, ReapplyOutcome::Reapplied, None).to_json(),
            json!({"sequence": 7, "kind": "SIGNAL", "name": "payment.confirmed", "outcome": "REAPPLIED"})
        );
        assert_eq!(
            entry(
                ReapplyKind::Update,
                ReapplyOutcome::Rejected,
                Some(ReapplyReason::RevisionConflict)
            )
            .to_json(),
            json!({"sequence": 7, "kind": "UPDATE", "operation": "payment.confirmed",
                   "outcome": "REJECTED", "reason": "REVISION_CONFLICT"})
        );
    }

    #[test]
    fn a_report_reads_back_as_it_was_written() {
        let entries = vec![
            entry(ReapplyKind::Signal, ReapplyOutcome::Reapplied, None),
            ReapplyEntry {
                kind: ReapplyKind::Update,
                name: "balance.replace".to_owned(),
                outcome: ReapplyOutcome::NotReapplied,
                reason: Some(ReapplyReason::NotRetained),
                ..entry(ReapplyKind::Update, ReapplyOutcome::Reapplied, None)
            },
        ];
        let read = report_from_json(&report_json(&entries));
        assert_eq!(
            read,
            entries.iter().map(ReportEntry::from).collect::<Vec<_>>()
        );
        assert_eq!(read[0].subject(), Some("payment.confirmed"));
        assert_eq!(read[1].subject(), Some("balance.replace"));
        assert_eq!(
            (
                read[1].kind.as_str(),
                read[1].outcome.as_str(),
                read[1].reason.as_deref()
            ),
            ("UPDATE", "NOT_REAPPLIED", Some("NOT_RETAINED"))
        );
        assert_eq!(report_from_json(&json!([])), Vec::new());
    }

    #[test]
    fn what_another_version_wrote_is_read_as_written_and_never_fails() {
        // a kind, an outcome and a reason this version does not know are kept as the strings they are
        let read = report_from_json(&json!([
            {"sequence": 1, "kind": "QUERY", "operation": "later", "outcome": "DONE", "reason": "FUTURE", "extra": 1},
            {"sequence": 2, "kind": "SIGNAL", "name": "x", "outcome": "REAPPLIED", "reason": null},
        ]));
        assert_eq!(read.len(), 2);
        assert_eq!(
            (
                read[0].kind.as_str(),
                read[0].outcome.as_str(),
                read[0].reason.as_deref()
            ),
            ("QUERY", "DONE", Some("FUTURE"))
        );
        assert_eq!(read[0].subject(), Some("later"));
        // an explicit null reason is no reason
        assert_eq!(read[1].reason, None);
        // a line that is not a line is left out, and what is not a list is an empty report
        for bad in [
            json!({"kind": "SIGNAL", "name": "x", "outcome": "REAPPLIED"}),
            json!({"sequence": "1", "kind": "SIGNAL", "name": "x", "outcome": "REAPPLIED"}),
            json!({"sequence": 1, "name": "x", "outcome": "REAPPLIED"}),
            json!({"sequence": 1, "kind": "SIGNAL", "name": "x"}),
            json!("a string"),
            json!(3),
        ] {
            assert_eq!(report_from_json(&json!([bad.clone()])), Vec::new(), "{bad}");
        }
        for not_a_list in [json!({}), json!(null), json!("x")] {
            assert_eq!(report_from_json(&not_a_list), Vec::new());
        }
        // a line without a name or an operation is a line without a subject
        let nameless =
            report_from_json(&json!([{"sequence": 1, "kind": "SIGNAL", "outcome": "REAPPLIED"}]));
        assert_eq!(nameless[0].subject(), None);
        // a line that has both (no version writes one) is about the signal: the name comes first
        let both = report_from_json(&json!([{
            "sequence": 1, "kind": "SIGNAL", "name": "the-signal", "operation": "the-operation",
            "outcome": "REAPPLIED"
        }]));
        assert_eq!(both[0].subject(), Some("the-signal"));
    }

    #[test]
    fn the_worst_report_fits_the_register() {
        // the ceiling is the contract's `maxItems` and the register's bound, and the arithmetic below depends on it
        assert_eq!(REAPPLY_CEILING, 256);
        // 256 entries of the longest names the journal allows, with the longest reason, as `jsonb` writes
        // them as text (a space after each `:` and `,`): under the 65,536 bytes the register accepts
        let longest = ReapplyEntry {
            sequence: 99_999_999,
            kind: ReapplyKind::Signal,
            name: "x".repeat(128),
            outcome: ReapplyOutcome::NotReapplied,
            reason: Some(ReapplyReason::PreviousSignalNotDelivered),
        };
        let compact = longest.to_json().to_string().len();
        // five keys: five spaces after a colon, four after a comma, and one comma between entries
        let as_jsonb = compact + 5 + 4 + 2;
        assert!(
            as_jsonb * REAPPLY_CEILING < 65_536,
            "{as_jsonb} bytes an entry"
        );
    }

    // ---- the candidates of a journal

    fn node(name: &str) -> NodeId {
        NodeId::parse(name).expect("a node")
    }

    /// `a` is an activity; `w` and `w2` wait for `payment.confirmed` and `v` for `payment.settled`.
    fn definition() -> WorkflowDefinition {
        let waiting = |id: &str, event: &str, next: Option<&str>| {
            let node = Node::new(
                node(id),
                NodeKind::WaitEvent {
                    event_type: EventType::parse(event).expect("an event type"),
                    correlation: Expression::parse("input.k").expect("an expression"),
                    timeout: StdDuration::from_secs(60),
                    on_timeout: None,
                    lookback: StdDuration::ZERO,
                },
            );
            match next {
                Some(next) => node.with_next(self::node(next)),
                None => node,
            }
        };
        WorkflowDefinition::new(
            Slug::parse("reapply-flow").expect("a slug"),
            node("a"),
            vec![
                Node::new(
                    node("a"),
                    NodeKind::Activity {
                        activity: ActivityName::parse("act.a").expect("an activity"),
                        retry: RetryPolicy::DEFAULT,
                        compensate_with: None,
                        reconcile_with: None,
                        idempotency_template: None,
                        secrets: Vec::new(),
                    },
                )
                .with_next(node("w")),
                waiting("w", "payment.confirmed", Some("v")),
                waiting("v", "payment.settled", Some("w2")),
                waiting("w2", "payment.confirmed", None),
            ],
        )
        .expect("a valid definition")
    }

    fn row(sequence: u64, event: RunEvent, digest: Option<&[u8]>, at_us: i64) -> SourceEvent {
        SourceEvent {
            sequence,
            event: AddressedRunEvent::root(event),
            event_type: String::new(),
            node: None,
            attempt: None,
            detail: json!({}),
            actor: "system".into(),
            trace_id: None,
            occurred_at: OffsetDateTime::UNIX_EPOCH + Duration::microseconds(at_us),
            payload_digest: digest.map(<[u8]>::to_vec),
            canonical: Vec::new(),
        }
    }

    fn satisfied(
        sequence: u64,
        node_id: &str,
        via: Option<WaitVia>,
        digest: Option<&[u8]>,
    ) -> SourceEvent {
        row(
            sequence,
            RunEvent::WaitSatisfied {
                node: node(node_id),
                via,
            },
            digest,
            i64::try_from(sequence).expect("small") * 1_000,
        )
    }

    /// The row where a step began to wait.
    fn waiting(sequence: u64, node_id: &str) -> SourceEvent {
        row(
            sequence,
            RunEvent::WaitStarted {
                node: node(node_id),
                correlation_key: sagat_domain::execution::CorrelationKey::parse("k")
                    .expect("a key"),
                deadline: OffsetDateTime::UNIX_EPOCH,
            },
            None,
            i64::try_from(sequence).expect("small") * 1_000,
        )
    }

    /// A wait satisfied by a signal at the instant `at_us`, which every row of one transaction shares.
    fn woken(sequence: u64, node_id: &str, digest: &[u8], at_us: i64) -> SourceEvent {
        row(
            sequence,
            RunEvent::WaitSatisfied {
                node: node(node_id),
                via: Some(WaitVia::Signal),
            },
            Some(digest),
            at_us,
        )
    }

    fn updated(sequence: u64, before: u64, retained: Option<bool>) -> SourceEvent {
        row(
            sequence,
            RunEvent::StateUpdated {
                operation: ActivityName::parse("balance.replace").expect("an operation"),
                before_revision: before,
                after_revision: before + 1,
                state_digest: Digest::new([0; 32]),
                arguments_retained: retained,
            },
            None,
            i64::try_from(sequence).expect("small") * 1_000,
        )
    }

    fn summary(found: &[Candidate]) -> Vec<(u64, &'static str, &str, String)> {
        found
            .iter()
            .map(|candidate| {
                (
                    candidate.sequence,
                    candidate.kind.as_str(),
                    candidate.name.as_str(),
                    match &candidate.material {
                        Material::Skip(reason) => reason.as_str().to_owned(),
                        Material::Signal { .. } => "signal".to_owned(),
                        Material::Update { before_revision } => format!("update@{before_revision}"),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn the_policy_chooses_signals_updates_both_or_nothing_and_the_order_is_the_journal_s() {
        let journal = vec![
            satisfied(3, "w", Some(WaitVia::Signal), Some(b"p1")),
            updated(4, 0, Some(true)),
            satisfied(5, "v", Some(WaitVia::Signal), Some(b"p2")),
            updated(6, 1, Some(true)),
        ];
        let def = definition();
        let all = candidates(&def, &journal, 2, ResetReapply::All).expect("candidates");
        assert_eq!(
            summary(&all),
            [
                (3, "SIGNAL", "payment.confirmed", "signal".to_owned()),
                (4, "UPDATE", "balance.replace", "update@0".to_owned()),
                (5, "SIGNAL", "payment.settled", "signal".to_owned()),
                (6, "UPDATE", "balance.replace", "update@1".to_owned()),
            ]
        );
        let signals = candidates(&def, &journal, 2, ResetReapply::Signals).expect("candidates");
        assert_eq!(
            signals.iter().map(|c| c.sequence).collect::<Vec<_>>(),
            [3, 5]
        );
        let updates = candidates(&def, &journal, 2, ResetReapply::Updates).expect("candidates");
        assert_eq!(
            updates.iter().map(|c| c.sequence).collect::<Vec<_>>(),
            [4, 6]
        );
        assert!(
            candidates(&def, &journal, 2, ResetReapply::None)
                .expect("candidates")
                .is_empty()
        );
    }

    #[test]
    fn only_what_comes_after_the_point_is_a_candidate_and_the_row_of_the_point_is_not() {
        let journal = vec![
            satisfied(3, "w", Some(WaitVia::Signal), Some(b"p1")),
            updated(4, 0, Some(true)),
            satisfied(5, "v", Some(WaitVia::Signal), Some(b"p2")),
        ];
        let def = definition();
        for (after, expected) in [
            (2, vec![3, 4, 5]),
            (3, vec![4, 5]),
            (4, vec![5]),
            (5, vec![]),
        ] {
            let found = candidates(&def, &journal, after, ResetReapply::All).expect("candidates");
            assert_eq!(
                found.iter().map(|c| c.sequence).collect::<Vec<_>>(),
                expected,
                "after {after}"
            );
        }
    }

    #[test]
    fn a_wait_an_event_satisfied_and_the_signal_of_a_signal_with_start_are_not_candidates() {
        let journal = vec![
            satisfied(3, "w", Some(WaitVia::Event), Some(b"p1")),
            satisfied(4, "v", Some(WaitVia::SignalWithStart), Some(b"p2")),
            satisfied(5, "w", Some(WaitVia::Signal), Some(b"p3")),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Signals).expect("found");
        assert_eq!(found.iter().map(|c| c.sequence).collect::<Vec<_>>(), [5]);
    }

    #[test]
    fn a_row_that_does_not_say_how_the_wait_was_satisfied_is_a_legacy_candidate() {
        let journal = vec![
            satisfied(3, "w", None, Some(b"p1")),
            satisfied(4, "v", Some(WaitVia::Signal), Some(b"p2")),
            updated(5, 0, Some(true)),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::All).expect("found");
        assert_eq!(
            summary(&found),
            [
                (3, "SIGNAL", "payment.confirmed", "LEGACY_EVENT".to_owned()),
                (4, "SIGNAL", "payment.settled", "signal".to_owned()),
                (5, "UPDATE", "balance.replace", "update@0".to_owned()),
            ]
        );
    }

    #[test]
    fn a_signal_whose_payload_is_not_named_is_not_retained() {
        let journal = vec![
            satisfied(3, "w", Some(WaitVia::Signal), None),
            satisfied(4, "v", Some(WaitVia::Signal), Some(b"p2")),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Signals).expect("found");
        assert_eq!(
            summary(&found),
            [
                (3, "SIGNAL", "payment.confirmed", "NOT_RETAINED".to_owned()),
                (4, "SIGNAL", "payment.settled", "signal".to_owned()),
            ]
        );
    }

    fn planned(sequence: u64, kind: ReapplyKind, reason: Option<ReapplyReason>) -> PlannedReapply {
        let candidate = Candidate {
            sequence,
            kind,
            name: "n".to_owned(),
            material: Material::Skip(ReapplyReason::LegacyEvent),
        };
        match reason {
            Some(reason) => PlannedReapply::skipped(&candidate, reason),
            None => PlannedReapply::will_try(
                &candidate,
                Some(Delivery::Signal {
                    payload: b"{}".to_vec(),
                }),
            ),
        }
    }

    fn outcomes(
        planned: &[PlannedReapply],
    ) -> Vec<(u64, &'static str, Option<&'static str>, bool)> {
        planned
            .iter()
            .map(|item| {
                (
                    item.entry.sequence,
                    item.entry.outcome.as_str(),
                    item.entry.reason.map(ReapplyReason::as_str),
                    item.delivery.is_some(),
                )
            })
            .collect()
    }

    #[test]
    fn the_first_signal_that_cannot_be_delivered_ends_the_signals_and_not_the_updates() {
        let mut list = vec![
            planned(3, ReapplyKind::Signal, None),
            planned(4, ReapplyKind::Signal, Some(ReapplyReason::NotRetained)),
            planned(5, ReapplyKind::Update, None),
            planned(6, ReapplyKind::Signal, None),
            // it has a reason of its own, and keeps it
            planned(7, ReapplyKind::Signal, Some(ReapplyReason::LegacyEvent)),
            planned(8, ReapplyKind::Update, None),
        ];
        end_signals_at_first_skip(&mut list);
        assert_eq!(
            outcomes(&list),
            [
                (3, "WILL_TRY", None, true),
                (4, "NOT_REAPPLIED", Some("NOT_RETAINED"), false),
                (5, "WILL_TRY", None, true),
                (
                    6,
                    "NOT_REAPPLIED",
                    Some("PREVIOUS_SIGNAL_NOT_DELIVERED"),
                    false
                ),
                (7, "NOT_REAPPLIED", Some("LEGACY_EVENT"), false),
                (8, "WILL_TRY", None, true),
            ]
        );
    }

    #[test]
    fn an_update_that_cannot_be_tried_stops_no_signal_and_signals_that_can_all_go() {
        let mut list = vec![
            planned(3, ReapplyKind::Update, Some(ReapplyReason::NotRetained)),
            planned(4, ReapplyKind::Signal, None),
            planned(5, ReapplyKind::Signal, None),
        ];
        end_signals_at_first_skip(&mut list);
        assert_eq!(
            outcomes(&list),
            [
                (3, "NOT_REAPPLIED", Some("NOT_RETAINED"), false),
                (4, "WILL_TRY", None, true),
                (5, "WILL_TRY", None, true),
            ]
        );
    }

    #[test]
    fn an_update_the_new_run_decided_is_reapplied_or_rejected_for_the_reason_of_its_receipt() {
        assert_eq!(
            update_answer(UpdateReceiptStatus::Applied, None),
            Some((ReapplyOutcome::Reapplied, None))
        );
        assert_eq!(
            update_answer(
                UpdateReceiptStatus::Rejected,
                Some(&UpdateRejection::ValidatorRejected)
            ),
            Some((
                ReapplyOutcome::Rejected,
                Some(ReapplyReason::UpdateRejected)
            ))
        );
        assert_eq!(
            update_answer(
                UpdateReceiptStatus::Rejected,
                Some(&UpdateRejection::RevisionConflict {
                    expected: 1,
                    actual: 0
                })
            ),
            Some((
                ReapplyOutcome::Rejected,
                Some(ReapplyReason::RevisionConflict)
            ))
        );
        // a rejected receipt without a rejection contradicts itself
        assert_eq!(update_answer(UpdateReceiptStatus::Rejected, None), None);
    }

    #[test]
    fn only_a_declaration_that_says_no_is_a_business_answer_and_the_failures_of_the_platform_abort()
    {
        for answer in [
            OperationError::InvalidArguments,
            OperationError::EvaluationFailed,
            OperationError::ValueTooLarge,
            OperationError::StateSchemaChanged,
            OperationError::UnsupportedDisclosure,
        ] {
            assert_eq!(
                update_refusal(&DeclaredOperationStoreError::Operation(answer.clone())),
                Some(ReapplyReason::UpdateNotAccepted),
                "{answer}"
            );
        }
        for failure in [
            DeclaredOperationStoreError::Operation(OperationError::InvalidStoredOperation),
            DeclaredOperationStoreError::Operation(OperationError::UpdateKeyReused),
            DeclaredOperationStoreError::Operation(OperationError::RunContinued),
            DeclaredOperationStoreError::Operation(OperationError::RevisionExhausted),
            DeclaredOperationStoreError::Operation(OperationError::InvalidUpdateKey),
            DeclaredOperationStoreError::NotFound,
            DeclaredOperationStoreError::Forbidden,
            DeclaredOperationStoreError::DefinitionUnavailable,
            DeclaredOperationStoreError::AuditMismatch,
            DeclaredOperationStoreError::AddressUnavailable,
            DeclaredOperationStoreError::ChainHeadChanged,
        ] {
            assert_eq!(update_refusal(&failure), None, "{failure}");
        }
    }

    #[test]
    fn a_candidate_never_prints_the_digest_of_its_content() {
        let candidate = Candidate {
            sequence: 7,
            kind: ReapplyKind::Signal,
            name: "payment.confirmed".to_owned(),
            material: Material::Signal {
                payload_digest: vec![111, 222, 173, 190],
            },
        };
        let printed = format!("{candidate:?} {:?}", candidate.material);
        for digit in ["111", "222", "173", "190"] {
            assert!(!printed.contains(digit), "{printed}");
        }
        assert!(printed.contains("payment.confirmed"), "{printed}");
        // what it is stays readable: the kind of material, the revision of an Update, the reason of a skip
        assert!(
            format!("{:?}", Material::Update { before_revision: 3 }).contains("before_revision: 3")
        );
        assert!(
            format!("{:?}", Material::Skip(ReapplyReason::NotRetained)).contains("NotRetained")
        );
    }

    #[test]
    fn what_is_delivered_again_is_never_printed() {
        let signal = Delivery::Signal {
            payload: br#"{"card":"4111111111111111"}"#.to_vec(),
        };
        let update = Delivery::Update {
            before_revision: 3,
            arguments: DeclaredOperationArguments::parse(br#"{"secret":"hunter2"}"#)
                .expect("arguments"),
        };
        let printed = format!(
            "{signal:?} {update:?} {:?}",
            planned(1, ReapplyKind::Signal, None)
        );
        assert!(
            !printed.contains("4111") && !printed.contains("hunter2"),
            "{printed}"
        );
        assert!(
            printed.contains("payload_bytes") && printed.contains("before_revision: 3"),
            "{printed}"
        );
    }

    #[test]
    fn the_key_of_a_candidate_is_the_reset_s_and_the_sequence_s_in_the_prefix_callers_cannot_use() {
        let reset = ResetId::generate();
        let key = reapply_key(reset, 42);
        assert_eq!(key, format!("reset:{reset}:42"));
        assert!(key.starts_with(RESET_KEY_PREFIX));
        assert_ne!(key, reapply_key(reset, 43));
        assert_ne!(key, reapply_key(ResetId::generate(), 42));
        // a valid update key: printable ASCII, no spaces, at most 128 characters
        assert!(sagat_domain::execution::UpdateKey::parse(&reapply_key(reset, u64::MAX)).is_ok());
    }

    #[test]
    fn an_update_that_did_not_keep_its_arguments_is_not_a_candidate_and_one_that_says_nothing_is_legacy()
     {
        let journal = vec![
            updated(3, 0, Some(false)),
            updated(4, 0, None),
            updated(5, 1, Some(true)),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Updates).expect("found");
        assert_eq!(
            summary(&found),
            [
                (4, "UPDATE", "balance.replace", "LEGACY_EVENT".to_owned()),
                // an Update that follows one that could not be reapplied is still tried: its revision decides
                (5, "UPDATE", "balance.replace", "update@1".to_owned()),
            ]
        );
    }

    #[test]
    fn the_steps_one_signal_woke_are_one_signal_and_a_second_signal_of_the_same_name_is_another() {
        let journal = vec![
            // both steps were waiting when the signal came
            waiting(1, "w"),
            waiting(2, "w2"),
            // one delivery that woke two steps waiting for the same name: same name, payload and instant
            row(
                3,
                RunEvent::WaitSatisfied {
                    node: node("w"),
                    via: Some(WaitVia::Signal),
                },
                Some(b"p1"),
                5_000,
            ),
            row(
                4,
                RunEvent::WaitSatisfied {
                    node: node("w2"),
                    via: Some(WaitVia::Signal),
                },
                Some(b"p1"),
                5_000,
            ),
            // the same name and payload at another instant is another delivery
            row(
                5,
                RunEvent::WaitSatisfied {
                    node: node("w2"),
                    via: Some(WaitVia::Signal),
                },
                Some(b"p1"),
                6_000,
            ),
            // the same name and instant with another payload is another delivery
            row(
                6,
                RunEvent::WaitSatisfied {
                    node: node("w2"),
                    via: Some(WaitVia::Signal),
                },
                Some(b"p2"),
                6_000,
            ),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Signals).expect("found");
        assert_eq!(
            found.iter().map(|c| c.sequence).collect::<Vec<_>>(),
            [3, 5, 6]
        );
    }

    #[test]
    fn two_signals_of_one_transaction_that_reached_consecutive_waits_are_two_signals() {
        // the reset of a reset: the first reset delivered two equal signals in the transaction that created its run,
        // so both rows have one instant and one payload. `w2` began to wait only after the first signal woke `w`:
        // the second signal is another delivery, and it is not dropped.
        let journal = vec![
            waiting(3, "w"),
            woken(4, "w", b"p1", 9_000),
            waiting(5, "w2"),
            woken(6, "w2", b"p1", 9_000),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Signals).expect("found");
        assert_eq!(found.iter().map(|c| c.sequence).collect::<Vec<_>>(), [4, 6]);
    }

    #[test]
    fn steps_that_were_all_waiting_when_the_signal_came_are_one_signal() {
        let journal = vec![
            waiting(3, "w"),
            waiting(4, "w2"),
            // one delivery woke both
            woken(5, "w", b"p1", 9_000),
            woken(6, "w2", b"p1", 9_000),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Signals).expect("found");
        assert_eq!(found.iter().map(|c| c.sequence).collect::<Vec<_>>(), [5]);
    }

    #[test]
    fn a_wait_whose_beginning_is_not_in_the_journal_is_never_merged_into_another_signal() {
        // nothing is dropped without being reported: when it cannot be shown that two rows were one delivery they are two
        let journal = vec![woken(3, "w", b"p1", 9_000), woken(4, "w2", b"p1", 9_000)];
        let found = candidates(&definition(), &journal, 1, ResetReapply::Signals).expect("found");
        assert_eq!(found.iter().map(|c| c.sequence).collect::<Vec<_>>(), [3, 4]);
    }

    #[test]
    fn what_is_not_of_the_root_and_a_wait_that_is_not_an_event_wait_are_not_candidates() {
        let mut scoped = satisfied(3, "w", Some(WaitVia::Signal), Some(b"p1"));
        scoped.event = AddressedRunEvent::in_path(
            sagat_domain::execution::InstancePath::root()
                .child(node("loop"), 0)
                .expect("a path"),
            RunEvent::WaitSatisfied {
                node: node("w"),
                via: Some(WaitVia::Signal),
            },
        )
        .expect("an address");
        let journal = vec![
            scoped,
            // `a` is an activity, not a wait for an event
            satisfied(4, "a", Some(WaitVia::Signal), Some(b"p2")),
            // and a node the definition does not have
            satisfied(5, "nowhere", Some(WaitVia::Signal), Some(b"p3")),
        ];
        let found = candidates(&definition(), &journal, 1, ResetReapply::All).expect("found");
        assert!(found.is_empty(), "{found:?}");
    }

    fn candidate_of(kind: ReapplyKind, material: Material) -> Candidate {
        Candidate {
            sequence: 7,
            kind,
            name: "the-name".into(),
            material,
        }
    }

    fn kept(content: Option<&[u8]>) -> Option<Retained> {
        Some(Retained {
            size: 3,
            content: content.map(<[u8]>::to_vec),
        })
    }

    #[test]
    fn a_signal_is_delivered_with_the_bytes_the_store_gave_and_never_with_bytes_it_did_not() {
        let candidate = candidate_of(
            ReapplyKind::Signal,
            Material::Signal {
                payload_digest: vec![1; 32],
            },
        );
        let tried = |planned: &PlannedReapply| planned.entry.outcome == ReapplyOutcome::WillTry;
        let not_retained = |planned: &PlannedReapply| {
            planned.entry.outcome == ReapplyOutcome::NotReapplied
                && planned.entry.reason == Some(ReapplyReason::NotRetained)
                && planned.delivery.is_none()
        };
        // a reset delivers what it read
        let planned = PlannedReapply::of_signal(&candidate, kept(Some(b"abc")), true);
        assert!(tried(&planned));
        assert!(
            matches!(&planned.delivery, Some(Delivery::Signal { payload }) if payload == b"abc")
        );
        // a preview does not carry the bytes, whatever the store gave
        let planned = PlannedReapply::of_signal(&candidate, kept(Some(b"abc")), false);
        assert!(tried(&planned) && planned.delivery.is_none());
        let planned = PlannedReapply::of_signal(&candidate, kept(None), false);
        assert!(tried(&planned) && planned.delivery.is_none());
        // the bytes a reset asked for and did not get are not delivered as an empty payload
        assert!(not_retained(&PlannedReapply::of_signal(
            &candidate,
            kept(None),
            true
        )));
        // and what the tenant does not keep at all is not tried, by either
        for keep in [true, false] {
            assert!(not_retained(&PlannedReapply::of_signal(
                &candidate, None, keep
            )));
        }
    }

    #[test]
    fn an_update_is_delivered_with_arguments_that_read_and_not_retained_otherwise() {
        let candidate = candidate_of(ReapplyKind::Update, Material::Update { before_revision: 4 });
        let not_retained = |planned: &PlannedReapply| {
            planned.entry.outcome == ReapplyOutcome::NotReapplied
                && planned.entry.reason == Some(ReapplyReason::NotRetained)
                && planned.delivery.is_none()
        };
        let planned = PlannedReapply::of_update(&candidate, 4, kept(Some(br#"{"n":1}"#)), true);
        assert_eq!(planned.entry.outcome, ReapplyOutcome::WillTry);
        assert!(matches!(
            planned.delivery,
            Some(Delivery::Update {
                before_revision: 4,
                ..
            })
        ));
        // a preview says they are kept and does not carry them
        let planned = PlannedReapply::of_update(&candidate, 4, kept(Some(br#"{"n":1}"#)), false);
        assert_eq!(planned.entry.outcome, ReapplyOutcome::WillTry);
        assert!(planned.delivery.is_none());
        // arguments a reset reads and that do not read as arguments, bytes it did not get, and nothing kept
        assert!(not_retained(&PlannedReapply::of_update(
            &candidate,
            4,
            kept(Some(b"not json")),
            true
        )));
        assert!(not_retained(&PlannedReapply::of_update(
            &candidate,
            4,
            kept(None),
            true
        )));
        for keep in [true, false] {
            assert!(not_retained(&PlannedReapply::of_update(
                &candidate, 4, None, keep
            )));
        }
    }

    #[test]
    fn the_ceiling_holds_for_signals_as_it_does_for_updates() {
        let def = definition();
        // a signal each, told apart by their payloads and their instants (they are not one delivery)
        let rows = |count: usize| -> Vec<SourceEvent> {
            (0..count)
                .map(|n| {
                    satisfied(
                        10 + n as u64,
                        "w",
                        Some(WaitVia::Signal),
                        Some(format!("payload-{n}").as_bytes()),
                    )
                })
                .collect()
        };
        assert_eq!(
            candidates(&def, &rows(REAPPLY_CEILING), 1, ResetReapply::Signals)
                .expect("at the ceiling")
                .len(),
            REAPPLY_CEILING
        );
        assert_eq!(
            candidates(&def, &rows(REAPPLY_CEILING + 1), 1, ResetReapply::Signals),
            Err(TooMany)
        );
        // the ones that will only be reported (their content is gone) count as well
        let contentless: Vec<SourceEvent> = (0..=REAPPLY_CEILING)
            .map(|n| satisfied(10 + n as u64, "w", Some(WaitVia::Signal), None))
            .collect();
        assert_eq!(
            candidates(&def, &contentless, 1, ResetReapply::All),
            Err(TooMany)
        );
    }

    #[test]
    fn the_ceiling_is_the_most_a_reapply_holds_and_one_more_refuses() {
        let def = definition();
        let rows = |count: usize| -> Vec<SourceEvent> {
            (0..count)
                .map(|n| updated(10 + n as u64, n as u64, Some(true)))
                .collect()
        };
        assert_eq!(
            candidates(&def, &rows(REAPPLY_CEILING), 1, ResetReapply::Updates)
                .expect("at the ceiling")
                .len(),
            REAPPLY_CEILING
        );
        assert_eq!(
            candidates(&def, &rows(REAPPLY_CEILING + 1), 1, ResetReapply::Updates),
            Err(TooMany)
        );
        // the skipped ones count too: the report lists them
        let legacy: Vec<SourceEvent> = (0..=REAPPLY_CEILING)
            .map(|n| updated(10 + n as u64, 0, None))
            .collect();
        assert_eq!(
            candidates(&def, &legacy, 1, ResetReapply::Updates),
            Err(TooMany)
        );
        // and what the policy leaves out does not
        assert!(
            candidates(&def, &rows(REAPPLY_CEILING + 1), 1, ResetReapply::Signals)
                .expect("none")
                .is_empty()
        );
    }
}
