//! Which shard, and how replicas come to share them.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use proptest::prelude::*;
use sagat_domain::execution::RunId;
use sagat_domain::sharding::{Held, SHARDS, Shard, ShardSet, fair_share, plan};
use uuid::Uuid;

#[test]
fn a_shard_is_a_function_of_the_identifier_and_the_sql_function_is_the_same() {
    // The same byte the SQL function reads: get_byte(uuid_send(id), 15) & 63.
    let id = Uuid::parse_str("01a12b51-158b-7147-b9b3-847882a5c9ff").expect("uuid");
    assert_eq!(Shard::of(id).index(), 0xff & 63);
    let again = Uuid::parse_str("01a12b51-158b-7147-b9b3-847882a5c940").expect("uuid");
    assert_eq!(Shard::of(again).index(), 0);
    assert_eq!(Shard::of(id), Shard::of(id));
}

#[test]
fn the_ids_the_platform_makes_fill_the_shards_evenly() {
    let mut counts = [0u32; SHARDS as usize];
    for _ in 0..64_000 {
        counts[usize::from(Shard::of(RunId::generate().as_uuid()).index())] += 1;
    }
    // 1000 each on average; a binomial with this many draws stays within a fifth of that.
    for (shard, n) in counts.iter().enumerate() {
        assert!((800..=1200).contains(n), "shard {shard} got {n} of 64000");
    }
}

#[test]
fn a_set_of_shards_holds_what_was_put_in_it() {
    let mut set = ShardSet::empty();
    assert!(set.is_empty());
    for index in [0, 5, 63] {
        set.insert(Shard::new(index).expect("shard"));
    }
    assert_eq!(set.len(), 3);
    assert_eq!(set.indexes(), vec![0, 5, 63]);
    assert!(set.contains(Shard::new(5).expect("shard")));
    set.remove(Shard::new(5).expect("shard"));
    assert!(!set.contains(Shard::new(5).expect("shard")));
    assert_eq!(ShardSet::all().len(), usize::from(SHARDS));
    assert!(Shard::new(SHARDS).is_none());
    let from: ShardSet = Shard::all().take(10).collect();
    assert_eq!(from.len(), 10);
}

#[test]
fn the_share_covers_every_shard_even_when_the_replicas_do_not_divide_them() {
    assert_eq!(fair_share(0), usize::from(SHARDS));
    assert_eq!(fair_share(1), usize::from(SHARDS));
    assert_eq!(fair_share(2), 32);
    assert_eq!(fair_share(3), 22);
    assert_eq!(fair_share(64), 1);
    assert_eq!(fair_share(100), 1);
    for replicas in 1..=100 {
        assert!(fair_share(replicas) * replicas >= usize::from(SHARDS));
    }
}

fn free() -> Held {
    Held {
        owner: None,
        generation: 0,
        remaining: Duration::ZERO,
    }
}

fn held_by(owner: &str, generation: u64) -> Held {
    Held {
        owner: Some(owner.to_owned()),
        generation,
        remaining: Duration::from_secs(10),
    }
}

#[test]
fn an_alone_replica_takes_everything_and_a_late_one_waits_for_what_is_given_back() {
    let none: BTreeMap<Shard, Held> = Shard::all().map(|s| (s, free())).collect();
    let alone = plan("a", 1, &none);
    assert_eq!(alone.acquire.len(), 64);
    assert!(alone.release.is_empty());

    // a holds all 64; b has come up: two replicas, a share of 32 each.
    let all_a: BTreeMap<Shard, Held> = Shard::all().map(|s| (s, held_by("a", 1))).collect();
    let b = plan("b", 2, &all_a);
    assert!(b.acquire.is_empty(), "nothing is free for b yet");
    let a = plan("a", 2, &all_a);
    assert!(a.acquire.is_empty());
    assert_eq!(a.release.len(), 32);
    // It gives back the highest numbered, so that what it keeps is stable.
    assert_eq!(a.release.first(), Shard::new(63).as_ref());
    assert!(a.release.iter().all(|s| s.index() >= 32));
}

#[test]
fn a_lease_that_ran_out_is_free_whoever_the_owner_was() {
    let mut shards: BTreeMap<Shard, Held> = Shard::all().map(|s| (s, held_by("a", 1))).collect();
    shards.insert(
        Shard::new(7).expect("shard"),
        Held {
            owner: Some("a".to_owned()),
            generation: 1,
            remaining: Duration::ZERO,
        },
    );
    let b = plan("b", 2, &shards);
    assert_eq!(b.acquire, vec![Shard::new(7).expect("shard")]);
}

/// A cluster, as the database would run it: every round each replica (in the order given) looks at
/// the table, plans, and the table applies its wishes first come first served; a released shard is
/// free, a shard taken from a new owner gets a new generation.
fn round(
    table: &mut BTreeMap<Shard, Held>,
    generations: &mut BTreeMap<Shard, u64>,
    replicas: &[String],
) {
    for me in replicas {
        let wishes = plan(me, replicas.len(), table);
        for shard in wishes.release {
            table.insert(shard, free());
        }
        for shard in wishes.acquire {
            if table.get(&shard).is_none_or(|h| !h.is_live()) {
                let generation = generations.entry(shard).or_insert(0);
                *generation += 1;
                table.insert(shard, held_by(me, *generation));
            }
        }
    }
}

proptest! {
    #[test]
    fn replicas_that_come_and_go_always_settle_with_every_shard_owned_once(
        steps in proptest::collection::vec((0usize..6, any::<bool>()), 1..30),
    ) {
        let mut table: BTreeMap<Shard, Held> = Shard::all().map(|s| (s, free())).collect();
        let mut generations = BTreeMap::new();
        let mut alive: BTreeSet<usize> = BTreeSet::new();
        for (who, up) in steps {
            if up { alive.insert(who); } else { alive.remove(&who); }
            // A replica that went away leaves its leases to run out.
            for held in table.values_mut() {
                let gone = held.owner.as_deref()
                    .and_then(|o| o.strip_prefix('r')).and_then(|n| n.parse::<usize>().ok())
                    .is_some_and(|n| !alive.contains(&n));
                if gone { held.remaining = Duration::ZERO; }
            }
            let replicas: Vec<String> = alive.iter().map(|n| format!("r{n}")).collect();
            if replicas.is_empty() { continue; }
            // A few rounds: enough for releases and takes to meet.
            for _ in 0..4 { round(&mut table, &mut generations, &replicas); }
            // Every shard has a live owner among the live replicas, and no one holds more than
            // its share.
            let share = fair_share(replicas.len());
            for (shard, held) in &table {
                prop_assert!(held.is_live(), "shard {shard} is nobody's with {replicas:?}");
                prop_assert!(replicas.contains(held.owner.as_ref().expect("owner")));
            }
            for me in &replicas {
                let n = table.values().filter(|h| h.is_held_by(me)).count();
                prop_assert!(n <= share, "{me} holds {n}, more than {share}");
            }
        }
    }
}
