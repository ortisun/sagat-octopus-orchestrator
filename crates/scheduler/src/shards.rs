//! Which replica looks at which shard (`FR-010-001`).
//!
//! The scheduler's work is split in [`SHARDS`](sagat_domain::sharding::SHARDS) shards by the run's
//! id. A replica works on the shards it holds a **lease** on, and the [`Keeper`] is the loop that
//! keeps those leases: it announces the replica, renews what it holds, gives back what is beyond
//! its fair share and takes what nobody holds. What it concludes is published in a [`Placement`]
//! that the passes read, and nothing else: the keeper never touches a run.
//!
//! Three things keep a replica that was paused, partitioned or slow from doing harm:
//!
//! - a lease has a **generation**, and every scheduler action checks, in its own transaction, that
//!   the generation it holds is still the shard's (the fence, `RunTx::holds_shard`): an action
//!   started before the lease was lost cannot commit after another replica took the shard;
//! - a snapshot **expires on the replica's own monotonic clock** shortly before the database would
//!   let somebody else take the lease, so a replica that cannot reach the database stops selecting
//!   work by itself;
//! - the passes are idempotent under the run's lock regardless: the fence is what bounds how much
//!   redundant work a split brain can do, not what makes it correct.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

use sagat_application::error::StoreError;
use sagat_application::shards::{Lease, ShardStore};
use sagat_domain::sharding::{self, Shard, ShardSet};

use crate::fairness::Weights;

/// How the keeper runs.
#[derive(Debug, Clone, Copy)]
pub struct KeeperSettings {
    /// How long a lease, and the announcement of the replica, last without being renewed. After a
    /// crash the shards of the replica are taken over this long afterwards, at the latest.
    pub ttl: Duration,
}

impl KeeperSettings {
    /// How often to run a round: a fifth of the lease. The loop waits this long (up to a fifth more,
    /// with the jitter) *after* a round, which itself takes time, so a third would leave room for one
    /// round to fail and not for two; a fifth leaves it for two, and the lease is renewed from the
    /// round that begins at most `ttl / 5 * 1.2` after the last good one.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.ttl / 5
    }
}

impl Default for KeeperSettings {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(15),
        }
    }
}

/// What a replica holds at a moment: the leases it believes it has, the weights of the tenants, and
/// until when to believe it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    leases: BTreeMap<Shard, u64>,
    weights: Weights,
    valid_until: Instant,
}

impl Snapshot {
    fn nothing(weights: Weights) -> Self {
        Self {
            leases: BTreeMap::new(),
            weights,
            valid_until: Instant::now(),
        }
    }

    /// The shards the replica may work on at `now`: none once the snapshot has run out.
    #[must_use]
    pub fn shards(&self, now: Instant) -> ShardSet {
        if now >= self.valid_until {
            return ShardSet::empty();
        }
        self.leases.keys().copied().collect()
    }

    /// The lease on `shard`, if the replica holds it at `now`.
    #[must_use]
    pub fn lease_of(&self, shard: Shard, now: Instant) -> Option<Lease> {
        if now >= self.valid_until {
            return None;
        }
        self.leases
            .get(&shard)
            .map(|&generation| Lease { shard, generation })
    }

    /// Every lease in the snapshot, whether or not it is still good.
    #[must_use]
    pub fn leases(&self) -> Vec<Lease> {
        self.leases
            .iter()
            .map(|(&shard, &generation)| Lease { shard, generation })
            .collect()
    }

    #[must_use]
    pub const fn weights(&self) -> &Weights {
        &self.weights
    }
}

/// The shards one replica works on, as the [`Keeper`] last concluded. Cheap to clone and to read.
#[derive(Clone)]
pub struct Placement {
    owner: Arc<str>,
    current: Arc<RwLock<Arc<Snapshot>>>,
}

impl std::fmt::Debug for Placement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Placement")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

impl Placement {
    /// A replica named `owner`, which holds no shard yet.
    #[must_use]
    pub fn new(owner: impl Into<Arc<str>>) -> Self {
        Self {
            owner: owner.into(),
            current: Arc::new(RwLock::new(Arc::new(Snapshot::nothing(Weights::default())))),
        }
    }

    /// The name the replica holds its leases under.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    #[must_use]
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn publish(&self, snapshot: Snapshot) {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(snapshot);
    }

    /// The shards the replica works on now.
    #[must_use]
    pub fn owned(&self) -> ShardSet {
        self.snapshot().shards(Instant::now())
    }
}

/// What one round of the keeper did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Round {
    /// Replicas that are alive, this one included.
    pub replicas: usize,
    /// Shards held after the round.
    pub held: usize,
    pub acquired: usize,
    pub released: usize,
    /// Leases the replica believed it had and no longer has (it was away for longer than the lease,
    /// or somebody gave the shard to another replica).
    pub lost: usize,
}

/// Keeps the leases of one replica.
pub struct Keeper<S> {
    store: S,
    placement: Placement,
    settings: KeeperSettings,
}

impl<S> std::fmt::Debug for Keeper<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keeper")
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}

impl<S: ShardStore> Keeper<S> {
    pub const fn new(store: S, placement: Placement, settings: KeeperSettings) -> Self {
        Self {
            store,
            placement,
            settings,
        }
    }

    #[must_use]
    pub const fn placement(&self) -> &Placement {
        &self.placement
    }

    /// Announces the replica, renews what it holds, gives back what is beyond its share, takes what
    /// is free, and publishes the result. On an error the previous snapshot stays until it expires
    /// on its own: a replica that cannot reach the database stops by itself.
    ///
    /// # Errors
    /// The store's: the round stopped where it failed and the next one starts over.
    pub async fn round(&self) -> Result<Round, StoreError> {
        use tracing::Instrument;
        let span = tracing::info_span!(
            "scheduler.keeper.round",
            owner = %self.placement.owner(),
            held = tracing::field::Empty,
            acquired = tracing::field::Empty,
            released = tracing::field::Empty,
        );
        let result = self.keep().instrument(span).await;
        if result.is_err() {
            // The gauges say what this replica can act on now, which is what its snapshot says (the
            // snapshot runs out by itself), and that it does not know how many replicas there are.
            #[allow(clippy::cast_precision_loss)] // a gauge
            metrics::gauge!("scheduler_shards_owned").set(self.placement.owned().len() as f64);
            metrics::gauge!("scheduler_replicas").set(0.0);
        }
        result
    }

    async fn keep(&self) -> Result<Round, StoreError> {
        // Every deadline is measured from before the first call, so the snapshot never outlives the
        // lease the database granted: it was granted after this instant.
        let started = Instant::now();
        let owner = self.placement.owner();
        let ttl = self.settings.ttl;
        let weights = match self.store.tenant_weights().await {
            Ok(weights) => Weights::new(weights),
            Err(error) => {
                tracing::warn!(%error, "could not read the tenant weights; keeping the previous ones");
                self.placement.snapshot().weights().clone()
            }
        };
        let replicas = self.store.announce(owner, ttl).await?;

        let before = self.placement.snapshot().leases();
        let renewed = if before.is_empty() {
            Vec::new()
        } else {
            self.store.renew(owner, &before, ttl).await?
        };
        let mut held: BTreeMap<Shard, u64> =
            renewed.iter().map(|l| (l.shard, l.generation)).collect();
        // What was renewed is the database's for another lease: say so at once, so that a later step
        // that fails does not leave the snapshot shorter than the leases.
        let valid_until = started + ttl;
        self.placement.publish(Snapshot {
            leases: held.clone(),
            weights: weights.clone(),
            valid_until,
        });

        let table = self.store.leases().await?;
        // Leases the database says are this replica's and the snapshot does not know: the answer to
        // an `acquire` was lost on its way back. The name is unique to this process, so they are its
        // own; they are renewed (so that they last as long as the snapshot says) and adopted.
        let unknown: Vec<Lease> = table
            .iter()
            .filter(|(shard, held_by)| held_by.is_held_by(owner) && !held.contains_key(shard))
            .map(|(shard, held_by)| Lease {
                shard: *shard,
                generation: held_by.generation,
            })
            .collect();
        if !unknown.is_empty() {
            for lease in self.store.renew(owner, &unknown, ttl).await? {
                held.insert(lease.shard, lease.generation);
            }
        }
        // Lost is what the database no longer lets this replica act on after the adoption: a shard
        // that was skipped for a moment (an action had its row locked) and adopted just now is not.
        let lost = before
            .iter()
            .filter(|lease| !held.contains_key(&lease.shard))
            .count();
        let plan = sharding::plan(owner, replicas, &table);

        // Stop using the shards before giving them back, never the other way round.
        let give_back: Vec<Lease> = plan
            .release
            .iter()
            .filter_map(|shard| {
                held.remove(shard).map(|generation| Lease {
                    shard: *shard,
                    generation,
                })
            })
            .collect();
        self.placement.publish(Snapshot {
            leases: held.clone(),
            weights: weights.clone(),
            valid_until,
        });
        if !give_back.is_empty() {
            self.store.release(owner, &give_back).await?;
        }
        let took = if plan.acquire.is_empty() {
            Vec::new()
        } else {
            self.store.acquire(owner, &plan.acquire, ttl).await?
        };
        for lease in &took {
            held.insert(lease.shard, lease.generation);
        }
        let round = Round {
            replicas,
            held: held.len(),
            acquired: took.len(),
            released: give_back.len(),
            lost,
        };
        self.placement.publish(Snapshot {
            leases: held,
            weights,
            valid_until,
        });

        let span = tracing::Span::current();
        span.record("held", round.held);
        span.record("acquired", round.acquired);
        span.record("released", round.released);
        metrics::gauge!("scheduler_shards_owned").set(round.held as f64);
        metrics::gauge!("scheduler_replicas").set(round.replicas as f64);
        metrics::counter!("scheduler_shards_acquired_total").increment(round.acquired as u64);
        metrics::counter!("scheduler_shards_released_total").increment(round.released as u64);
        metrics::counter!("scheduler_shards_lost_total").increment(round.lost as u64);
        if round.lost > 0 {
            tracing::warn!(
                lost = round.lost,
                runbook = "docs/runbooks/scaling.md#shard-fencing",
                "shard leases were lost: the replica was away for longer than the lease, or a shard was given to another"
            );
        }
        // A handover is an event worth a line; a round that changed nothing is not.
        if round.acquired > 0 || round.released > 0 {
            tracing::info!(
                held = round.held,
                acquired = round.acquired,
                released = round.released,
                replicas = round.replicas,
                "scheduler shards changed hands"
            );
        }
        Ok(round)
    }

    /// A clean shutdown: stops using the shards, gives them back so that the others take them at
    /// once instead of after the lease, and leaves the replicas that are alive.
    pub async fn leave(&self) {
        let owner = self.placement.owner();
        let held = self.placement.snapshot().leases();
        let weights = self.placement.snapshot().weights().clone();
        self.placement.publish(Snapshot::nothing(weights));
        if !held.is_empty()
            && let Err(error) = self.store.release(owner, &held).await
        {
            tracing::warn!(%error, "could not give the shards back; they will be taken when their lease ends");
        }
        if let Err(error) = self.store.withdraw(owner).await {
            tracing::warn!(%error, "could not leave the replicas that are alive; the others will notice when the announcement ends");
        }
        metrics::gauge!("scheduler_shards_owned").set(0.0);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use sagat_domain::identity::TenantId;
    use sagat_domain::sharding::{Held, SHARDS};

    use super::*;

    /// The contract of the store with a clock the test moves.
    #[derive(Default)]
    struct Fake {
        state: Mutex<State>,
        down: Mutex<bool>,
        /// The next `acquire` changes the leases and then fails: the answer is lost on its way back.
        lose_the_answer: Mutex<bool>,
        /// Shards whose row is locked by an action in flight: a renewal skips each of them once
        /// (`FOR UPDATE SKIP LOCKED`), and the lock is gone by the next call.
        locked_once: Mutex<std::collections::BTreeSet<Shard>>,
    }

    #[derive(Default)]
    struct State {
        now: Duration,
        slots: BTreeMap<Shard, (Option<String>, u64, Duration)>,
        members: BTreeMap<String, Duration>,
        weights: BTreeMap<TenantId, u16>,
    }

    #[derive(Clone)]
    struct Shared(Arc<Fake>);

    impl Shared {
        fn new() -> Self {
            Self(Arc::new(Fake::default()))
        }
        fn advance(&self, by: Duration) {
            self.0.state.lock().unwrap().now += by;
        }
        fn down(&self, down: bool) {
            *self.0.down.lock().unwrap() = down;
        }
        fn check(&self) -> Result<(), StoreError> {
            if *self.0.down.lock().unwrap() {
                Err(StoreError::Internal {
                    detail: "the database is down".into(),
                })
            } else {
                Ok(())
            }
        }
        fn owner_of(&self, shard: Shard) -> Option<(String, u64)> {
            let state = self.0.state.lock().unwrap();
            state
                .slots
                .get(&shard)
                .and_then(|(owner, generation, expires)| {
                    owner
                        .clone()
                        .filter(|_| *expires > state.now)
                        .map(|o| (o, *generation))
                })
        }
    }

    impl ShardStore for Shared {
        async fn announce(&self, owner: &str, ttl: Duration) -> Result<usize, StoreError> {
            self.check()?;
            let mut state = self.0.state.lock().unwrap();
            let expires = state.now + ttl;
            state.members.insert(owner.to_owned(), expires);
            let now = state.now;
            Ok(state.members.values().filter(|e| **e > now).count())
        }
        async fn withdraw(&self, owner: &str) -> Result<(), StoreError> {
            self.check()?;
            self.0.state.lock().unwrap().members.remove(owner);
            Ok(())
        }
        async fn leases(&self) -> Result<BTreeMap<Shard, Held>, StoreError> {
            self.check()?;
            let state = self.0.state.lock().unwrap();
            Ok(Shard::all()
                .map(|shard| {
                    let (owner, generation, expires) = state
                        .slots
                        .get(&shard)
                        .cloned()
                        .unwrap_or((None, 0, Duration::ZERO));
                    (
                        shard,
                        Held {
                            owner,
                            generation,
                            remaining: expires.saturating_sub(state.now),
                        },
                    )
                })
                .collect())
        }
        async fn acquire(
            &self,
            owner: &str,
            shards: &[Shard],
            ttl: Duration,
        ) -> Result<Vec<Lease>, StoreError> {
            self.check()?;
            let mut state = self.0.state.lock().unwrap();
            let now = state.now;
            let mut got = Vec::new();
            for shard in shards {
                let slot = state
                    .slots
                    .entry(*shard)
                    .or_insert((None, 0, Duration::ZERO));
                if slot.2 <= now {
                    *slot = (Some(owner.to_owned()), slot.1 + 1, now + ttl);
                    got.push(Lease {
                        shard: *shard,
                        generation: slot.1,
                    });
                }
            }
            drop(state);
            if std::mem::take(&mut *self.0.lose_the_answer.lock().unwrap()) {
                return Err(StoreError::Unavailable {
                    detail: "the answer was lost".into(),
                });
            }
            Ok(got)
        }
        async fn renew(
            &self,
            owner: &str,
            held: &[Lease],
            ttl: Duration,
        ) -> Result<Vec<Lease>, StoreError> {
            self.check()?;
            let mut state = self.0.state.lock().unwrap();
            let now = state.now;
            let mut kept = Vec::new();
            for lease in held {
                if self.0.locked_once.lock().unwrap().remove(&lease.shard) {
                    continue;
                }
                if let Some(slot) = state.slots.get_mut(&lease.shard)
                    && slot.0.as_deref() == Some(owner)
                    && slot.1 == lease.generation
                    && slot.2 > now
                {
                    slot.2 = now + ttl;
                    kept.push(*lease);
                }
            }
            Ok(kept)
        }
        async fn release(&self, owner: &str, held: &[Lease]) -> Result<(), StoreError> {
            self.check()?;
            let mut state = self.0.state.lock().unwrap();
            let now = state.now;
            for lease in held {
                if let Some(slot) = state.slots.get_mut(&lease.shard)
                    && slot.0.as_deref() == Some(owner)
                    && slot.1 == lease.generation
                {
                    slot.2 = now;
                }
            }
            Ok(())
        }
        async fn tenant_weights(&self) -> Result<BTreeMap<TenantId, u16>, StoreError> {
            self.check()?;
            Ok(self.0.state.lock().unwrap().weights.clone())
        }
    }

    const TTL: Duration = Duration::from_secs(15);

    fn keeper(store: &Shared, name: &str) -> Keeper<Shared> {
        Keeper::new(
            store.clone(),
            Placement::new(name),
            KeeperSettings { ttl: TTL },
        )
    }

    /// Everyone runs a round, in order, then time passes: what the engines do every interval.
    async fn rounds(keepers: &[&Keeper<Shared>], store: &Shared, count: usize) {
        for _ in 0..count {
            for keeper in keepers {
                keeper.round().await.unwrap();
            }
            store.advance(KeeperSettings { ttl: TTL }.interval());
        }
    }

    #[tokio::test]
    async fn a_lone_replica_takes_every_shard() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        let round = a.round().await.unwrap();
        assert_eq!(round.held, usize::from(SHARDS));
        assert_eq!(a.placement().owned(), ShardSet::all());
    }

    #[tokio::test]
    async fn replicas_converge_to_a_fair_split_and_every_shard_has_one_owner() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        let b = keeper(&store, "b");
        let c = keeper(&store, "c");
        a.round().await.unwrap();
        rounds(&[&a, &b, &c], &store, 6).await;
        let (na, nb, nc) = (
            a.placement().owned(),
            b.placement().owned(),
            c.placement().owned(),
        );
        assert_eq!(na.len() + nb.len() + nc.len(), usize::from(SHARDS));
        for shard in Shard::all() {
            let owners = [na, nb, nc].iter().filter(|s| s.contains(shard)).count();
            assert_eq!(owners, 1, "shard {shard} has {owners} owners");
        }
        // 64 over 3: the share is 22, so nobody holds more than that and nobody holds nothing.
        for set in [na, nb, nc] {
            assert!((20..=22).contains(&set.len()), "{} shards", set.len());
        }
    }

    #[tokio::test]
    async fn the_shards_of_a_replica_that_stops_are_taken_after_the_lease() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        let b = keeper(&store, "b");
        a.round().await.unwrap();
        rounds(&[&a, &b], &store, 6).await;
        assert_eq!(b.placement().owned().len(), 32);
        // `a` crashes: no more rounds from it. Before the lease ends `b` cannot take its shards.
        store.advance(Duration::from_secs(5));
        b.round().await.unwrap();
        assert_eq!(b.placement().owned().len(), 32);
        store.advance(TTL);
        rounds(&[&b], &store, 2).await;
        assert_eq!(b.placement().owned(), ShardSet::all());
    }

    #[tokio::test]
    async fn a_shard_skipped_by_the_renewal_for_a_moment_is_adopted_in_the_same_round_and_not_lost()
    {
        let store = Shared::new();
        let a = keeper(&store, "a");
        a.round().await.unwrap();
        let busy = Shard::new(5).unwrap();
        // An action holds the row of shard 5 `FOR SHARE` as the renewal passes: the renewal does not
        // wait for it and does not renew it; the lock is gone when the keeper reads the table and
        // renews the leases it holds and its snapshot does not know.
        store.0.locked_once.lock().unwrap().insert(busy);
        store.advance(KeeperSettings { ttl: TTL }.interval());
        let round = a.round().await.unwrap();
        assert_eq!(round.lost, 0, "it was skipped, not lost");
        assert_eq!(round.held, usize::from(SHARDS));
        assert!(a.placement().owned().contains(busy));
        assert_eq!(
            store.owner_of(busy).unwrap().1,
            1,
            "no takeover, same generation"
        );
    }

    #[tokio::test]
    async fn two_failed_rounds_in_a_row_do_not_lose_the_leases() {
        // The interval is a fifth of the lease: with the pause after a round and its jitter, a replica
        // whose database fails for two rounds still renews before the lease ends.
        let store = Shared::new();
        let a = keeper(&store, "a");
        a.round().await.unwrap();
        let slowest = KeeperSettings { ttl: TTL }.interval().mul_f64(1.2);
        store.down(true);
        for _ in 0..2 {
            store.advance(slowest);
            assert!(a.round().await.is_err());
        }
        store.down(false);
        store.advance(slowest);
        let round = a.round().await.unwrap();
        assert_eq!(round.lost, 0, "renewed in time: {round:?}");
        assert_eq!(round.held, usize::from(SHARDS));
    }

    #[tokio::test]
    async fn a_replica_that_leaves_cleanly_hands_over_at_once() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        let b = keeper(&store, "b");
        a.round().await.unwrap();
        rounds(&[&a, &b], &store, 6).await;
        a.leave().await;
        assert!(a.placement().owned().is_empty());
        // No time passes: the lease did not have to run out.
        b.round().await.unwrap();
        b.round().await.unwrap();
        assert_eq!(b.placement().owned(), ShardSet::all());
    }

    #[tokio::test]
    async fn a_takeover_raises_the_generation_so_the_old_one_is_fenced() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        a.round().await.unwrap();
        let shard = Shard::new(7).unwrap();
        let old = a
            .placement()
            .snapshot()
            .lease_of(shard, Instant::now())
            .unwrap();
        // `a` is paused for longer than the lease; `b` arrives and takes everything.
        store.advance(TTL * 2);
        let b = keeper(&store, "b");
        b.round().await.unwrap();
        let (owner, generation) = store.owner_of(shard).unwrap();
        assert_eq!(owner, "b");
        assert!(
            generation > old.generation,
            "{generation} against {}",
            old.generation
        );
        // When `a` wakes up it cannot renew its way back: the generation it holds is not the shard's.
        let renewed = store.renew("a", &[old], TTL).await.unwrap();
        assert!(renewed.is_empty());
    }

    #[tokio::test]
    async fn a_replica_that_cannot_reach_the_database_stops_on_its_own() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        a.round().await.unwrap();
        assert!(!a.placement().owned().is_empty());
        store.down(true);
        assert!(a.round().await.is_err());
        // The snapshot still stands until its time is out, then there is nothing, with no call made.
        let snapshot = a.placement().snapshot();
        assert!(!snapshot.shards(Instant::now()).is_empty());
        let after_the_lease = Instant::now() + TTL + Duration::from_millis(1);
        assert!(snapshot.shards(after_the_lease).is_empty());
        assert!(
            snapshot
                .lease_of(Shard::new(0).unwrap(), after_the_lease)
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_lease_lost_while_away_is_counted_and_taken_again_with_a_new_generation() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        a.round().await.unwrap();
        store.advance(TTL * 2);
        let round = a.round().await.unwrap();
        assert_eq!(round.lost, usize::from(SHARDS));
        assert_eq!(
            round.held,
            usize::from(SHARDS),
            "it takes them again, as new leases"
        );
        let lease = a
            .placement()
            .snapshot()
            .lease_of(Shard::new(0).unwrap(), Instant::now())
            .unwrap();
        assert_eq!(lease.generation, 2);
    }

    #[tokio::test]
    async fn leases_whose_acquire_answer_was_lost_are_adopted_and_not_left_to_run_out() {
        let store = Shared::new();
        let a = keeper(&store, "a");
        *store.0.lose_the_answer.lock().unwrap() = true;
        assert!(a.round().await.is_err());
        // The database holds every shard for `a`; `a` has been told nothing and works on none.
        assert!(a.placement().owned().is_empty());
        assert_eq!(
            store.owner_of(Shard::new(5).unwrap()),
            Some(("a".to_owned(), 1))
        );

        // The next round finds them in the table, renews them and works on them: no wait for a lease
        // to run out, no new generation.
        let round = a.round().await.unwrap();
        assert_eq!(round.held, usize::from(SHARDS));
        assert_eq!(a.placement().owned(), ShardSet::all());
        let lease = a
            .placement()
            .snapshot()
            .lease_of(Shard::new(5).unwrap(), Instant::now())
            .unwrap();
        assert_eq!(lease.generation, 1);
        // Another replica of the same name would be a mistake the name's random suffix prevents; a
        // lease held by someone else is never adopted.
        let b = keeper(&store, "b");
        b.round().await.unwrap();
        assert!(
            !b.placement()
                .owned()
                .iter()
                .any(|s| a.placement().owned().contains(s))
                || b.placement().owned().is_empty()
        );
    }

    #[tokio::test]
    async fn the_weights_travel_with_the_snapshot() {
        let store = Shared::new();
        let tenant = TenantId::generate();
        store.0.state.lock().unwrap().weights.insert(tenant, 8);
        let a = keeper(&store, "a");
        a.round().await.unwrap();
        assert_eq!(a.placement().snapshot().weights().of(tenant), 8);
        assert_eq!(
            a.placement().snapshot().weights().of(TenantId::generate()),
            1
        );
    }
}
