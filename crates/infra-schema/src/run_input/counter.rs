//! The work counter (D043-047, D043-058, D043-067): every loop step and every subschema application of the
//! validator is charged before it runs, and a charge that would pass the budget stops the whole validation. It is the
//! bound on the work, because it counts what the validator does; the clock it reads every [`BACKSTOP_CLOCK_EVERY`]
//! units is a backstop for a starved machine, never the bound.

use std::time::{Duration, Instant};

use super::{BACKSTOP_CLOCK_EVERY, RUN_INPUT_BACKSTOP, RUN_INPUT_WORK_UNITS};

/// Why an evaluation stopped before its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    /// The instance does not satisfy the subschema.
    Fail,
    /// The next charge would pass the budget: final for the input, never one branch's failure.
    WorkLimit,
    /// The work went on past the backstop: transient, never remembered.
    Backstop,
}

/// The limits of one validation: the budget of units and the backstop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Units a validation may spend ([`RUN_INPUT_WORK_UNITS`] in production).
    pub units: u64,
    /// How long the work may go on before the backstop stops it ([`RUN_INPUT_BACKSTOP`] in production).
    pub backstop: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            units: RUN_INPUT_WORK_UNITS,
            backstop: RUN_INPUT_BACKSTOP,
        }
    }
}

/// What one validation spent: the units charged and, with the `test-support` feature, the validator's primitive
/// operations (lookups, comparisons, the bytes a pattern scans), so a test can hold the operations to a constant
/// multiple of the units (G043-017). Without the feature `operations` is zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub units: u64,
    pub operations: u64,
}

pub(super) struct Counter {
    used: u64,
    budget: u64,
    next_clock: u64,
    deadline: Instant,
    #[cfg(feature = "test-support")]
    operations: u64,
}

impl Counter {
    pub(super) fn new(limits: Limits) -> Self {
        Self {
            used: 0,
            budget: limits.units,
            next_clock: BACKSTOP_CLOCK_EVERY,
            deadline: Instant::now() + limits.backstop,
            #[cfg(feature = "test-support")]
            operations: 0,
        }
    }

    /// Charges `units` before the step they pay for. Past the budget nothing is charged and the validation stops.
    pub(super) fn charge(&mut self, units: u64) -> Result<(), Stop> {
        let next = self.used.saturating_add(units);
        if next > self.budget {
            return Err(Stop::WorkLimit);
        }
        self.used = next;
        if self.used >= self.next_clock {
            self.next_clock = self.used.saturating_add(BACKSTOP_CLOCK_EVERY);
            if Instant::now() >= self.deadline {
                return Err(Stop::Backstop);
            }
        }
        Ok(())
    }

    /// Counts `n` primitive operations of the validator (with the `test-support` feature; nothing otherwise).
    #[inline]
    #[allow(clippy::unused_self, unused_variables)]
    pub(super) fn op(&mut self, n: u64) {
        #[cfg(feature = "test-support")]
        {
            self.operations = self.operations.saturating_add(n);
        }
    }

    pub(super) fn counters(&self) -> Counters {
        Counters {
            units: self.used,
            #[cfg(feature = "test-support")]
            operations: self.operations,
            #[cfg(not(feature = "test-support"))]
            operations: 0,
        }
    }
}
