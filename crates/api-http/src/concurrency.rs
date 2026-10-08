//! A cap on the requests of one tenant that are being served at the same time (gap G13).
//!
//! The global in-flight cap protects the process; this one protects the other tenants from a single
//! tenant. A client that holds requests open (a slow or stalled upload, a slow consumer) can only
//! occupy its own tenant's share of the slots, so everybody else keeps being served.
//!
//! The cap is taken after authentication and released when the permit is dropped, which the handler
//! does by returning (the permit travels inside the authenticated caller).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use sagat_domain::identity::TenantId;

/// Counts in-flight requests per tenant. Only tenants with at least one request in flight have an
/// entry, so the table is bounded by the global in-flight cap, not by the number of tenants.
#[derive(Debug)]
pub struct TenantConcurrency {
    cap: usize,
    active: Mutex<HashMap<TenantId, usize>>,
}

/// Returned when a tenant already has its maximum number of requests in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the tenant already has {cap} requests in flight")]
pub struct TenantAtCapacity {
    pub cap: usize,
}

/// One in-flight request of a tenant. Dropping it frees the slot.
#[derive(Debug)]
#[must_use = "dropping the permit immediately releases the tenant's slot"]
pub struct TenantPermit {
    owner: Arc<TenantConcurrency>,
    tenant: TenantId,
}

impl TenantConcurrency {
    /// A cap of at least one request per tenant (a cap of zero would refuse everything).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            active: Mutex::new(HashMap::new()),
        }
    }

    /// The effective cap.
    #[must_use]
    pub const fn cap(&self) -> usize {
        self.cap
    }

    /// Takes a slot for `tenant`.
    ///
    /// # Errors
    /// [`TenantAtCapacity`] when the tenant already has [`Self::cap`] requests in flight.
    pub fn try_acquire(
        self: &Arc<Self>,
        tenant: TenantId,
    ) -> Result<TenantPermit, TenantAtCapacity> {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        let in_flight = active.entry(tenant).or_insert(0);
        if *in_flight >= self.cap {
            return Err(TenantAtCapacity { cap: self.cap });
        }
        *in_flight += 1;
        Ok(TenantPermit {
            owner: Arc::clone(self),
            tenant,
        })
    }

    /// Tenants with at least one request in flight (for tests and diagnostics).
    #[must_use]
    pub fn tenants_in_flight(&self) -> usize {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Drop for TenantPermit {
    fn drop(&mut self) {
        let mut active = self
            .owner
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(in_flight) = active.get_mut(&self.tenant) {
            *in_flight = in_flight.saturating_sub(1);
            if *in_flight == 0 {
                active.remove(&self.tenant);
            }
        }
    }
}
