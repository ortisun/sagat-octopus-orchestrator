//! The port of shard ownership (`FR-010-001`): which replica of the engine looks at which shard of
//! the scheduler's work, by a lease with a generation.
//!
//! The leases are rows in the database and every time comparison is the database's clock, so
//! replicas with skewed clocks agree on who owns what. A lease is **renewed** by its owner (same
//! generation), **taken** when it ran out or was given back (the generation goes up by one), and
//! **fenced** by whoever acts on it: the scheduler checks, inside the transaction in which it
//! changes a run, that the generation it holds is still the shard's. The fence is what makes a
//! replica that was paused for longer than its lease harmless when it wakes up.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use sagat_domain::identity::TenantId;
use sagat_domain::sharding::{Held, Shard};

use crate::error::StoreError;

/// A lease on a shard as its owner holds it: the shard and the generation it was taken at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Lease {
    pub shard: Shard,
    pub generation: u64,
}

/// Shard ownership and the weights of the tenants. Implemented by `sagat-infra-postgres`.
pub trait ShardStore: Send + Sync {
    /// Says that `owner` is alive for `ttl` and returns how many replicas are alive, itself
    /// included. A replica that holds no shard yet is still counted.
    fn announce(
        &self,
        owner: &str,
        ttl: Duration,
    ) -> impl Future<Output = Result<usize, StoreError>> + Send;

    /// Takes `owner` out of the replicas that are alive (a clean shutdown), without waiting for it
    /// to expire.
    fn withdraw(&self, owner: &str) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Every shard's lease, as of the database's clock.
    fn leases(&self) -> impl Future<Output = Result<BTreeMap<Shard, Held>, StoreError>> + Send;

    /// Tries to take each of `shards` for `owner` for `ttl`: the ones that are free or whose lease
    /// ran out. Returns the leases it got (a shard another replica took first is not among them).
    fn acquire(
        &self,
        owner: &str,
        shards: &[Shard],
        ttl: Duration,
    ) -> impl Future<Output = Result<Vec<Lease>, StoreError>> + Send;

    /// Extends the leases `owner` holds, for `ttl` from now. Returns the ones that were still its
    /// own: a lease that ran out and was taken by another replica is not renewed, whatever the
    /// owner believes.
    fn renew(
        &self,
        owner: &str,
        held: &[Lease],
        ttl: Duration,
    ) -> impl Future<Output = Result<Vec<Lease>, StoreError>> + Send;

    /// Gives the leases back so that another replica can take them at once.
    fn release(
        &self,
        owner: &str,
        held: &[Lease],
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// The scheduling weight of every tenant that has one (the others have 1).
    fn tenant_weights(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<TenantId, u16>, StoreError>> + Send;
}
