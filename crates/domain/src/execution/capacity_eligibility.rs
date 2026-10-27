//! Pure tenant capacity eligibility (B1c). Counters only: no group registry, no storage.
//!
//! The adapter reads the tenant's counters and the selected group's enablement under the tenant
//! guard, supplies the *current* effective limit, and persists the returned counters atomically
//! with the group's enablement. This model neither implements that port nor enables nested
//! execution, and it grants no authority: possession, run/worker authority and quiescence are
//! proven elsewhere (A1, B1a, B1b).
//!
//! Rules (refining ADR-0030 §§13–14; `capacity-eligibility.md` D701–D703 records the decisions):
//! - A fresh group needs `retained < limit`. A parked group re-enables for *forward* work only with
//!   `enabled < limit`. Limit zero therefore admits and forward-enables nothing; the drain lane
//!   below still opens.
//! - A reduction never revokes an enabled group. While `enabled > limit` the model refuses *new
//!   forward* claims; that those holders then park at their next safe point is an obligation of the
//!   adapter (G701/G706), not something this model performs.
//! - Reserved drain (work consuming an already reserved compensation/reconciliation identity)
//!   never depends on the limit. An enabled group runs it. A parked group with no ordinary room
//!   takes the tenant's single drain lane. The lane adds at most one executing group beyond the
//!   enabled ones; it does not serialise drain among enabled groups.
//! - Counters are anonymous: a transition names an enablement, and is refused when the counters
//!   hold no group in it.

use thiserror::Error;

/// How one retained group relates to the tenant's execution capacity right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityEnablement {
    /// Retained (holds its unit) but not executing; durable work stays unclaimable.
    Parked,
    /// Enabled within the ordinary bound; may take forward and reserved-drain work.
    Forward,
    /// Holds the tenant's single drain lane; may take reserved-drain work only.
    Drain,
}

/// Class of the work a group wants to claim. The adapter derives it from the instance's own
/// reservation state; a caller assertion is not enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityWork {
    Forward,
    ReservedDrain,
}

/// A refused operation leaves the counters unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CapacityEligibilityError {
    #[error("the tenant has no room under its current capacity limit")]
    NoRoom,
    #[error("the tenant's single drain lane is occupied")]
    DrainLaneBusy,
    #[error("enabled groups exceed the current limit; park before taking more forward work")]
    OverBound,
    #[error("a drain-only group cannot take forward work")]
    DrainOnly,
    #[error("a parked group cannot take work")]
    NotEnabled,
    #[error("the transition does not apply to the group's current enablement")]
    WrongEnablement,
    #[error("the capacity counters are inconsistent")]
    InconsistentCounters,
}

/// Constant-size tenant projection of its capacity groups.
///
/// `retained` counts every group still holding a unit (legacy flat `active_iterations`),
/// `enabled` those in [`CapacityEnablement::Forward`], `draining` the one in
/// [`CapacityEnablement::Drain`]. Parked groups are `retained - enabled - draining`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TenantCapacity {
    retained: u64,
    enabled: u64,
    draining: u64,
}

impl TenantCapacity {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            retained: 0,
            enabled: 0,
            draining: 0,
        }
    }

    /// Restores a persisted projection. These checks validate supplied consistency, not the
    /// actual group rows; the adapter must reconcile them under the tenant guard.
    ///
    /// # Errors
    /// [`CapacityEligibilityError::InconsistentCounters`] when there is more than one drain
    /// lane or `enabled + draining` exceeds `retained`.
    pub const fn restore(
        retained: u64,
        enabled: u64,
        draining: u64,
    ) -> Result<Self, CapacityEligibilityError> {
        match enabled.checked_add(draining) {
            Some(active) if draining <= 1 && active <= retained => Ok(Self {
                retained,
                enabled,
                draining,
            }),
            _ => Err(CapacityEligibilityError::InconsistentCounters),
        }
    }

    #[must_use]
    pub const fn retained(&self) -> u64 {
        self.retained
    }
    #[must_use]
    pub const fn enabled(&self) -> u64 {
        self.enabled
    }
    #[must_use]
    pub const fn draining(&self) -> u64 {
        self.draining
    }

    /// Admits a fresh group, born enabled for forward work.
    ///
    /// # Errors
    /// [`CapacityEligibilityError::NoRoom`] unless `retained < limit` (always at limit zero).
    pub const fn allocate_group(
        &mut self,
        limit: u64,
    ) -> Result<CapacityEnablement, CapacityEligibilityError> {
        if self.retained >= limit {
            return Err(CapacityEligibilityError::NoRoom);
        }
        // retained < limit <= u64::MAX, and enabled <= retained, so neither increment can wrap.
        self.retained += 1;
        self.enabled += 1;
        Ok(CapacityEnablement::Forward)
    }

    /// Re-enables a parked group for ordinary forward execution.
    ///
    /// # Errors
    /// [`CapacityEligibilityError::WrongEnablement`] unless `from` is parked,
    /// [`CapacityEligibilityError::InconsistentCounters`] when no parked group can exist,
    /// [`CapacityEligibilityError::NoRoom`] unless `enabled < limit`.
    pub const fn enable(
        &mut self,
        from: CapacityEnablement,
        limit: u64,
    ) -> Result<CapacityEnablement, CapacityEligibilityError> {
        if let Err(error) = self.require_parked(from) {
            return Err(error);
        }
        if self.enabled >= limit {
            return Err(CapacityEligibilityError::NoRoom);
        }
        // A parked group exists, so enabled + 1 <= retained: no wrap.
        self.enabled += 1;
        Ok(CapacityEnablement::Forward)
    }

    /// Enables a parked group that has accepted reserved-drain obligations: ordinary room first,
    /// otherwise the single drain lane, which does not depend on the limit.
    ///
    /// # Errors
    /// As [`Self::enable`], with [`CapacityEligibilityError::DrainLaneBusy`] instead of `NoRoom`
    /// when the lane is occupied.
    pub const fn enable_for_drain(
        &mut self,
        from: CapacityEnablement,
        limit: u64,
    ) -> Result<CapacityEnablement, CapacityEligibilityError> {
        if let Err(error) = self.require_parked(from) {
            return Err(error);
        }
        if self.enabled < limit {
            self.enabled += 1;
            return Ok(CapacityEnablement::Forward);
        }
        if self.draining == 0 {
            self.draining = 1;
            return Ok(CapacityEnablement::Drain);
        }
        Err(CapacityEligibilityError::DrainLaneBusy)
    }

    /// Parks a group at a safe point. Never limited: leaving can only free capacity. The adapter
    /// must have proven the group's own quiescence.
    ///
    /// # Errors
    /// [`CapacityEligibilityError::WrongEnablement`] for a parked group,
    /// [`CapacityEligibilityError::InconsistentCounters`] when the counters hold no such group.
    pub const fn park(
        &mut self,
        from: CapacityEnablement,
    ) -> Result<CapacityEnablement, CapacityEligibilityError> {
        match from {
            CapacityEnablement::Parked => Err(CapacityEligibilityError::WrongEnablement),
            CapacityEnablement::Forward if self.enabled > 0 => {
                self.enabled -= 1;
                Ok(CapacityEnablement::Parked)
            }
            CapacityEnablement::Drain if self.draining > 0 => {
                self.draining -= 1;
                Ok(CapacityEnablement::Parked)
            }
            _ => Err(CapacityEligibilityError::InconsistentCounters),
        }
    }

    /// Releases a settled group's unit from whatever enablement it had.
    ///
    /// # Errors
    /// [`CapacityEligibilityError::InconsistentCounters`] when the counters hold no such group.
    pub const fn retire(
        &mut self,
        from: CapacityEnablement,
    ) -> Result<(), CapacityEligibilityError> {
        let consistent = match from {
            CapacityEnablement::Parked => self.enabled + self.draining < self.retained,
            CapacityEnablement::Forward => self.enabled > 0,
            CapacityEnablement::Drain => self.draining > 0,
        };
        if !consistent || self.retained == 0 {
            return Err(CapacityEligibilityError::InconsistentCounters);
        }
        match from {
            CapacityEnablement::Parked => {}
            CapacityEnablement::Forward => self.enabled -= 1,
            CapacityEnablement::Drain => self.draining -= 1,
        }
        self.retained -= 1;
        Ok(())
    }

    /// Whether `group` may claim `work` now. Read-only: claiming does not move a counter.
    ///
    /// # Errors
    /// [`CapacityEligibilityError::InconsistentCounters`] when the counters hold no group in the
    /// stated enablement,
    /// [`CapacityEligibilityError::NotEnabled`] for a parked group,
    /// [`CapacityEligibilityError::DrainOnly`] for forward work on the drain lane,
    /// [`CapacityEligibilityError::OverBound`] for forward work while `enabled > limit`.
    pub const fn claim(
        &self,
        limit: u64,
        group: CapacityEnablement,
        work: CapacityWork,
    ) -> Result<(), CapacityEligibilityError> {
        let present = match group {
            CapacityEnablement::Parked => self.enabled + self.draining < self.retained,
            CapacityEnablement::Forward => self.enabled > 0,
            CapacityEnablement::Drain => self.draining > 0,
        };
        if !present {
            return Err(CapacityEligibilityError::InconsistentCounters);
        }
        match (group, work) {
            (CapacityEnablement::Parked, _) => Err(CapacityEligibilityError::NotEnabled),
            (CapacityEnablement::Drain, CapacityWork::Forward) => {
                Err(CapacityEligibilityError::DrainOnly)
            }
            (CapacityEnablement::Forward, CapacityWork::Forward) if self.enabled > limit => {
                Err(CapacityEligibilityError::OverBound)
            }
            _ => Ok(()),
        }
    }

    /// A parked group must be `from`, and the counters must leave room for one.
    const fn require_parked(
        &self,
        from: CapacityEnablement,
    ) -> Result<(), CapacityEligibilityError> {
        if !matches!(from, CapacityEnablement::Parked) {
            return Err(CapacityEligibilityError::WrongEnablement);
        }
        // draining <= 1 and enabled + draining <= retained are restore invariants; the sum
        // cannot wrap for any projection built by `restore` or the transitions above.
        if self.enabled + self.draining >= self.retained {
            return Err(CapacityEligibilityError::InconsistentCounters);
        }
        Ok(())
    }
}
