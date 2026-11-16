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
//! Scope of this build (0.3.0 slice, foundation): the reducer rules and their replay. The readiness mapping that
//! makes a gated step wait instead of becoming `READY` (FR-022-051) is T022-096; until it lands a step-scope `queued`
//! is applied to a step the planner made `READY` in the same transition.

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
}

/// The gates of a run (its start) or of a step: at most one of each kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gates {
    concurrency: Option<GateSlot>,
    rate: Option<GateSlot>,
}

impl Gates {
    /// No gate decided.
    pub const NONE: Self = Self {
        concurrency: None,
        rate: None,
    };

    #[must_use]
    pub const fn concurrency(&self) -> Option<&GateSlot> {
        self.concurrency.as_ref()
    }

    #[must_use]
    pub const fn rate(&self) -> Option<&GateSlot> {
        self.rate.as_ref()
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

    /// The permit this level holds, if it holds one.
    #[must_use]
    pub fn held_permit(&self) -> Option<&GateSlot> {
        self.concurrency
            .as_ref()
            .filter(|slot| slot.phase == GatePhase::Held)
    }

    /// The gates still waiting, rate first (the precedence of FR-022-052).
    pub fn waiting(&self) -> impl Iterator<Item = (crate::workflow::GateKind, &GateSlot)> {
        [
            (crate::workflow::GateKind::RateLimit, self.rate.as_ref()),
            (
                crate::workflow::GateKind::Concurrency,
                self.concurrency.as_ref(),
            ),
        ]
        .into_iter()
        .filter_map(|(kind, slot)| {
            slot.filter(|slot| slot.is_queued())
                .map(|slot| (kind, slot))
        })
    }

    fn slots(&self) -> impl Iterator<Item = &GateSlot> {
        self.concurrency.iter().chain(self.rate.iter())
    }

    fn slot_mut(&mut self, kind: crate::workflow::GateKind) -> &mut Option<GateSlot> {
        match kind {
            crate::workflow::GateKind::Concurrency => &mut self.concurrency,
            crate::workflow::GateKind::RateLimit => &mut self.rate,
        }
    }

    /// Applies `event` to the gates of the run (`attempt` 0) or of a step's attempt.
    ///
    /// `fresh` says whether a gate with no state of this attempt may be decided at once (a run's admission, a step
    /// that became ready in this transition); a gate already decided for this attempt never is again.
    pub(super) fn apply(
        &mut self,
        event: &GateEvent,
        attempt: u32,
        fresh: bool,
    ) -> Result<(), ExecutionError> {
        let kind = event.change.kind();
        let slot = self.slot_mut(kind);
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
        });
        Ok(())
    }
}
