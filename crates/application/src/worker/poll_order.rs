//! Where a poll starts trying its candidates (design 038, FR-038-004, OD-2026-11-12-3).
//!
//! Every poll of a namespace reads the same oldest candidates. Tried in the same order, polls in
//! flight together all go for the same first runs, and all but one find them locked. Each poll here
//! takes a turn among the polls of its namespace in flight in this process, and starts past what the
//! polls before it asked for. The order is a preference, never a filter: a poll still tries every
//! candidate of its window, wrapping round. A poll alone has turn 0 and tries them oldest first. The
//! turn is the server's, never the client's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use sagat_domain::identity::{NamespaceId, TenantId};

/// The polls of each namespace in flight in this process.
#[derive(Debug, Default)]
pub(super) struct PollsInFlight(Mutex<HashMap<(TenantId, NamespaceId), usize>>);

/// A poll's place among the polls of its namespace in flight; it leaves when dropped.
pub(super) struct PollTurn {
    polls: Arc<PollsInFlight>,
    key: (TenantId, NamespaceId),
    /// How many polls of the namespace were already in flight when this one began.
    pub(super) ahead: usize,
}

impl PollsInFlight {
    pub(super) fn enter(self: &Arc<Self>, tenant: TenantId, namespace: NamespaceId) -> PollTurn {
        let key = (tenant, namespace);
        let mut polls = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let in_flight = polls.entry(key).or_insert(0);
        let ahead = *in_flight;
        *in_flight += 1;
        PollTurn {
            polls: Arc::clone(self),
            key,
            ahead,
        }
    }
}

impl Drop for PollTurn {
    fn drop(&mut self) {
        let mut polls = self.polls.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(in_flight) = polls.get_mut(&self.key) {
            *in_flight -= 1;
            if *in_flight == 0 {
                polls.remove(&self.key);
            }
        }
    }
}

/// Rotates `candidates` so that a poll with `ahead` polls before it, each asking for `wanted`, starts
/// past them. Nothing is removed; a poll alone (`ahead` 0) keeps the scan's order.
pub(super) fn start_past<T>(candidates: &mut [T], ahead: usize, wanted: usize) {
    if candidates.is_empty() {
        return;
    }
    let start = ahead.saturating_mul(wanted) % candidates.len();
    candidates.rotate_left(start);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poll_alone_keeps_the_order_of_the_scan() {
        let mut candidates = [1, 2, 3, 4];
        start_past(&mut candidates, 0, 2);
        assert_eq!(candidates, [1, 2, 3, 4]);
    }

    #[test]
    fn a_poll_starts_past_what_the_polls_ahead_of_it_asked_for_and_removes_nothing() {
        let mut candidates = [1, 2, 3, 4, 5];
        start_past(&mut candidates, 1, 2);
        assert_eq!(candidates, [3, 4, 5, 1, 2]);
        let mut candidates = [1, 2, 3, 4, 5];
        start_past(&mut candidates, 3, 2);
        assert_eq!(candidates, [2, 3, 4, 5, 1], "wraps round the window");
        let mut empty: [u8; 0] = [];
        start_past(&mut empty, 4, 2);
    }

    #[test]
    fn turns_count_the_polls_in_flight_per_namespace_and_free_on_drop() {
        let polls = Arc::new(PollsInFlight::default());
        let tenant = TenantId::new(uuid::Uuid::new_v4());
        let (one, two) = (
            NamespaceId::new(uuid::Uuid::new_v4()),
            NamespaceId::new(uuid::Uuid::new_v4()),
        );
        let first = polls.enter(tenant, one);
        let second = polls.enter(tenant, one);
        let other = polls.enter(tenant, two);
        assert_eq!((first.ahead, second.ahead, other.ahead), (0, 1, 0));
        drop(first);
        drop(second);
        assert_eq!(
            polls.enter(tenant, one).ahead,
            0,
            "a namespace with nothing in flight starts at 0"
        );
        drop(other);
        assert!(
            polls.0.lock().unwrap().is_empty(),
            "nothing is kept for a namespace once its polls end"
        );
    }
}
