//! Shard ownership in PostgreSQL (`FR-010-001`): the leases, the replicas that are alive, the
//! weights of the tenants, and the fence a scheduler action takes inside its transaction.
//!
//! Every time is the database's clock (`clock_timestamp()`), so replicas whose own clocks disagree
//! agree on who owns what. A lease is a row of `scheduler_shards`:
//!
//! - it is **taken** by an `UPDATE ... WHERE expires_at <= now()` (or unowned), which also adds one
//!   to the generation, so two replicas racing for a shard cannot both win (the second finds the
//!   row no longer free);
//! - it is **renewed** by its owner with the same generation, only while it has not run out and is
//!   still the owner's: a replica that was away for longer than the lease cannot renew its way
//!   back, not even into a shard nobody took in the meantime; it takes it again, with a new
//!   generation, so that nothing it started before it was away can still commit;
//! - it is **fenced** by a `SELECT ... FOR SHARE` of the row in the transaction that changes a run:
//!   a takeover (an `UPDATE` of the same row) waits for the transactions that found the lease good,
//!   and the ones that start after find the new generation.

use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::shards::{Lease, ShardStore};
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::{Held, Shard};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};
use crate::pool::BeginSafely;

fn seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

fn shard_of(index: i16) -> Result<Shard, StoreError> {
    u8::try_from(index)
        .ok()
        .and_then(Shard::new)
        .ok_or_else(|| corrupt("shard"))
}

fn generation(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| corrupt("shard generation"))
}

fn indexes(shards: impl Iterator<Item = Shard>) -> Vec<i16> {
    shards.map(|s| i16::from(s.index())).collect()
}

impl ShardStore for PgRunStore {
    async fn announce(&self, owner: &str, ttl: Duration) -> Result<usize, StoreError> {
        let mut tx = self.pool().begin_safely().await.map_err(store_error)?;
        sqlx::query(
            "INSERT INTO scheduler_members (owner, started_at, expires_at) \
             VALUES ($1, clock_timestamp(), clock_timestamp() + make_interval(secs => $2)) \
             ON CONFLICT (owner) DO UPDATE \
               SET expires_at = EXCLUDED.expires_at, \
                   started_at = CASE WHEN scheduler_members.expires_at < clock_timestamp() \
                                     THEN EXCLUDED.started_at ELSE scheduler_members.started_at END",
        )
        .bind(owner)
        .bind(seconds(ttl))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        // The members that went away without saying so are forgotten here, so the table stays small.
        sqlx::query(
            "DELETE FROM scheduler_members WHERE expires_at < clock_timestamp() - interval '1 hour'",
        )
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        let alive: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM scheduler_members WHERE expires_at > clock_timestamp()",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;
        Ok(usize::try_from(alive).unwrap_or(usize::MAX).max(1))
    }

    async fn withdraw(&self, owner: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM scheduler_members WHERE owner = $1")
            .bind(owner)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn leases(&self) -> Result<BTreeMap<Shard, Held>, StoreError> {
        let rows = sqlx::query(
            "SELECT shard, owner, generation, \
                    GREATEST(extract(epoch FROM expires_at - clock_timestamp()), 0)::float8 AS remaining \
             FROM scheduler_shards",
        )
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                let remaining: f64 = col(row, "remaining")?;
                Ok((
                    shard_of(col::<i16>(row, "shard")?)?,
                    Held {
                        owner: col(row, "owner")?,
                        generation: generation(col::<i64>(row, "generation")?)?,
                        remaining: Duration::from_secs_f64(remaining.max(0.0)),
                    },
                ))
            })
            .collect()
    }

    async fn acquire(
        &self,
        owner: &str,
        shards: &[Shard],
        ttl: Duration,
    ) -> Result<Vec<Lease>, StoreError> {
        if shards.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "UPDATE scheduler_shards s \
                SET owner = $1, generation = s.generation + 1, acquired_at = clock_timestamp(), \
                    expires_at = clock_timestamp() + make_interval(secs => $3) \
              WHERE s.shard = ANY($2::smallint[]) AND s.expires_at <= clock_timestamp() \
              RETURNING s.shard, s.generation",
        )
        .bind(owner)
        .bind(indexes(shards.iter().copied()))
        .bind(seconds(ttl))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                Ok(Lease {
                    shard: shard_of(col::<i16>(row, "shard")?)?,
                    generation: generation(col::<i64>(row, "generation")?)?,
                })
            })
            .collect()
    }

    async fn renew(
        &self,
        owner: &str,
        held: &[Lease],
        ttl: Duration,
    ) -> Result<Vec<Lease>, StoreError> {
        if held.is_empty() {
            return Ok(Vec::new());
        }
        let shards: Vec<i16> = held.iter().map(|l| i16::from(l.shard.index())).collect();
        let generations: Vec<i64> = held
            .iter()
            .map(|l| i64::try_from(l.generation).unwrap_or(i64::MAX))
            .collect();
        let rows = sqlx::query(
            "UPDATE scheduler_shards s \
                SET expires_at = clock_timestamp() + make_interval(secs => $4) \
               FROM unnest($2::smallint[], $3::bigint[]) AS h(shard, generation) \
              WHERE s.shard = h.shard AND s.generation = h.generation AND s.owner = $1 \
                AND s.expires_at > clock_timestamp() \
              RETURNING s.shard, s.generation",
        )
        .bind(owner)
        .bind(shards)
        .bind(generations)
        .bind(seconds(ttl))
        .fetch_all(self.pool())
        .await
        .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                Ok(Lease {
                    shard: shard_of(col::<i16>(row, "shard")?)?,
                    generation: generation(col::<i64>(row, "generation")?)?,
                })
            })
            .collect()
    }

    async fn release(&self, owner: &str, held: &[Lease]) -> Result<(), StoreError> {
        if held.is_empty() {
            return Ok(());
        }
        let shards: Vec<i16> = held.iter().map(|l| i16::from(l.shard.index())).collect();
        let generations: Vec<i64> = held
            .iter()
            .map(|l| i64::try_from(l.generation).unwrap_or(i64::MAX))
            .collect();
        sqlx::query(
            "UPDATE scheduler_shards s SET expires_at = clock_timestamp() \
               FROM unnest($2::smallint[], $3::bigint[]) AS h(shard, generation) \
              WHERE s.shard = h.shard AND s.generation = h.generation AND s.owner = $1 \
                AND s.expires_at > clock_timestamp()",
        )
        .bind(owner)
        .bind(shards)
        .bind(generations)
        .execute(self.pool())
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn tenant_weights(&self) -> Result<BTreeMap<TenantId, u16>, StoreError> {
        let rows = sqlx::query("SELECT tenant_id, weight FROM tenant_scheduling")
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
        rows.iter()
            .map(|row| {
                let weight: i16 = col(row, "weight")?;
                Ok((
                    TenantId::new(col::<Uuid>(row, "tenant_id")?),
                    u16::try_from(weight).map_err(|_| corrupt("scheduling weight"))?,
                ))
            })
            .collect()
    }
}
