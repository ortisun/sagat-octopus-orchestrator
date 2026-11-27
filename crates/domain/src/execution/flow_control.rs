//! The journal of flow control (feature 022, `contracts/journal-events.md`): the five `concurrency.*` and the five
//! `rate_limit.*` events, and the state they leave on a run and on its steps.
//!
//! Every decision is a journal fact committed with the transition it causes (FR-022-045): replay reads the events and
//! never a clock, a window or a queue. What an event carries is closed and payload-free: the pool, the scope, the
//! resolved group, the key's 12-hex label (never its value, D1168) and, for a step, the attempt the decision is for
//! (the step's current attempt plus one, D022-029).
//!
//! State machine of one gate of one run or one attempt of one step:
//!
//! ```text
//! (none) --queued--> Queued --acquired/granted--> Held --released (permit) | returned (unleased slot)--> Ended
//!   |                  |--wait_timed_out--> TimedOut
//!   |                  '--dequeued-------> Dequeued
//!   '--acquired/granted (the key had room at once)--> Held
//! ```
//!
//! A run whose entry waits for a gate holds it, as a start delay does: nothing is released until every gate it waits
//! for is granted. A run cannot finish while it waits for a gate or holds a run permit: the planner dequeues a waiter
//! that is stopped and releases the permit before the run's last event (FR-022-021, ADR-0056 decision 4).
//!
//! Readiness (T022-096, FR-022-051, D022-020): a state that knows its definition's gated nodes
//! ([`super::RunState::with_gated_steps`]) maps `step.released`, `step.retry_due` and `step.retry_requested` of such a
//! node to `WAITING` ([`super::WaitingOn::Gate`]), and only the grant of the last gate it declares makes it `READY`. The
//! planner then asks for the decision ([`super::Evaluation::Gates`]) and the adapter journals a grant or a wait of each
//! gate in the same commit. A state rebuilt from the journal alone does not know the gated nodes: it sees the step
//! `READY` for the length of one commit and reaches the same state through the decisions that commit journals.

use std::fmt;

use super::ExecutionError;
use crate::workflow::{NodeId, RatePeriod, ResolvedGroup};

/// The pool of a key: ordinary runs, or draft test trees (US06), which never share a row (D1185).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GatePool {
    Ordinary,
    DraftTest,
}

impl GatePool {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ordinary => "ORDINARY",
            Self::DraftTest => "DRAFT_TEST",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Ordinary, Self::DraftTest]
            .into_iter()
            .find(|pool| pool.as_str() == value)
    }
}

/// The 12 lower-case hexadecimal characters that stand for a key in the journal and in listings: the prefix of its
/// keyed digest (D1168), never its value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyLabel(String);

/// How many characters a key label has.
pub const KEY_LABEL_LEN: usize = 12;

impl KeyLabel {
    /// # Errors
    /// [`ExecutionError::Invariant`] for anything but 12 lower-case hexadecimal characters.
    pub fn parse(value: &str) -> Result<Self, ExecutionError> {
        if value.len() == KEY_LABEL_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(ExecutionError::Invariant(
                "a key label is 12 lower-case hexadecimal characters",
            ))
        }
    }

    /// The label of a 32-byte keyed digest: the hex of its first six bytes.
    #[must_use]
    pub fn of_digest(digest: &[u8; 32]) -> Self {
        use std::fmt::Write as _;
        let mut label = String::with_capacity(KEY_LABEL_LEN);
        for byte in &digest[..KEY_LABEL_LEN / 2] {
            let _ = write!(label, "{byte:02x}");
        }
        Self(label)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The key a decision is about: pool, resolved group and label. The scope is the [`GateTarget`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateRef {
    pub pool: GatePool,
    pub group: ResolvedGroup,
    pub key_label: KeyLabel,
}

/// Who waits or holds: the run (its start) or one attempt of one root step.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GateTarget {
    Run,
    Step { node: NodeId, attempt: u32 },
}

impl GateTarget {
    /// `RUN` or `STEP`, as the journal's `scope` stores it.
    #[must_use]
    pub const fn scope(&self) -> &'static str {
        match self {
            Self::Run => "RUN",
            Self::Step { .. } => "STEP",
        }
    }
}

/// What happened to the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateChange {
    /// `concurrency.queued`: no permit was free; the waiter gives up at `queue_deadline_us`.
    ConcurrencyQueued { queue_deadline_us: i64 },
    /// `concurrency.acquired`: the permit is held.
    ConcurrencyAcquired,
    /// `concurrency.released`: the permit is given back (the run ended, the attempt ended, or a granted step left
    /// `READY` other than by its lease).
    ConcurrencyReleased,
    /// `concurrency.wait_timed_out`: the waiter's deadline passed while this gate refused it.
    ConcurrencyWaitTimedOut,
    /// `concurrency.dequeued`: the waiter left the queue without a permit (cancellation, the run's deadline, or the
    /// timeout of its other gate).
    ConcurrencyDequeued,
    /// `rate_limit.queued`: the window of the key had no room; the waiter is buffered.
    RateQueued {
        limit: u32,
        per: RatePeriod,
        queue_deadline_us: i64,
    },
    /// `rate_limit.granted`: a slot was granted at `granted_at_us` on the database clock.
    RateGranted {
        limit: u32,
        per: RatePeriod,
        granted_at_us: i64,
        from_queue: bool,
    },
    /// `rate_limit.wait_timed_out`.
    RateWaitTimedOut,
    /// `rate_limit.dequeued`.
    RateDequeued,
    /// `rate_limit.returned`: the slot of a granted step that left `READY` before its lease is returned.
    RateReturned,
}

impl GateChange {
    /// The stored event type.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ConcurrencyQueued { .. } => "concurrency.queued",
            Self::ConcurrencyAcquired => "concurrency.acquired",
            Self::ConcurrencyReleased => "concurrency.released",
            Self::ConcurrencyWaitTimedOut => "concurrency.wait_timed_out",
            Self::ConcurrencyDequeued => "concurrency.dequeued",
            Self::RateQueued { .. } => "rate_limit.queued",
            Self::RateGranted { .. } => "rate_limit.granted",
            Self::RateWaitTimedOut => "rate_limit.wait_timed_out",
            Self::RateDequeued => "rate_limit.dequeued",
            Self::RateReturned => "rate_limit.returned",
        }
    }

    /// Which gate it is about.
    #[must_use]
    pub const fn kind(&self) -> crate::workflow::GateKind {
        match self {
            Self::ConcurrencyQueued { .. }
            | Self::ConcurrencyAcquired
            | Self::ConcurrencyReleased
            | Self::ConcurrencyWaitTimedOut
            | Self::ConcurrencyDequeued => crate::workflow::GateKind::Concurrency,
            Self::RateQueued { .. }
            | Self::RateGranted { .. }
            | Self::RateWaitTimedOut
            | Self::RateDequeued
            | Self::RateReturned => crate::workflow::GateKind::RateLimit,
        }
    }
}

/// One flow-control journal event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateEvent {
    pub target: GateTarget,
    pub gate: GateRef,
    pub change: GateChange,
}

impl GateEvent {
    /// The node of a step-scope event.
    #[must_use]
    pub const fn node(&self) -> Option<&NodeId> {
        match &self.target {
            GateTarget::Run => None,
            GateTarget::Step { node, .. } => Some(node),
        }
    }

    /// The attempt a step-scope event is for.
    #[must_use]
    pub const fn attempt(&self) -> Option<u32> {
        match &self.target {
            GateTarget::Run => None,
            GateTarget::Step { attempt, .. } => Some(*attempt),
        }
    }
}

/// Where one gate of a run or of one attempt stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePhase {
    /// Waiting for the permit or the slot, until the instant (microseconds since the epoch).
    Queued {
        deadline_us: i64,
    },
    /// The permit is held, or the slot was granted (a slot is then counted by time, not by this state).
    Held,
    /// Given back: a permit released, or an unleased slot returned.
    Ended,
    TimedOut,
    Dequeued,
}

/// One gate of a run, or of one attempt of a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSlot {
    gate: GateRef,
    phase: GatePhase,
    /// The attempt a step's gate is for; 0 for a run.
    attempt: u32,
    /// A concurrency permit must be given back; a rate slot stops counting by time.
    gate_is_permit: bool,
}

impl GateSlot {
    #[must_use]
    pub const fn gate(&self) -> &GateRef {
        &self.gate
    }

    #[must_use]
    pub const fn phase(&self) -> GatePhase {
        self.phase
    }

    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    const fn is_queued(&self) -> bool {
        matches!(self.phase, GatePhase::Queued { .. })
    }

    /// Rebuilds a slot from trusted storage (the step projection written from a state the reducer produced).
    #[must_use]
    pub const fn restore(
        kind: crate::workflow::GateKind,
        gate: GateRef,
        phase: GatePhase,
        attempt: u32,
    ) -> Self {
        Self {
            gate,
            phase,
            attempt,
            gate_is_permit: matches!(kind, crate::workflow::GateKind::Concurrency),
        }
    }
}

/// Why the gates of a run's start are decided (feature 022, T022-064), and so what may be decided: carried from the domain
/// to the adapter unchanged (reviews of 2026-11-20, architecture M2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartCause {
    /// The run's creation (T10): in the transaction that holds the namespace's admission lock; it may queue.
    Admission,
    /// The release of a start delay (FR-022-018: the run "acquires or enqueues when the delay elapses"): it may queue, and
    /// takes the namespace's admission lock first (D022-051, D022-052).
    Release,
    /// A reset's admission, right after `run.reset` (018's plan, row 17): a fresh permit or a refusal, never a wait.
    Reset,
}

impl StartCause {
    /// Whether a wait in the key's queue is a decision this cause may take.
    #[must_use]
    pub const fn may_queue(self) -> bool {
        !matches!(self, Self::Reset)
    }
}

/// Checks the decisions of a run's start against what its definition declares (feature 022): exactly one per declared
/// gate, at run scope, a grant or (only when `cause` may queue) a wait. The one rule of every start decision, whatever
/// asked for it (architecture review M2 of 2026-11-20).
///
/// # Errors
/// [`ExecutionError::Invariant`] when the decisions are not those.
pub fn check_start_decisions(
    declared: DeclaredGates,
    decisions: &[GateEvent],
    cause: StartCause,
) -> Result<(), ExecutionError> {
    let decided: Vec<_> = decisions.iter().map(|d| d.change.kind()).collect();
    if decided.len() != declared.kinds().count()
        || declared
            .kinds()
            .any(|kind| decided.iter().filter(|k| **k == kind).count() != 1)
    {
        return Err(ExecutionError::Invariant(
            "the start's gates are decided once each, exactly the ones it declares",
        ));
    }
    for decision in decisions {
        let waits = matches!(
            decision.change,
            GateChange::ConcurrencyQueued { .. } | GateChange::RateQueued { .. }
        );
        let grants = matches!(
            decision.change,
            GateChange::ConcurrencyAcquired | GateChange::RateGranted { .. }
        );
        if !matches!(decision.target, GateTarget::Run) || !(waits || grants) {
            return Err(ExecutionError::Invariant(
                "the start's gates are granted or queued at run scope",
            ));
        }
        if waits && !cause.may_queue() {
            return Err(ExecutionError::Invariant(
                "a reset takes a fresh permit or slot, or is refused: it never queues",
            ));
        }
    }
    Ok(())
}

/// One root step whose gates the readiness gate decides, for the attempt it would start next (feature 022, T28).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRequest {
    pub node: NodeId,
    pub attempt: u32,
}

/// The gates a root step declares (feature 022): what its readiness waits for. A step of a node that declares one is
/// never `READY` without a grant of each (FR-022-051, invariant 11 of `data-model.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeclaredGates {
    pub concurrency: bool,
    pub rate: bool,
}

impl DeclaredGates {
    /// The gates `flow` declares, or `None` for a level that declares none.
    #[must_use]
    pub const fn of(flow: &crate::workflow::FlowControl) -> Option<Self> {
        if flow.is_gated() {
            Some(Self {
                concurrency: flow.concurrency().is_some(),
                rate: flow.rate_limit().is_some(),
            })
        } else {
            None
        }
    }

    /// The kinds declared, rate first (the precedence of FR-022-052).
    pub fn kinds(self) -> impl Iterator<Item = crate::workflow::GateKind> {
        [
            (self.rate, crate::workflow::GateKind::RateLimit),
            (self.concurrency, crate::workflow::GateKind::Concurrency),
        ]
        .into_iter()
        .filter_map(|(declared, kind)| declared.then_some(kind))
    }
}

/// The gates of a run (its start) or of a step: at most one of each kind.
///
/// Boxed (decision D022-102): every step's state and every run's state holds one, and a debug build copies those values
/// through many frames, so the gates are one pointer, and an ungated run or step allocates nothing.
#[derive(Clone, Default)]
pub struct Gates {
    slots: Option<Box<GateSlots>>,
}

/// The slots of [`Gates`], one of each kind.
#[derive(Clone, Default)]
struct GateSlots {
    concurrency: Option<GateSlot>,
    rate: Option<GateSlot>,
}

/// Prints the two slots, as before they were boxed.
impl std::fmt::Debug for Gates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gates")
            .field("concurrency", &self.concurrency())
            .field("rate", &self.rate())
            .finish()
    }
}

/// Equal when their slots are, whether or not a box was allocated.
impl PartialEq for Gates {
    fn eq(&self, other: &Self) -> bool {
        self.concurrency() == other.concurrency() && self.rate() == other.rate()
    }
}

impl Eq for Gates {}

impl Gates {
    /// No gate decided.
    pub const NONE: Self = Self { slots: None };

    #[must_use]
    pub fn concurrency(&self) -> Option<&GateSlot> {
        self.slots
            .as_ref()
            .and_then(|slots| slots.concurrency.as_ref())
    }

    #[must_use]
    pub fn rate(&self) -> Option<&GateSlot> {
        self.slots.as_ref().and_then(|slots| slots.rate.as_ref())
    }

    /// Whether any gate still waits.
    #[must_use]
    pub fn is_waiting(&self) -> bool {
        self.slots().any(GateSlot::is_queued)
    }

    /// Whether the run's entry is held: a gate waits, or the wait ended without a grant (the run must stop).
    #[must_use]
    pub fn holds_entry(&self) -> bool {
        self.slots().any(|slot| {
            matches!(
                slot.phase,
                GatePhase::Queued { .. } | GatePhase::TimedOut | GatePhase::Dequeued
            )
        })
    }

    /// Whether a gate still waits or still holds something to give back: a permit, or a slot of an attempt that may
    /// still be returned. A run whose steps hold one does not finish.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.slots().any(|slot| {
            matches!(slot.phase, GatePhase::Queued { .. })
                || (slot.phase == GatePhase::Held && slot.gate_is_permit)
        })
    }

    /// Whether attempt `attempt` may be leased: no gate of it waits, was given back or was given up on.
    #[must_use]
    pub fn admits_lease(&self, attempt: u32) -> bool {
        self.slots()
            .filter(|slot| slot.attempt == attempt)
            .all(|slot| slot.phase == GatePhase::Held)
    }

    /// The permit this level holds, if it holds one.
    #[must_use]
    pub fn held_permit(&self) -> Option<&GateSlot> {
        self.concurrency()
            .filter(|slot| slot.phase == GatePhase::Held)
    }

    /// The gates still waiting, rate first (the precedence of FR-022-052).
    pub fn waiting(&self) -> impl Iterator<Item = (crate::workflow::GateKind, &GateSlot)> {
        [
            (crate::workflow::GateKind::RateLimit, self.rate()),
            (crate::workflow::GateKind::Concurrency, self.concurrency()),
        ]
        .into_iter()
        .filter_map(|(kind, slot)| {
            slot.filter(|slot| slot.is_queued())
                .map(|slot| (kind, slot))
        })
    }

    fn slots(&self) -> impl Iterator<Item = &GateSlot> {
        self.concurrency().into_iter().chain(self.rate())
    }

    /// The decisions a reset restored with its prefix are history (FR-022-038; 018's plan, row 17): a permit or a slot
    /// the source held at the point belongs to an attempt that ended there (no step is open at a point), so the reset run
    /// has nothing to give back for it and holds no row of it.
    pub(super) fn retire_restored(&mut self) {
        let Some(slots) = self.slots.as_mut() else {
            return;
        };
        for slot in [&mut slots.concurrency, &mut slots.rate]
            .into_iter()
            .flatten()
        {
            if slot.phase == GatePhase::Held {
                slot.phase = GatePhase::Ended;
            }
        }
    }

    /// Rebuilds the gates of a step from trusted storage.
    #[must_use]
    pub fn restore(concurrency: Option<GateSlot>, rate: Option<GateSlot>) -> Self {
        if concurrency.is_none() && rate.is_none() {
            return Self::NONE;
        }
        Self {
            slots: Some(Box::new(GateSlots { concurrency, rate })),
        }
    }

    /// The slot of `kind`, whatever its attempt.
    #[must_use]
    pub fn of_kind(&self, kind: crate::workflow::GateKind) -> Option<&GateSlot> {
        match kind {
            crate::workflow::GateKind::Concurrency => self.concurrency(),
            crate::workflow::GateKind::RateLimit => self.rate(),
        }
    }

    /// Whether attempt `attempt` holds a grant of every gate in `declared` (FR-022-051).
    #[must_use]
    pub fn holds_all(&self, declared: DeclaredGates, attempt: u32) -> bool {
        declared.kinds().all(|kind| {
            self.of_kind(kind)
                .is_some_and(|slot| slot.attempt == attempt && slot.phase == GatePhase::Held)
        })
    }

    /// Whether a gate of attempt `attempt` was given back before that attempt was leased (a pause, a hold for review,
    /// or any other exit from `READY` without a lease, FR-022-046): that attempt may be asked again.
    #[must_use]
    pub fn returned_unleased(&self, attempt: u32) -> bool {
        self.slots()
            .any(|slot| slot.attempt == attempt && slot.phase == GatePhase::Ended)
    }

    /// Whether every gate in `declared` is decided for attempt `attempt` (granted, or waited for): what a commit that
    /// leaves a gated step waiting must hold (invariant 11).
    #[must_use]
    pub fn decided_all(&self, declared: DeclaredGates, attempt: u32) -> bool {
        declared.kinds().all(|kind| {
            self.of_kind(kind).is_some_and(|slot| {
                slot.attempt == attempt
                    && matches!(slot.phase, GatePhase::Held | GatePhase::Queued { .. })
            })
        })
    }

    fn slot_mut(&mut self, kind: crate::workflow::GateKind) -> &mut Option<GateSlot> {
        let slots = self.slots.get_or_insert_with(Box::default);
        match kind {
            crate::workflow::GateKind::Concurrency => &mut slots.concurrency,
            crate::workflow::GateKind::RateLimit => &mut slots.rate,
        }
    }

    /// Applies `event` to the gates of the run (`attempt` 0) or of a step's attempt.
    ///
    /// `fresh` says whether a gate with no state of this attempt may be decided at once (a run's admission, a step
    /// that became ready in this transition); a gate already decided for this attempt is decided again only after the
    /// attempt gave it back unleased (`Ended`, a pause or a hold for review, FR-022-046).
    pub(super) fn apply(
        &mut self,
        event: &GateEvent,
        attempt: u32,
        fresh: bool,
    ) -> Result<(), ExecutionError> {
        let kind = event.change.kind();
        let deciding = matches!(
            event.change,
            GateChange::ConcurrencyQueued { .. }
                | GateChange::ConcurrencyAcquired
                | GateChange::RateQueued { .. }
                | GateChange::RateGranted { .. }
        );
        // a wait that one gate gave up on is over: the other gate cannot then grant or queue it (both or neither,
        // FR-022-044)
        let other = match kind {
            crate::workflow::GateKind::Concurrency => self.rate(),
            crate::workflow::GateKind::RateLimit => self.concurrency(),
        };
        if deciding
            && other.is_some_and(|other| {
                other.attempt == attempt
                    && matches!(other.phase, GatePhase::TimedOut | GatePhase::Dequeued)
            })
        {
            return Err(ExecutionError::Invariant(
                "a wait one gate gave up on is not granted by the other",
            ));
        }
        let slot = self.slot_mut(kind);
        // a new attempt does not take the place of a permit the last one still holds
        if deciding
            && slot.as_ref().is_some_and(|slot| {
                slot.attempt != attempt && slot.phase == GatePhase::Held && slot.gate_is_permit
            })
        {
            return Err(ExecutionError::Invariant(
                "a permit the last attempt holds is given back before the next attempt decides",
            ));
        }
        // a slot of an earlier attempt is history: this attempt starts with none
        let current = slot.as_ref().filter(|slot| slot.attempt == attempt);
        let same_key = |slot: &GateSlot| slot.gate == event.gate;
        let next = match (
            event.change,
            current.map(|slot| (slot.phase, same_key(slot))),
        ) {
            (
                GateChange::ConcurrencyQueued { queue_deadline_us }
                | GateChange::RateQueued {
                    queue_deadline_us, ..
                },
                None,
            ) if fresh => GatePhase::Queued {
                deadline_us: queue_deadline_us,
            },
            (GateChange::ConcurrencyAcquired | GateChange::RateGranted { .. }, None) if fresh => {
                GatePhase::Held
            }
            // a slot this attempt gave back unleased (a pause, FR-022-046) is asked again, for the same attempt: an
            // attempt that was leased ends its slot and the next decision is for the next attempt
            (
                GateChange::ConcurrencyQueued { queue_deadline_us }
                | GateChange::RateQueued {
                    queue_deadline_us, ..
                },
                Some((GatePhase::Ended, _)),
            ) if fresh => GatePhase::Queued {
                deadline_us: queue_deadline_us,
            },
            (
                GateChange::ConcurrencyAcquired | GateChange::RateGranted { .. },
                Some((GatePhase::Ended, _)),
            ) if fresh => GatePhase::Held,
            (
                GateChange::ConcurrencyAcquired | GateChange::RateGranted { .. },
                Some((GatePhase::Queued { .. }, true)),
            ) => GatePhase::Held,
            (
                GateChange::ConcurrencyWaitTimedOut | GateChange::RateWaitTimedOut,
                Some((GatePhase::Queued { .. }, true)),
            ) => GatePhase::TimedOut,
            (
                GateChange::ConcurrencyDequeued | GateChange::RateDequeued,
                Some((GatePhase::Queued { .. }, true)),
            ) => GatePhase::Dequeued,
            (
                GateChange::ConcurrencyReleased | GateChange::RateReturned,
                Some((GatePhase::Held, true)),
            ) => GatePhase::Ended,
            _ => {
                return Err(ExecutionError::Invariant(
                    "a flow-control event does not follow the state of its gate",
                ));
            }
        };
        *slot = Some(GateSlot {
            gate: event.gate.clone(),
            phase: next,
            attempt,
            gate_is_permit: kind == crate::workflow::GateKind::Concurrency,
        });
        Ok(())
    }
}

/// The pure window decision of a rate-limit key (task T022-071; FR-022-040, FR-022-048; decisions D022-003, D022-005,
/// D022-009, D022-014, D022-021, D022-073, D022-078): which grants count for an acquirer, whether an arrival is granted,
/// queued or refused, whether the grant routine admits a queued waiter, what one pass of it grants, and a waiter's
/// position and the estimate of its grant, each a function of the instants passed in (microseconds since the epoch on
/// the database clock). It reads no clock and no row. The adapter decides in SQL with the same rules
/// (`crates/infra-postgres/src/execution/{readiness_gate.rs,gate_queue.rs}`); the differential test of decision D022-082
/// holds the two to one answer, and this module is its oracle.
///
/// Every limit passed in is the effective one: the lower of the acquirer's declaration and the namespace's current
/// ceiling of its window's length (decisions D022-071, D022-076 item 7); the caller applies the ceiling.
pub mod window {
    use crate::workflow::RatePeriod;

    /// The length of a window in microseconds.
    #[must_use]
    pub const fn period_us(per: RatePeriod) -> i64 {
        // at most an hour: 3.6e9 microseconds fit an `i64`
        (per.millis() * 1_000) as i64
    }

    /// One grant of the key as a decision counts it (decision D022-005): a run's grant is leased at its grant instant; a
    /// step's is unleased until the lease pass copies its lease instant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Grant {
        pub leased_at_us: Option<i64>,
    }

    impl Grant {
        /// A grant leased at `at_us` (a run's start, or a step once its lease is recorded).
        #[must_use]
        pub const fn leased(at_us: i64) -> Self {
            Self {
                leased_at_us: Some(at_us),
            }
        }

        /// A step's grant whose lease is not recorded yet.
        pub const UNLEASED: Self = Self { leased_at_us: None };

        /// Whether the grant counts for a window of `per` at `at_us`: unleased, or leased after `at_us - per` (the
        /// half-open window `(at - per, at]` of decision D022-032; a lease instant ahead of the clock counts, so a clock
        /// set back grants nothing until it catches up, NFR-022-008).
        #[must_use]
        pub const fn counts(self, per: RatePeriod, at_us: i64) -> bool {
            match self.leased_at_us {
                None => true,
                Some(leased) => leased > at_us.saturating_sub(period_us(per)),
            }
        }
    }

    /// What an acquirer applies to the key's shared grants: its own effective limit over its own window (D022-009).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Bound {
        pub limit: u32,
        pub per: RatePeriod,
    }

    impl Bound {
        /// Whether a refusal of this bound for lack of room, within one pass, holds back a younger waiter of `other`: one
        /// whose limit is not higher and whose window is not shorter (decision D022-078 item 2; looser is a higher limit or
        /// a shorter window, which skip-ahead allows, D1196).
        #[must_use]
        pub fn holds_back(self, other: Self) -> bool {
            other.limit <= self.limit && other.per >= self.per
        }
    }

    /// A queued waiter of the key, in queue order (its enqueuing instant, then its run and node).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Waiter {
        pub bound: Bound,
        /// Its queue deadline: from that instant on it is the timeout routine's, never granted.
        pub deadline_us: i64,
        /// Its run is running or waiting and not paused: the pass serves it and counts it ahead (ADR-0089 decision 4).
        pub live: bool,
    }

    /// What an arrival meets at the key.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Admission {
        Granted,
        Queued,
        /// Only for an arrival that may not queue (a reset): no room for it (decision D022-076 item 3).
        Refused,
    }

    /// Where a waiter stands (decision D022-014): its position (1 is next) and the estimate of its grant.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Place {
        pub position: u32,
        pub estimate_us: Option<i64>,
    }

    /// The key's grants and its waiters in queue order, as one decision reads them under the key's lock.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct Window {
        pub grants: Vec<Grant>,
        pub waiters: Vec<Waiter>,
    }

    impl Window {
        /// The grants that count for a window of `per` at `at_us`.
        #[must_use]
        pub fn counted(&self, per: RatePeriod, at_us: i64) -> u32 {
            let n = self
                .grants
                .iter()
                .filter(|grant| grant.counts(per, at_us))
                .count();
            u32::try_from(n).unwrap_or(u32::MAX)
        }

        /// Whether `waiter` is eligible at `at_us`, its own window counted as of `counted_at_us` (no later than `at_us`;
        /// decision D022-073 item 2): its run is live, its deadline is after `at_us`, and its own limit leaves room for it
        /// over its own window (an older waiter refused by its own lower limit does not block a younger one, D1196).
        #[must_use]
        pub fn is_eligible(&self, waiter: &Waiter, at_us: i64, counted_at_us: i64) -> bool {
            waiter.live
                && waiter.deadline_us > at_us
                && waiter.bound.limit > self.counted(waiter.bound.per, at_us.min(counted_at_us))
        }

        /// The eligible waiters among `waiters`.
        fn eligible(&self, waiters: &[Waiter], at_us: i64, counted_at_us: i64) -> u32 {
            let n = waiters
                .iter()
                .filter(|waiter| self.is_eligible(waiter, at_us, counted_at_us))
                .count();
            u32::try_from(n).unwrap_or(u32::MAX)
        }

        /// The decision about a new arrival of `bound` at `at_us`. An arrival that may queue is granted only when its
        /// window has room and no eligible waiter is ahead (strict admission order, decision D022-073), else queued; one
        /// that may not queue (a reset) is granted by the reservation rule (counted grants plus the eligible waiters below
        /// its limit, decision D022-076 item 3; ADR-0089 decision 4), else refused.
        #[must_use]
        pub fn admit(&self, bound: Bound, at_us: i64, may_queue: bool) -> Admission {
            let counted = self.counted(bound.per, at_us);
            let waits = if may_queue {
                Admission::Queued
            } else {
                Admission::Refused
            };
            if counted >= bound.limit {
                return waits;
            }
            let admitted = if may_queue {
                self.eligible(&self.waiters, at_us, at_us) == 0
            } else {
                let reserved = self.eligible(&self.waiters, at_us, at_us).min(bound.limit);
                counted.saturating_add(reserved) < bound.limit
            };
            if admitted { Admission::Granted } else { waits }
        }

        /// Whether the grant routine admits the waiter at `index` at `at_us`, the key's grants counted as of the pass's
        /// selection `counted_at_us`: it is live and before its deadline, and the key's counted grants plus the eligible
        /// waiters ahead of it are below its limit (the reservation rule, ADR-0089 decision 4).
        #[must_use]
        pub fn admits_queued(&self, index: usize, at_us: i64, counted_at_us: i64) -> bool {
            let Some(waiter) = self.waiters.get(index) else {
                return false;
            };
            if !waiter.live || waiter.deadline_us <= at_us {
                return false;
            }
            let counted = self.counted(waiter.bound.per, at_us.min(counted_at_us));
            let ahead = self.eligible(&self.waiters[..index], at_us, counted_at_us);
            counted.saturating_add(ahead) < waiter.bound.limit
        }

        /// One pass of the grant routine over the key from `at_us`, its grants counted as of `counted_at_us` (decision
        /// D022-073 item 2): the live waiters before their deadline at `at_us` are tried oldest first, at most `budget`,
        /// the `i`-th in its own transaction at `at_us + i` microseconds (each reads the clock after its locks, so the
        /// instants of one pass increase); each one the counts admit is granted (its grant leased at its instant when
        /// `leased_at_grant`, a run's start, else unleased, a step) unless a refusal for lack of room earlier in the pass
        /// holds it back (decision D022-078 item 2). Returns, for each waiter granted, its position in the queue as it was
        /// and its grant's instant; the waiters granted leave the queue.
        pub fn pass(
            &mut self,
            at_us: i64,
            counted_at_us: i64,
            leased_at_grant: bool,
            budget: usize,
        ) -> Vec<(usize, i64)> {
            let listed: Vec<usize> = (0..self.waiters.len())
                .filter(|i| self.waiters[*i].live && self.waiters[*i].deadline_us > at_us)
                .take(budget)
                .collect();
            let mut refused: Vec<Bound> = Vec::new();
            let mut granted: Vec<(usize, i64)> = Vec::new();
            for (tried, original) in (0_i64..).zip(listed) {
                let now = at_us.saturating_add(tried);
                // its index once the waiters granted before it in this pass have left the queue
                let index = original - granted.iter().filter(|(g, _)| *g < original).count();
                let bound = self.waiters[index].bound;
                if !self.admits_queued(index, now, counted_at_us) {
                    if !refused.iter().any(|known| known.holds_back(bound)) {
                        refused.push(bound);
                    }
                    continue;
                }
                if refused.iter().any(|known| known.holds_back(bound)) {
                    continue;
                }
                self.waiters.remove(index);
                self.grants.push(if leased_at_grant {
                    Grant::leased(now)
                } else {
                    Grant::UNLEASED
                });
                granted.push((original, now));
            }
            granted
        }

        /// Where the waiter at `index` stands at `at_us` (decision D022-014; plan "Visibility of the rate gate"): ahead of
        /// it count the waiters the grant routine will count ahead when it has room (live, before their deadline, and
        /// either of no tighter limit and window than it, so eligible whenever it has room, or below the count of their
        /// own window by their own limit, decisions D022-076 item 5 and D022-078 item 3). The estimate is
        /// [`estimate_us`]; none while a counted grant is unleased (the slot ahead is held by a step a worker has not
        /// leased). `None` for no such waiter.
        #[must_use]
        pub fn place(&self, index: usize, at_us: i64) -> Option<Place> {
            let me = self.waiters.get(index)?;
            let ahead = self.waiters[..index]
                .iter()
                .filter(|x| {
                    x.live
                        && x.deadline_us > at_us
                        && ((x.bound.limit >= me.bound.limit && x.bound.per <= me.bound.per)
                            || x.bound.limit > self.counted(x.bound.per, at_us))
                })
                .count();
            let ahead = u64::try_from(ahead).unwrap_or(u64::MAX);
            let mut leases: Vec<i64> = Vec::new();
            let mut unleased = false;
            for grant in &self.grants {
                if grant.counts(me.bound.per, at_us) {
                    match grant.leased_at_us {
                        Some(lease) => leases.push(lease),
                        None => unleased = true,
                    }
                }
            }
            leases.sort_unstable();
            let counted = u64::try_from(leases.len()).unwrap_or(u64::MAX);
            let k = (counted + ahead).saturating_sub(u64::from(me.bound.limit));
            let kth = usize::try_from(k).ok().and_then(|k| leases.get(k).copied());
            let estimate = if unleased {
                None
            } else {
                estimate_us(Reading {
                    at_us,
                    counted,
                    ahead,
                    limit: me.bound.limit,
                    kth_lease_us: kth,
                    per: me.bound.per,
                    deadline_us: me.deadline_us,
                    skipped: !me.live,
                })
            };
            Some(Place {
                position: u32::try_from(ahead.saturating_add(1)).unwrap_or(u32::MAX),
                estimate_us: estimate,
            })
        }
    }

    /// What one statement read of a waiting acquirer's place at `at_us` (plan "Visibility of the rate gate"): the key's
    /// grants counted over its window, the waiters counted ahead of it, its limit, the lease instant of the `k`-th oldest
    /// counted grant for `k = counted + ahead - limit + 1` (when `1 <= k <= counted`), its window and deadline, and whether
    /// the pass skips its run.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Reading {
        pub at_us: i64,
        pub counted: u64,
        pub ahead: u64,
        pub limit: u32,
        pub kth_lease_us: Option<i64>,
        pub per: RatePeriod,
        pub deadline_us: i64,
        pub skipped: bool,
    }

    /// The estimate of a waiter's grant (decision D022-014): the grant routine grants it when `counted + ahead < limit`; a
    /// grant to a waiter ahead moves one from `ahead` to `counted`, so the sum falls only when a counted grant ages out, at
    /// its lease plus its window (the half-open window). `at_us` when there is room now, the `k`-th lease plus the window
    /// when `1 <= k <= counted`, none beyond one window (`k > counted`). None too when the pass will not grant it (its run
    /// skipped, or the instant at or after its deadline: at the deadline exactly the waiter times out first).
    #[must_use]
    pub fn estimate_us(reading: Reading) -> Option<i64> {
        if reading.skipped {
            return None;
        }
        let sum = reading.counted.saturating_add(reading.ahead);
        let at = if sum < u64::from(reading.limit) {
            reading.at_us
        } else {
            let k = sum - u64::from(reading.limit) + 1;
            if k > reading.counted {
                return None;
            }
            reading.kth_lease_us?.saturating_add(period_us(reading.per))
        };
        (at < reading.deadline_us).then_some(at)
    }
}
