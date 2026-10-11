//! The leases on the shards of the scheduler's work, on a real PostgreSQL (FR-010-001, T010-006,
//! T010-007): who may take a shard, who may renew it, and what the fence in the transaction of an
//! action says about each of them.

use std::collections::BTreeSet;
use std::time::Duration;

use sagat_application::runs::{Lock, RunStore};
use sagat_application::shards::{Lease, ShardStore};
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::{SHARDS, Shard};
use serde_json::json;
use uuid::Uuid;

use crate::support::engine::{World, chain, world};

const TTL: Duration = Duration::from_secs(30);

fn all() -> Vec<Shard> {
    Shard::all().collect()
}

fn shard(n: u8) -> Shard {
    Shard::new(n).expect("a shard")
}

async fn expire(w: &World, owner: &str) {
    sqlx::query(
        "UPDATE scheduler_shards SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = $1",
    )
    .bind(owner)
    .execute(&w.db.pool)
    .await
    .expect("expire the leases");
}

async fn row(w: &World, shard: Shard) -> (Option<String>, i64) {
    sqlx::query_as("SELECT owner, generation FROM scheduler_shards WHERE shard = $1")
        .bind(i16::from(shard.index()))
        .fetch_one(&w.db.pool)
        .await
        .expect("the shard's row")
}

#[tokio::test]
async fn the_migration_seeds_sixty_four_free_shards() {
    let w = world().await;
    let free: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM scheduler_shards WHERE owner IS NULL AND generation = 0",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(free, i64::from(SHARDS));
    let held = w.store.leases().await.expect("leases");
    assert_eq!(held.len(), usize::from(SHARDS));
    assert!(held.values().all(|h| !h.is_live()), "nobody holds a shard");
}

#[tokio::test]
async fn the_shard_of_an_id_is_the_same_in_the_database_and_in_the_domain() {
    // The indexes are built on `run_shard(id)` and the candidate queries filter on it, while the
    // replica fences on `Shard::of`: if they ever disagree, a run is looked at by one replica and
    // fenced by the lease of another.
    let w = world().await;
    let ids: Vec<Uuid> = (0..2_000)
        .map(|n| {
            if n % 2 == 0 {
                Uuid::now_v7()
            } else {
                Uuid::new_v4()
            }
        })
        .collect();
    let in_database: Vec<i16> = sqlx::query_scalar(
        "SELECT run_shard(id) FROM unnest($1::uuid[]) WITH ORDINALITY AS t(id, n) ORDER BY n",
    )
    .bind(&ids)
    .fetch_all(&w.db.pool)
    .await
    .expect("shards");
    let in_domain: Vec<i16> = ids
        .iter()
        .map(|id| i16::from(Shard::of(*id).index()))
        .collect();
    assert_eq!(in_database, in_domain);
    // And they spread: a UUIDv7's last byte is random, so no shard is empty over two thousand ids.
    let used: BTreeSet<i16> = in_domain.into_iter().collect();
    assert_eq!(used.len(), usize::from(SHARDS));
}

#[tokio::test]
async fn a_free_shard_is_taken_and_the_generation_counts_the_takeovers() {
    let w = world().await;
    let got = w
        .store
        .acquire("alpha", &[shard(3)], TTL)
        .await
        .expect("acquire");
    assert_eq!(
        got,
        vec![Lease {
            shard: shard(3),
            generation: 1
        }]
    );
    // Held and good: nobody else gets it, and nothing changes.
    assert!(
        w.store
            .acquire("beta", &[shard(3)], TTL)
            .await
            .expect("acquire")
            .is_empty()
    );
    assert!(
        w.store
            .acquire("alpha", &[shard(3)], TTL)
            .await
            .expect("acquire")
            .is_empty(),
        "even its owner takes it through a renewal, not by taking it again"
    );
    assert_eq!(row(&w, shard(3)).await, (Some("alpha".into()), 1));

    // It runs out and the other takes it: the generation moves.
    expire(&w, "alpha").await;
    let second = w
        .store
        .acquire("beta", &[shard(3)], TTL)
        .await
        .expect("acquire");
    assert_eq!(second[0].generation, 2);
    assert_eq!(row(&w, shard(3)).await, (Some("beta".into()), 2));

    // And so does the owner that comes back after its lease ran out: it is a new lease.
    expire(&w, "beta").await;
    let third = w
        .store
        .acquire("beta", &[shard(3)], TTL)
        .await
        .expect("acquire");
    assert_eq!(third[0].generation, 3);
}

#[tokio::test]
async fn replicas_racing_for_every_shard_never_both_win_one() {
    let w = world().await;
    let (a, b, c) = (w.replica().await, w.replica().await, w.replica().await);
    let every = all();
    let (ga, gb, gc) = tokio::join!(
        a.store.acquire("alpha", &every, TTL),
        b.store.acquire("beta", &every, TTL),
        c.store.acquire("gamma", &every, TTL),
    );
    let (ga, gb, gc) = (ga.expect("alpha"), gb.expect("beta"), gc.expect("gamma"));
    let won = |leases: &[Lease]| -> BTreeSet<Shard> { leases.iter().map(|l| l.shard).collect() };
    let (sa, sb, sc) = (won(&ga), won(&gb), won(&gc));
    assert_eq!(sa.len() + sb.len() + sc.len(), usize::from(SHARDS));
    assert!(sa.is_disjoint(&sb) && sa.is_disjoint(&sc) && sb.is_disjoint(&sc));
    // Each shard has exactly the owner the lease says.
    for lease in ga.iter().chain(&gb).chain(&gc) {
        assert_eq!(lease.generation, 1);
    }
    let owners: BTreeSet<String> =
        sqlx::query_scalar("SELECT DISTINCT owner FROM scheduler_shards")
            .fetch_all(&w.db.pool)
            .await
            .expect("owners")
            .into_iter()
            .collect();
    assert!(owners.len() <= 3);
}

#[tokio::test]
async fn a_lease_is_renewed_by_its_owner_at_its_generation_and_only_while_it_lasts() {
    let w = world().await;
    let mine = w
        .store
        .acquire("alpha", &[shard(1), shard(2)], Duration::from_secs(5))
        .await
        .expect("acquire");

    // The owner renews, and the lease is longer afterwards.
    let before: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - clock_timestamp())::float8 FROM scheduler_shards WHERE shard = 1",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("remaining");
    assert!(before < 5.0);
    let renewed = w.store.renew("alpha", &mine, TTL).await.expect("renew");
    assert_eq!(renewed, mine, "the generation stays: it is the same lease");
    let after: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - clock_timestamp())::float8 FROM scheduler_shards WHERE shard = 1",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("remaining");
    assert!(after > 25.0, "{after}");

    // Somebody else, or the wrong generation, renews nothing.
    assert!(
        w.store
            .renew("beta", &mine, TTL)
            .await
            .expect("renew")
            .is_empty()
    );
    let stale = [Lease {
        shard: shard(1),
        generation: 0,
    }];
    assert!(
        w.store
            .renew("alpha", &stale, TTL)
            .await
            .expect("renew")
            .is_empty()
    );

    // Once it ran out the owner cannot renew its way back, even if nobody took the shard in the
    // meantime: it takes it again, with a new generation, so nothing it began before can commit.
    expire(&w, "alpha").await;
    assert!(
        w.store
            .renew("alpha", &mine, TTL)
            .await
            .expect("renew")
            .is_empty()
    );
    let again = w
        .store
        .acquire("alpha", &[shard(1)], TTL)
        .await
        .expect("acquire");
    assert_eq!(again[0].generation, 2);
}

#[tokio::test]
async fn a_lease_given_back_can_be_taken_at_once() {
    let w = world().await;
    let mine = w
        .store
        .acquire("alpha", &[shard(9)], TTL)
        .await
        .expect("acquire");
    // Another owner cannot give it back for them.
    w.store.release("beta", &mine).await.expect("release");
    assert!(
        w.store
            .acquire("beta", &[shard(9)], TTL)
            .await
            .expect("acquire")
            .is_empty()
    );
    // Nor at an old generation.
    w.store
        .release(
            "alpha",
            &[Lease {
                shard: shard(9),
                generation: 7,
            }],
        )
        .await
        .expect("release");
    assert!(
        w.store
            .acquire("beta", &[shard(9)], TTL)
            .await
            .expect("acquire")
            .is_empty()
    );
    // Its owner can, and then it is free with no waiting for the lease.
    w.store.release("alpha", &mine).await.expect("release");
    let taken = w
        .store
        .acquire("beta", &[shard(9)], TTL)
        .await
        .expect("acquire");
    assert_eq!(taken[0].generation, 2);
}

#[tokio::test]
async fn the_replicas_alive_are_counted_whether_or_not_they_hold_a_shard() {
    let w = world().await;
    assert_eq!(w.store.announce("alpha", TTL).await.expect("announce"), 1);
    assert_eq!(w.store.announce("beta", TTL).await.expect("announce"), 2);
    assert_eq!(
        w.store.announce("alpha", TTL).await.expect("announce"),
        2,
        "announcing again is not another replica"
    );
    // One that left cleanly is not counted any more.
    w.store.withdraw("beta").await.expect("withdraw");
    assert_eq!(w.store.announce("alpha", TTL).await.expect("announce"), 1);
    // One whose announcement ran out is not either.
    w.store.announce("gamma", TTL).await.expect("announce");
    sqlx::query("UPDATE scheduler_members SET expires_at = clock_timestamp() - interval '1 second' WHERE owner = 'gamma'")
        .execute(&w.db.pool)
        .await
        .expect("expire");
    assert_eq!(w.store.announce("alpha", TTL).await.expect("announce"), 1);
    // And a replica that comes back after a long absence is counted again.
    assert_eq!(w.store.announce("gamma", TTL).await.expect("announce"), 2);
}

#[tokio::test]
async fn the_fence_is_good_for_the_current_lease_and_for_nothing_else() {
    let w = world().await;
    let (wf, _) = w.publish(chain("fenced")).await;
    let run = w.start(wf, &json!({})).await;
    let tenant = w.admin.tenant_id();
    let lease = w
        .store
        .acquire("alpha", &[shard(5)], TTL)
        .await
        .expect("acquire")
        .remove(0);

    let mut tx = w
        .store
        .begin(tenant, run, Lock::Wait)
        .await
        .expect("begin")
        .expect("the run");
    use sagat_application::execution::RunTx;
    assert!(tx.holds_shard("alpha", lease).await.expect("fence"));
    // Not another owner, not another generation, not another shard.
    assert!(!tx.holds_shard("beta", lease).await.expect("fence"));
    assert!(
        !tx.holds_shard(
            "alpha",
            Lease {
                generation: lease.generation + 1,
                ..lease
            }
        )
        .await
        .expect("fence")
    );
    assert!(
        !tx.holds_shard(
            "alpha",
            Lease {
                shard: shard(6),
                ..lease
            }
        )
        .await
        .expect("fence")
    );
    drop(tx);

    // A lease that ran out is not good, even if nobody took the shard.
    expire(&w, "alpha").await;
    let mut tx = w
        .store
        .begin(tenant, run, Lock::Wait)
        .await
        .expect("begin")
        .expect("the run");
    assert!(!tx.holds_shard("alpha", lease).await.expect("fence"));
    drop(tx);

    // And one that another replica took: the old generation is refused, the new one is good.
    let theirs = w
        .store
        .acquire("beta", &[shard(5)], TTL)
        .await
        .expect("acquire")
        .remove(0);
    let mut tx = w
        .store
        .begin(tenant, run, Lock::Wait)
        .await
        .expect("begin")
        .expect("the run");
    assert!(!tx.holds_shard("alpha", lease).await.expect("fence"));
    assert!(tx.holds_shard("beta", theirs).await.expect("fence"));
}

#[tokio::test]
async fn a_takeover_never_overtakes_the_transaction_that_found_the_lease_good() {
    // The fence is only worth something if the shard cannot change hands between the check and the
    // commit of the action. The takeover does not wait for the action (a node lost in the middle of
    // one would hold the shard for as long as the server takes to notice): it skips the shard, takes
    // the free ones in the same call, and asks for the skipped one again at the next round.
    let w = world().await;
    let (wf, _) = w.publish(chain("fenced-race")).await;
    let run = w.start(wf, &json!({})).await;
    let tenant = w.admin.tenant_id();
    let lease = w
        .store
        .acquire("alpha", &[shard(11)], Duration::from_secs(3))
        .await
        .expect("acquire")
        .remove(0);

    let mut tx = w
        .store
        .begin(tenant, run, Lock::Wait)
        .await
        .expect("begin")
        .expect("the run");
    use sagat_application::execution::RunTx;
    assert!(
        tx.holds_shard("alpha", lease).await.expect("fence"),
        "good when it was checked"
    );

    // The lease runs out while the action is still going; the other replica goes for the shard and
    // for another one that is free.
    tokio::time::sleep(Duration::from_millis(3_300)).await;
    let rival = w.replica().await;
    let started = std::time::Instant::now();
    let got = tokio::time::timeout(
        Duration::from_secs(3),
        rival.store.acquire("beta", &[shard(11), shard(12)], TTL),
    )
    .await
    .expect("the takeover does not wait for the action")
    .expect("acquire");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        got.iter().map(|l| l.shard).collect::<Vec<_>>(),
        vec![shard(12)],
        "the free shard is taken, the one the action holds is not"
    );
    assert_eq!(row(&w, shard(11)).await.0.as_deref(), Some("alpha"));

    // The action ends; the next round takes the shard, with a new generation.
    drop(tx);
    let taken = rival
        .store
        .acquire("beta", &[shard(11)], TTL)
        .await
        .expect("acquire");
    assert_eq!(taken[0].generation, lease.generation + 1);
}

#[tokio::test]
async fn an_action_cut_off_in_the_middle_does_not_hold_its_shard_for_longer_than_the_server_lets_it_idle()
 {
    // A node is lost after the fence and before the commit: the server keeps the transaction, and the
    // lock it holds, until it notices the connection is gone. The pools cap how long a transaction may
    // sit idle, so the shard is free again after that.
    use sagat_infra_postgres::{PoolConfig, connect};

    let w = world().await;
    let mut config = PoolConfig::new(w.db.url.clone());
    config.max_connections = 2;
    config.idle_in_transaction_timeout = Duration::from_millis(800);
    let pool = connect(&config).await.expect("pool");
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SELECT shard FROM scheduler_shards WHERE shard = 20 FOR SHARE")
        .execute(&mut *tx)
        .await
        .expect("the fence's lock");
    // The holder goes silent (here it is just not used again) and the server cuts it off.
    tokio::time::sleep(Duration::from_millis(1_800)).await;
    let taken = tokio::time::timeout(
        Duration::from_secs(3),
        w.store.acquire("beta", &[shard(20)], TTL),
    )
    .await
    .expect("the shard is free once the idle transaction was ended")
    .expect("acquire");
    assert_eq!(taken.len(), 1);
    assert!(
        sqlx::query("SELECT 1").execute(&mut *tx).await.is_err(),
        "the server ended it"
    );
}

#[tokio::test]
async fn the_weights_of_the_tenants_are_read_and_bounded() {
    let w = world().await;
    assert!(w.store.tenant_weights().await.expect("weights").is_empty());
    sqlx::query(
        "INSERT INTO tenant_scheduling (tenant_id, weight, updated_at) VALUES ($1, 5, now())",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a weight");
    let weights = w.store.tenant_weights().await.expect("weights");
    assert_eq!(weights.get(&w.admin.tenant_id()), Some(&5));
    assert_eq!(weights.get(&w.other_admin.tenant_id()), None);

    for bad in [0_i16, 17, -1] {
        let refused = sqlx::query(
            "INSERT INTO tenant_scheduling (tenant_id, weight, updated_at) VALUES ($1, $2, now())",
        )
        .bind(w.other_admin.tenant_id().as_uuid())
        .bind(bad)
        .execute(&w.db.pool)
        .await;
        assert!(refused.is_err(), "weight {bad} must be refused");
    }
    // A weight belongs to a tenant that exists.
    let orphan = sqlx::query(
        "INSERT INTO tenant_scheduling (tenant_id, weight, updated_at) VALUES ($1, 2, now())",
    )
    .bind(TenantId::generate().as_uuid())
    .execute(&w.db.pool)
    .await;
    assert!(orphan.is_err());
}
