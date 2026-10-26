//! Pure capacity-group lifecycle (A1). Possession grants no execution permission.
//!
//! A transactional adapter must supply the selected member's current exact registry fact, hold
//! its own execution authority, prove quiescence/settlement and persist header/member changes
//! together. This model neither implements that port nor enables nested execution.

use thiserror::Error;

use crate::identity::TenantId;

uuid_id!(
    /// A retained tenant capacity reservation, independent of a particular run's finality.
    CapacityGroupId
);
uuid_id!(
    /// A non-reusable logical participant in one capacity group.
    CapacityMemberId
);

/// Possession is separate from logical membership and tenant execution eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityPossession {
    Vacant,
    Held(CapacityMemberId),
    Retired,
}

/// Exact registry state read for the selected identity, not inferred from a missing cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityMembership {
    NeverSeen,
    Active,
    Finished,
}

/// One bounded registry fact. The adapter must bind it to the current group revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityMemberSnapshot {
    group: CapacityGroupId,
    member: CapacityMemberId,
    state: CapacityMembership,
}

impl CapacityMemberSnapshot {
    #[must_use]
    pub const fn new(
        group: CapacityGroupId,
        member: CapacityMemberId,
        state: CapacityMembership,
    ) -> Self {
        Self {
            group,
            member,
            state,
        }
    }

    #[must_use]
    pub const fn group_id(self) -> CapacityGroupId {
        self.group
    }

    #[must_use]
    pub const fn member_id(self) -> CapacityMemberId {
        self.member
    }

    #[must_use]
    pub const fn state(self) -> CapacityMembership {
        self.state
    }
}

/// A rejected transition leaves the entire header unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CapacityGroupError {
    #[error("the capacity group has retired")]
    Retired,
    #[error("capacity group revision changed to {current}")]
    StaleRevision { current: u64 },
    #[error("capacity possession epoch changed to {current}")]
    StaleEpoch { current: u64 },
    #[error("the member fact belongs to another group")]
    ForeignGroup,
    #[error("a known member identity cannot be rejoined")]
    MemberAlreadyKnown,
    #[error("the member is not active")]
    InactiveMember,
    #[error("the member does not hold possession")]
    NotHolder,
    #[error("the group is already held")]
    Occupied,
    #[error("the capacity snapshot is inconsistent")]
    InconsistentSnapshot,
    #[error("a capacity counter cannot advance")]
    CounterExhausted,
}

/// Constant-size header: historical membership stays in the adapter's exact registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityGroup {
    id: CapacityGroupId,
    tenant: TenantId,
    revision: u64,
    epoch: u64,
    active_members: u32,
    possession: CapacityPossession,
}

impl CapacityGroup {
    /// Creates a header for fresh, authorized identities. The adapter must atomically register
    /// the initial active member and enforce group/member uniqueness before using this header.
    #[must_use]
    pub const fn new(id: CapacityGroupId, tenant: TenantId, initial: CapacityMemberId) -> Self {
        Self {
            id,
            tenant,
            revision: 1,
            epoch: 1,
            active_members: 1,
            possession: CapacityPossession::Held(initial),
        }
    }

    /// Restores a bounded persisted header without resetting its fencing counters.
    ///
    /// The adapter must supply the current exact holder fact under its group guard. These checks
    /// validate supplied consistency, not database counts, tenant ownership, historical uniqueness
    /// or permission to execute, vacate or finish. No registry is hydrated here.
    ///
    /// # Errors
    /// Returns [`CapacityGroupError::InconsistentSnapshot`] for invalid counters/counts,
    /// a missing or contradictory held-member fact, or a fact without a holder.
    pub fn restore(
        id: CapacityGroupId,
        tenant: TenantId,
        revision: u64,
        epoch: u64,
        active_members: u32,
        possession: CapacityPossession,
        holder_snapshot: Option<CapacityMemberSnapshot>,
    ) -> Result<Self, CapacityGroupError> {
        if revision == 0 || epoch == 0 || epoch > revision || u64::from(active_members) > revision {
            return Err(CapacityGroupError::InconsistentSnapshot);
        }
        let consistent = match (possession, holder_snapshot) {
            (CapacityPossession::Retired, None) => active_members == 0,
            (CapacityPossession::Vacant, None) => active_members > 0,
            (CapacityPossession::Held(holder), Some(member)) => {
                active_members > 0
                    && member.group == id
                    && member.member == holder
                    && member.state == CapacityMembership::Active
            }
            _ => false,
        };
        if !consistent {
            return Err(CapacityGroupError::InconsistentSnapshot);
        }
        Ok(Self {
            id,
            tenant,
            revision,
            epoch,
            active_members,
            possession,
        })
    }

    #[must_use]
    pub const fn id(&self) -> CapacityGroupId {
        self.id
    }
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }
    #[must_use]
    pub const fn active_members(&self) -> u32 {
        self.active_members
    }
    #[must_use]
    pub const fn possession(&self) -> CapacityPossession {
        self.possession
    }

    /// Joins a never-used identity without changing the holder's epoch.
    ///
    /// # Errors
    /// Rejects stale, foreign, known or contradictory facts and exhausted counters.
    pub fn join(
        &mut self,
        expected_revision: u64,
        member: CapacityMemberSnapshot,
    ) -> Result<CapacityMemberSnapshot, CapacityGroupError> {
        let revision = self.next_revision(expected_revision)?;
        self.check_group(member)?;
        if member.state != CapacityMembership::NeverSeen {
            return Err(CapacityGroupError::MemberAlreadyKnown);
        }
        if self.possession == CapacityPossession::Held(member.member) {
            return Err(CapacityGroupError::InconsistentSnapshot);
        }
        let active = self
            .active_members
            .checked_add(1)
            .ok_or(CapacityGroupError::CounterExhausted)?;
        self.active_members = active;
        self.revision = revision;
        Ok(CapacityMemberSnapshot::new(
            self.id,
            member.member,
            CapacityMembership::Active,
        ))
    }

    /// Vacates possession after the caller proves its own path has no unsettled effect in flight.
    /// Ready but gated work and durable waits may remain; logical membership is retained.
    ///
    /// # Errors
    /// Rejects stale, foreign, inactive or non-holder facts and exhausted counters.
    pub fn vacate(
        &mut self,
        expected_revision: u64,
        expected_epoch: u64,
        member: CapacityMemberSnapshot,
    ) -> Result<(), CapacityGroupError> {
        let revision = self.next_revision(expected_revision)?;
        self.check_active(member)?;
        let epoch = self.next_epoch(expected_epoch)?;
        if self.possession != CapacityPossession::Held(member.member) {
            return Err(CapacityGroupError::NotHolder);
        }
        self.possession = CapacityPossession::Vacant;
        self.epoch = epoch;
        self.revision = revision;
        Ok(())
    }

    /// Acquires a vacancy. The caller separately establishes current execution eligibility.
    ///
    /// # Errors
    /// Rejects stale, foreign or inactive facts, occupied groups and exhausted counters.
    pub fn acquire(
        &mut self,
        expected_revision: u64,
        expected_epoch: u64,
        member: CapacityMemberSnapshot,
    ) -> Result<(), CapacityGroupError> {
        let revision = self.next_revision(expected_revision)?;
        self.check_active(member)?;
        let epoch = self.next_epoch(expected_epoch)?;
        if self.possession != CapacityPossession::Vacant {
            return Err(CapacityGroupError::Occupied);
        }
        self.possession = CapacityPossession::Held(member.member);
        self.epoch = epoch;
        self.revision = revision;
        Ok(())
    }

    /// Settles only this logical member, after its own obligations are proved settled. A holder
    /// leaves a vacancy without requiring a ready recipient; the last member retires the group.
    /// CONTINUED is not settlement of a logical child-chain member.
    ///
    /// # Errors
    /// Rejects stale, foreign, inactive or inconsistent facts and exhausted counters.
    pub fn finish(
        &mut self,
        expected_revision: u64,
        member: CapacityMemberSnapshot,
    ) -> Result<CapacityMemberSnapshot, CapacityGroupError> {
        let revision = self.next_revision(expected_revision)?;
        self.check_active(member)?;
        let active = self
            .active_members
            .checked_sub(1)
            .ok_or(CapacityGroupError::InconsistentSnapshot)?;
        let holder = self.possession == CapacityPossession::Held(member.member);
        if active == 0 && matches!(self.possession, CapacityPossession::Held(_)) && !holder {
            return Err(CapacityGroupError::InconsistentSnapshot);
        }
        let epoch = if holder || active == 0 {
            self.next_epoch(self.epoch)?
        } else {
            self.epoch
        };
        let possession = if active == 0 {
            CapacityPossession::Retired
        } else if holder {
            CapacityPossession::Vacant
        } else {
            self.possession
        };
        self.active_members = active;
        self.possession = possession;
        self.epoch = epoch;
        self.revision = revision;
        Ok(CapacityMemberSnapshot::new(
            self.id,
            member.member,
            CapacityMembership::Finished,
        ))
    }

    fn next_revision(&self, expected: u64) -> Result<u64, CapacityGroupError> {
        if self.possession == CapacityPossession::Retired {
            return Err(CapacityGroupError::Retired);
        }
        if self.revision != expected {
            return Err(CapacityGroupError::StaleRevision {
                current: self.revision,
            });
        }
        self.revision
            .checked_add(1)
            .ok_or(CapacityGroupError::CounterExhausted)
    }

    fn next_epoch(&self, expected: u64) -> Result<u64, CapacityGroupError> {
        if self.epoch != expected {
            return Err(CapacityGroupError::StaleEpoch {
                current: self.epoch,
            });
        }
        self.epoch
            .checked_add(1)
            .ok_or(CapacityGroupError::CounterExhausted)
    }

    fn check_group(&self, member: CapacityMemberSnapshot) -> Result<(), CapacityGroupError> {
        if member.group != self.id {
            return Err(CapacityGroupError::ForeignGroup);
        }
        Ok(())
    }

    fn check_active(&self, member: CapacityMemberSnapshot) -> Result<(), CapacityGroupError> {
        self.check_group(member)?;
        if member.state != CapacityMembership::Active {
            return Err(CapacityGroupError::InactiveMember);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> (CapacityGroup, CapacityMemberSnapshot) {
        let id = CapacityGroupId::new(uuid::Uuid::from_u128(1));
        let who = CapacityMemberId::new(uuid::Uuid::from_u128(2));
        (
            CapacityGroup::new(id, TenantId::new(uuid::Uuid::from_u128(3)), who),
            CapacityMemberSnapshot::new(id, who, CapacityMembership::Active),
        )
    }

    #[test]
    fn exhausted_revision_or_epoch_does_not_partially_vacate() {
        let (mut g, member) = group();
        g.revision = u64::MAX;
        let before = g.clone();
        assert_eq!(
            g.vacate(g.revision, g.epoch, member),
            Err(CapacityGroupError::CounterExhausted)
        );
        assert_eq!(g, before);
        let (mut g, member) = group();
        g.epoch = u64::MAX;
        let before = g.clone();
        assert_eq!(
            g.finish(g.revision, member),
            Err(CapacityGroupError::CounterExhausted)
        );
        assert_eq!(g, before);
    }

    #[test]
    fn exhausted_membership_count_does_not_advance_revision() {
        let (mut g, _) = group();
        g.active_members = u32::MAX;
        let before = g.clone();
        let new = CapacityMemberSnapshot::new(
            g.id,
            CapacityMemberId::new(uuid::Uuid::from_u128(4)),
            CapacityMembership::NeverSeen,
        );
        assert_eq!(
            g.join(g.revision, new),
            Err(CapacityGroupError::CounterExhausted)
        );
        assert_eq!(g, before);
    }
}
