//! The queue of a gated step (feature 022): what the grant routine (T26, T022-097) and the timeout routine (T27,
//! T022-098) decide about one waiter, in the waiter's run transaction, under one sorted lock of its keys (semaphores,
//! then windows, D022-038; ADR-0089 decisions 3 to 5; ADR-0094 pending for where the readiness decision is taken).
//!
//! A gate admits a waiter at position `p` (the eligible waiters of the key ahead of it) when the key's counted holders
//! or grants plus `p` are below the waiter's own limit: the reservation rule of ADR-0089 decision 4. A waiter ahead is
//! eligible when it is not past its deadline, its run is live and not paused, and its own limit leaves room for it
//! (an older waiter refused by its own lower limit does not block; one refused only by its other gate keeps its room).
//! Both or neither (FR-022-044): every gate the step waits for admits it, or nothing is written. A waiter past its
//! deadline is never granted: it is the timeout routine's.

use sagat_domain::execution::{GateChange, GateEvent, GatePool, GateRef, GateTarget, KeyLabel};
use sagat_domain::workflow::{GateKind, GateLevel, RatePeriod, ResolvedGroup};

use super::readiness_gate::GateOutcome;
use super::*;
use crate::flow_control::{ExistingKey, KeyTable, lock_existing};

/// A waiter row as read: kind, group, digest, limit, period, enqueued and deadline instants.
type QueuedRow = (
    String,
    String,
    Vec<u8>,
    i32,
    i32,
    OffsetDateTime,
    OffsetDateTime,
);

/// One waiter row of the step, as stored.
struct Queued {
    kind: GateKind,
    group: ResolvedGroup,
    digest: [u8; 32],
    limit: i32,
    period_ms: i32,
    enqueued_at: OffsetDateTime,
    deadline_at: OffsetDateTime,
}

impl Queued {
    const fn table(&self) -> KeyTable {
        match self.kind {
            GateKind::Concurrency => KeyTable::Semaphores,
            GateKind::RateLimit => KeyTable::Windows,
        }
    }
}

impl PgRunTx {
    /// The waiter rows of `node`'s attempt `attempt`, rate first, under one sorted lock of their keys, and the database
    /// clock read after the locks (D022-027).
    async fn locked_queue(
        &mut self,
        node: &NodeId,
        attempt: u32,
        lock: bool,
    ) -> Result<(Vec<Queued>, OffsetDateTime), StoreError> {
        if lock && self.flow_control_keys.is_none() {
            // a process that does not run the gate decides nothing (fail closed, D022-037)
            return Err(StoreError::FlowControl(
                sagat_application::flow_control::FlowControlRefusal::BinaryCapabilityMissing,
            ));
        }
        let attempt_column = i32::try_from(attempt).map_err(|_| corrupt("attempt"))?;
        let rows: Vec<QueuedRow> =
            sqlx::query_as(
                "SELECT 'RATE', group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at \
                   FROM rate_limit_waiters \
                  WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = 'STEP' AND node = $3 AND attempt = $4 \
                 UNION ALL \
                 SELECT 'CONCURRENCY', group_name, key_digest, declared_limit, 0, enqueued_at, deadline_at \
                   FROM concurrency_waiters \
                  WHERE tenant_id = $1 AND run_id = $2 AND permit_scope = 'STEP' AND node = $3 AND attempt = $4 \
                 ORDER BY 1 DESC",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(node.as_str())
            .bind(attempt_column)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
        let declared = self
            .state
            .declared_gates(node)
            .ok_or_else(|| corrupt("a queued step of a node that declares no gate"))?;
        let mut queued = Vec::with_capacity(rows.len());
        for (kind, group, digest, limit, period_ms, enqueued_at, deadline_at) in rows {
            queued.push(Queued {
                kind: if kind == "RATE" {
                    GateKind::RateLimit
                } else {
                    GateKind::Concurrency
                },
                group: ResolvedGroup::parse(&group).map_err(|_| corrupt("waiter group"))?,
                digest: digest.try_into().map_err(|_| corrupt("waiter key"))?,
                limit,
                period_ms,
                enqueued_at,
                deadline_at,
            });
        }
        // the rows are the step's journal: one per gate it declares, or the projection is corrupt
        let kinds: Vec<_> = queued.iter().map(|q| q.kind).collect();
        if kinds != declared.kinds().collect::<Vec<_>>() {
            return Err(corrupt("the waiter rows of a queued step"));
        }
        for table in [KeyTable::Semaphores, KeyTable::Windows] {
            if !lock {
                break;
            }
            let keys: Vec<ExistingKey> = queued
                .iter()
                .filter(|q| q.table() == table)
                .map(|q| ExistingKey {
                    tenant: self.info.tenant_id,
                    namespace: self.info.namespace_id,
                    pool: GatePool::Ordinary,
                    level: GateLevel::Step,
                    group: q.group.clone(),
                    digest: q.digest,
                })
                .collect();
            if !keys.is_empty() {
                lock_existing(&mut self.tx, table, &keys).await?;
            }
        }
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok((queued, now))
    }

    /// Whether the gate of `queued` admits the waiter now: its key's counted holders or grants plus the eligible waiters
    /// ahead of it are below its own limit (ADR-0089 decision 4). The key row is locked.
    async fn admits(&mut self, queued: &Queued, node: &NodeId) -> Result<bool, StoreError> {
        let statement = match queued.kind {
            GateKind::Concurrency => {
                "WITH taken AS (SELECT holders::bigint AS n FROM concurrency_semaphores \
                                 WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = 'STEP' \
                                   AND group_name = $4 AND key_digest = $5) \
                 SELECT taken.n, (SELECT count(*) FROM (SELECT 1 FROM concurrency_waiters w \
                                   JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 \
                                    AND w.permit_scope = 'STEP' AND w.group_name = $4 AND w.key_digest = $5 \
                                    AND (w.enqueued_at, w.run_id, w.node) < ($6, $7, $8) \
                                    AND w.deadline_at > clock_timestamp() \
                                    AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused \
                                    AND w.declared_limit > taken.n AND $9::bigint >= 0 LIMIT $10) ahead) \
                   FROM taken"
            }
            // a step's grant counts while it is unleased and for its window after its lease (data-model, Rate limits)
            GateKind::RateLimit => {
                "WITH taken AS (SELECT count(*) AS n FROM rate_limit_grants g \
                                 WHERE g.tenant_id = $1 AND g.namespace_id = $2 AND g.pool = $3 \
                                   AND g.limit_scope = 'STEP' AND g.group_name = $4 AND g.key_digest = $5 \
                                   AND (g.leased_at IS NULL \
                                        OR g.leased_at > clock_timestamp() - make_interval(secs => $9::bigint / 1000.0))) \
                 SELECT taken.n, (SELECT count(*) FROM (SELECT 1 FROM rate_limit_waiters w \
                                   JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 \
                                    AND w.limit_scope = 'STEP' AND w.group_name = $4 AND w.key_digest = $5 \
                                    AND (w.enqueued_at, w.run_id, w.node) < ($6, $7, $8) \
                                    AND w.deadline_at > clock_timestamp() \
                                    AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused \
                                    AND w.declared_limit > taken.n LIMIT $10) ahead) \
                   FROM taken"
            }
        };
        let (taken, ahead): (i64, i64) = sqlx::query_as(statement)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(GatePool::Ordinary.as_str())
            .bind(queued.group.as_str())
            .bind(queued.digest.as_slice())
            .bind(queued.enqueued_at)
            .bind(self.info.run_id.as_uuid())
            .bind(node.as_str())
            .bind(i64::from(queued.period_ms))
            // no more of the queue ahead is read than can matter: the waiter's own limit
            .bind(i64::from(queued.limit))
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(taken + ahead < i64::from(queued.limit))
    }

    /// See [`sagat_application::execution::RunTx::grant_queued_step`].
    pub(super) async fn grant_from_queue(
        &mut self,
        node: &NodeId,
        attempt: u32,
    ) -> Result<Option<Vec<GateEvent>>, StoreError> {
        let (queued, now) = self.locked_queue(node, attempt, true).await?;
        if queued.iter().any(|q| q.deadline_at <= now) {
            return Ok(None);
        }
        for q in &queued {
            if !self.admits(q, node).await? {
                return Ok(None);
            }
        }
        let attempt_column = i32::try_from(attempt).map_err(|_| corrupt("attempt"))?;
        let mut events = Vec::with_capacity(queued.len());
        // the rows of the grant, rate first (the order the waiter rows were read in)
        for q in &queued {
            let change = match q.kind {
                GateKind::Concurrency => {
                    self.move_row(
                        "DELETE FROM concurrency_waiters WHERE tenant_id = $1 AND run_id = $2 AND permit_scope = 'STEP' \
                         AND node = $3 AND attempt = $4",
                        node,
                        attempt_column,
                    )
                    .await?;
                    sqlx::query(
                        "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, \
                         pool, group_name, key_digest, declared_limit, acquired_at) \
                         VALUES ($1, $2, $3, 'STEP', $4, $5, $6, $7, $8, $9, $10)",
                    )
                    .bind(self.info.tenant_id.as_uuid())
                    .bind(self.info.namespace_id.as_uuid())
                    .bind(self.info.run_id.as_uuid())
                    .bind(node.as_str())
                    .bind(attempt_column)
                    .bind(GatePool::Ordinary.as_str())
                    .bind(q.group.as_str())
                    .bind(q.digest.as_slice())
                    .bind(q.limit)
                    .bind(now)
                    .execute(&mut *self.tx)
                    .await
                    .map_err(store_error)?;
                    GateChange::ConcurrencyAcquired
                }
                GateKind::RateLimit => {
                    self.move_row(
                        "DELETE FROM rate_limit_waiters WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = 'STEP' \
                         AND node = $3 AND attempt = $4",
                        node,
                        attempt_column,
                    )
                    .await?;
                    sqlx::query(
                        "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                         group_name, key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
                         VALUES ($1, $2, $3, $4, $5, $6, 'STEP', $7, $8, $9, $10, $11, NULL, run_shard($3))",
                    )
                    .bind(self.info.tenant_id.as_uuid())
                    .bind(self.info.namespace_id.as_uuid())
                    .bind(self.info.run_id.as_uuid())
                    .bind(node.as_str())
                    .bind(attempt_column)
                    .bind(GatePool::Ordinary.as_str())
                    .bind(q.group.as_str())
                    .bind(q.digest.as_slice())
                    .bind(q.limit)
                    .bind(q.period_ms)
                    .bind(now)
                    .execute(&mut *self.tx)
                    .await
                    .map_err(store_error)?;
                    GateChange::RateGranted {
                        limit: u32::try_from(q.limit).map_err(|_| corrupt("declared limit"))?,
                        per: period(q.period_ms)?,
                        granted_at_us: i64::try_from(now.unix_timestamp_nanos() / 1_000)
                            .map_err(|_| corrupt("instant"))?,
                        from_queue: true,
                    }
                }
            };
            self.gate_outcomes
                .push((q.kind, GateOutcome::GrantedFromQueue));
            events.push(GateEvent {
                target: GateTarget::Step {
                    node: node.clone(),
                    attempt,
                },
                gate: GateRef {
                    pool: GatePool::Ordinary,
                    group: q.group.clone(),
                    key_label: KeyLabel::of_digest(&q.digest),
                },
                change,
            });
        }
        Ok(Some(events))
    }

    /// See [`sagat_application::execution::RunTx::refusing_gate`].
    pub(super) async fn gate_that_refuses(
        &mut self,
        node: &NodeId,
        attempt: u32,
    ) -> Result<GateKind, StoreError> {
        // read without the keys' locks (correctness review M2, 2026-11-19): the rows go with the events, under the one
        // sorted lock of the round or of `persist`, so a gated failure route cannot lock a key out of order; the gate
        // named is the one that refused at the read (rate first, FR-022-052), which a concurrent commit may change
        let (queued, _) = self.locked_queue(node, attempt, false).await?;
        // rate first (FR-022-052): the first gate that refuses it now, else the first it waits for
        for q in &queued {
            if !self.admits(q, node).await? {
                return Ok(q.kind);
            }
        }
        queued
            .first()
            .map(|q| q.kind)
            .ok_or_else(|| corrupt("a queued step without a waiter row"))
    }

    async fn move_row(
        &mut self,
        delete: &'static str,
        node: &NodeId,
        attempt: i32,
    ) -> Result<(), StoreError> {
        let deleted = sqlx::query(delete)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(node.as_str())
            .bind(attempt)
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?
            .rows_affected();
        if deleted == 1 {
            Ok(())
        } else {
            Err(corrupt("the waiter row a grant takes"))
        }
    }
}

fn period(period_ms: i32) -> Result<RatePeriod, StoreError> {
    [RatePeriod::Second, RatePeriod::Minute, RatePeriod::Hour]
        .into_iter()
        .find(|per| {
            i128::from(period_ms) == i128::try_from(per.duration().as_millis()).unwrap_or(-1)
        })
        .ok_or_else(|| corrupt("rate period"))
}
