//! Limits and slots for server-sent streams (D142, gap G143).
//!
//! A stream outlives the request that opened it, so the per-tenant concurrency cap of
//! [`crate::concurrency`] (taken for the length of a handler) does not bound it. This registry does:
//! at most [`StreamLimits::per_subject`] streams per subject and [`StreamLimits::per_tenant`] per
//! tenant are open at once, a slot is released when the stream ends for any reason (the client left,
//! the run finished, the token expired), and a table entry exists only while a stream is open.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use sagat_domain::identity::TenantId;

/// How streams are opened, polled and closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits {
    /// Open streams in the whole process: what all the tenants together may ask of the database
    /// and the memory of one instance.
    pub global: usize,
    /// Open streams per tenant.
    pub per_tenant: usize,
    /// Open streams per subject of a tenant.
    pub per_subject: usize,
    /// How often the server looks for a change while the run is moving.
    pub poll_min: Duration,
    /// How often it looks once nothing has moved for [`Self::idle_after`].
    pub poll_max: Duration,
    pub idle_after: Duration,
    /// A comment line every so often keeps proxies from closing a quiet stream.
    pub ping: Duration,
    /// The longest a stream stays open; the client reconnects and misses nothing.
    pub max_lifetime: Duration,
    /// How often an open stream asks again whether its subject is still a member with the right to
    /// read: a role taken away ends the stream within this time, not at the end of its lifetime.
    pub recheck: Duration,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            global: 2000,
            per_tenant: 200,
            per_subject: 20,
            poll_min: Duration::from_millis(500),
            poll_max: Duration::from_secs(2),
            idle_after: Duration::from_secs(30),
            ping: Duration::from_secs(15),
            max_lifetime: Duration::from_secs(15 * 60),
            recheck: Duration::from_secs(30),
        }
    }
}

/// Counts the open streams.
#[derive(Debug)]
pub struct StreamRegistry {
    limits: StreamLimits,
    open: Mutex<Open>,
}

#[derive(Debug, Default)]
struct Open {
    tenants: HashMap<TenantId, usize>,
    subjects: HashMap<(TenantId, String), usize>,
}

/// Why a stream could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StreamRefused {
    #[error("the platform already has its maximum number of streams open")]
    GlobalAtCapacity,
    #[error("the subject already has its maximum number of streams open")]
    SubjectAtCapacity,
    #[error("the tenant already has its maximum number of streams open")]
    TenantAtCapacity,
}

/// One open stream. Dropping it frees the slot.
#[derive(Debug)]
#[must_use = "dropping the slot immediately releases it"]
pub struct StreamSlot {
    owner: Arc<StreamRegistry>,
    tenant: TenantId,
    subject: String,
}

impl StreamRegistry {
    #[must_use]
    pub fn new(limits: StreamLimits) -> Arc<Self> {
        Arc::new(Self {
            limits: StreamLimits {
                global: limits.global.max(1),
                per_tenant: limits.per_tenant.max(1),
                per_subject: limits.per_subject.max(1),
                ..limits
            },
            open: Mutex::new(Open::default()),
        })
    }

    #[must_use]
    pub const fn limits(&self) -> &StreamLimits {
        &self.limits
    }

    /// Takes a slot for `subject` of `tenant`.
    ///
    /// # Errors
    /// [`StreamRefused`] when the platform, the subject or the tenant is at its cap.
    pub fn try_open(
        self: &Arc<Self>,
        tenant: TenantId,
        subject: &str,
    ) -> Result<StreamSlot, StreamRefused> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (tenant, subject.to_owned());
        if open.tenants.values().sum::<usize>() >= self.limits.global {
            return Err(StreamRefused::GlobalAtCapacity);
        }
        if open.subjects.get(&key).copied().unwrap_or(0) >= self.limits.per_subject {
            return Err(StreamRefused::SubjectAtCapacity);
        }
        if open.tenants.get(&tenant).copied().unwrap_or(0) >= self.limits.per_tenant {
            return Err(StreamRefused::TenantAtCapacity);
        }
        *open.subjects.entry(key).or_insert(0) += 1;
        *open.tenants.entry(tenant).or_insert(0) += 1;
        Ok(StreamSlot {
            owner: Arc::clone(self),
            tenant,
            subject: subject.to_owned(),
        })
    }

    /// Streams open right now (for tests and diagnostics).
    #[must_use]
    pub fn open_streams(&self) -> usize {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tenants
            .values()
            .sum()
    }
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut open = self
            .owner
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let key = (self.tenant, std::mem::take(&mut self.subject));
        if let Some(n) = open.subjects.get_mut(&key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.subjects.remove(&key);
            }
        }
        if let Some(n) = open.tenants.get_mut(&self.tenant) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                open.tenants.remove(&self.tenant);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(per_tenant: usize, per_subject: usize) -> StreamLimits {
        StreamLimits {
            per_tenant,
            per_subject,
            ..StreamLimits::default()
        }
    }

    #[test]
    fn a_subject_and_a_tenant_each_have_a_cap_and_a_dropped_slot_frees_it() {
        let registry = StreamRegistry::new(limits(3, 2));
        let tenant = TenantId::generate();
        let first = registry.try_open(tenant, "alice").expect("1");
        let _second = registry.try_open(tenant, "alice").expect("2");
        assert_eq!(
            registry.try_open(tenant, "alice").unwrap_err(),
            StreamRefused::SubjectAtCapacity
        );
        let _third = registry.try_open(tenant, "bob").expect("another subject");
        assert_eq!(
            registry.try_open(tenant, "carol").unwrap_err(),
            StreamRefused::TenantAtCapacity
        );
        assert_eq!(registry.open_streams(), 3);
        drop(first);
        assert!(registry.try_open(tenant, "alice").is_ok());
    }

    #[test]
    fn one_tenant_cannot_use_another_tenants_share_and_the_table_empties() {
        let registry = StreamRegistry::new(limits(1, 1));
        let (a, b) = (TenantId::generate(), TenantId::generate());
        let slot_a = registry.try_open(a, "alice").expect("a");
        let slot_b = registry
            .try_open(b, "alice")
            .expect("the same subject name in another tenant");
        assert_eq!(registry.open_streams(), 2);
        drop(slot_a);
        drop(slot_b);
        assert_eq!(registry.open_streams(), 0);
        let open = registry.open.lock().expect("lock");
        assert!(open.tenants.is_empty() && open.subjects.is_empty());
    }

    #[test]
    fn a_cap_of_zero_still_allows_one() {
        let registry = StreamRegistry::new(limits(0, 0));
        let tenant = TenantId::generate();
        assert!(registry.try_open(tenant, "alice").is_ok());
    }
}
