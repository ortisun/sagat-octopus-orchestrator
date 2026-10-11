//! Which scheduler shard a piece of work belongs to, and how replicas share the shards.
//!
//! The work of the scheduler (timers, retries, lost leases, the outbox...) is split into a fixed
//! number of shards by the identifier of the run it concerns, so that each replica of the engine can
//! look at its own shards and at no one else's. A shard is owned by one replica at a time, by a
//! lease whose *generation* grows each time the owner changes: an owner that was slow, paused or cut
//! off from the database and comes back finds that its generation is no longer the shard's, and
//! what it was about to do is refused (the fence). The leases and the fence live in the database;
//! this module is the part that needs no database: which shard, how many each replica should hold,
//! and what a replica should do next given what it sees.

use std::collections::BTreeMap;
use std::time::Duration;

use uuid::Uuid;

/// How many shards there are. Fixed for the life of a database: the shard of a run is part of the
/// indexes (`run_shard(...)` in SQL is the same function), so changing it means rebuilding them.
/// A power of two no larger than 64, so that a set of shards is one machine word.
pub const SHARDS: u8 = 64;

const _: () = assert!(SHARDS.is_power_of_two() && SHARDS <= 64);

/// One of the [`SHARDS`] shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shard(u8);

impl Shard {
    /// The shard with this index, when there is one.
    #[must_use]
    pub const fn new(index: u8) -> Option<Self> {
        if index < SHARDS {
            Some(Self(index))
        } else {
            None
        }
    }

    /// The shard of the work that concerns this identifier (a run, usually). The last byte of a
    /// UUIDv7 is random, so the shards fill evenly; the same function in SQL is
    /// `run_shard(uuid) = get_byte(uuid_send(uuid), 15) & 63`.
    #[must_use]
    pub fn of(id: Uuid) -> Self {
        Self(id.as_bytes()[15] & (SHARDS - 1))
    }

    #[must_use]
    pub const fn index(self) -> u8 {
        self.0
    }

    /// Every shard, in order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..SHARDS).map(Self)
    }
}

impl std::fmt::Display for Shard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A set of shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShardSet(u64);

impl ShardSet {
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Every shard.
    #[must_use]
    pub const fn all() -> Self {
        if SHARDS == 64 {
            Self(u64::MAX)
        } else {
            Self((1u64 << SHARDS) - 1)
        }
    }

    pub fn insert(&mut self, shard: Shard) {
        self.0 |= 1u64 << shard.0;
    }

    pub fn remove(&mut self, shard: Shard) {
        self.0 &= !(1u64 << shard.0);
    }

    #[must_use]
    pub const fn contains(self, shard: Shard) -> bool {
        self.0 & (1u64 << shard.0) != 0
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = Shard> {
        Shard::all().filter(move |s| self.contains(*s))
    }

    /// The indexes, for a query (`run_shard(run_id) = ANY($1)`).
    #[must_use]
    pub fn indexes(self) -> Vec<i16> {
        self.iter().map(|s| i16::from(s.0)).collect()
    }
}

impl FromIterator<Shard> for ShardSet {
    fn from_iter<T: IntoIterator<Item = Shard>>(iter: T) -> Self {
        let mut set = Self::empty();
        for shard in iter {
            set.insert(shard);
        }
        set
    }
}

/// How many shards each of `owners` replicas should hold so that every shard has one: the larger
/// share when they do not divide evenly. Zero owners are taken as one (the replica asking).
#[must_use]
pub fn fair_share(owners: usize) -> usize {
    usize::from(SHARDS).div_ceil(owners.max(1))
}

/// What a replica sees of one shard's lease: who holds it and whether the lease is still good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub owner: Option<String>,
    pub generation: u64,
    /// How long the lease has left; zero when it has run out.
    pub remaining: Duration,
}

impl Held {
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.owner.is_some() && !self.remaining.is_zero()
    }

    #[must_use]
    pub fn is_held_by(&self, owner: &str) -> bool {
        self.is_live() && self.owner.as_deref() == Some(owner)
    }
}

/// What a replica should try next.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Shards nobody holds (never held, released, or expired) that it should take, up to its share.
    pub acquire: Vec<Shard>,
    /// Shards it holds beyond its share, which another replica is waiting to take over.
    pub release: Vec<Shard>,
}

/// The next step for `me`, given every shard's lease and how many replicas are alive (counting
/// `me`). Pure: the database decides who wins a race, this only chooses what to try.
///
/// - the share is [`fair_share`] of the live replicas;
/// - below its share, `me` takes free shards, the lowest numbered first, so that replicas that
///   start together contend for the same few and the loser moves on to the next;
/// - above its share it gives back the highest numbered ones it holds: the replica that came up and
///   found nothing to take is waiting for exactly this.
///
/// A replica that is alive but holds nothing must be counted, or the others would never give it
/// its share: that is why the replicas announce themselves apart from the leases.
#[must_use]
pub fn plan(me: &str, live_replicas: usize, shards: &BTreeMap<Shard, Held>) -> Plan {
    let share = fair_share(live_replicas);
    let mine: Vec<Shard> = shards
        .iter()
        .filter(|(_, held)| held.is_held_by(me))
        .map(|(shard, _)| *shard)
        .collect();
    let mut plan = Plan::default();
    if mine.len() < share {
        plan.acquire = Shard::all()
            .filter(|s| shards.get(s).is_none_or(|held| !held.is_live()))
            .take(share - mine.len())
            .collect();
    } else if mine.len() > share {
        let over = mine.len() - share;
        plan.release = mine.into_iter().rev().take(over).collect();
    }
    plan
}
