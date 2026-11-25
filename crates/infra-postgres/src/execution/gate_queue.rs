//! The queue of a gated step (feature 022): what the grant routine (T26, T022-097) and the timeout routine (T27,
//! T022-098) decide about one waiter, in the waiter's run transaction, under one sorted lock of its keys (semaphores,
//! then windows, D022-038; ADR-0089 decisions 3 to 5; ADR-0094 pending for where the readiness decision is taken).
//!
//! A gate admits a waiter at position `p` (the eligible waiters of the key ahead of it) when the key's counted holders
//! or grants plus `p` are below the waiter's own limit: the reservation rule of ADR-0089 decision 4. A waiter ahead is
//! eligible when it is not past its deadline, its run is live and not paused, and its own limit leaves room for it over
//! its own window (an older waiter refused by its own lower limit does not block; one refused only by its other gate
//! keeps its room). Every limit is the lower of the one filed in the waiter row and the namespace's current ceiling
//! (decisions D022-071 and D022-074), and a grant records the limit it was decided with (decision D022-075 item 7).
//! Both or neither (FR-022-044): every gate the step waits for admits it, or nothing is written. A waiter past its
//! deadline is never granted: it is the timeout routine's.

use sagat_domain::execution::{GateChange, GateEvent, GatePool, GateRef, GateTarget, KeyLabel};
use sagat_domain::workflow::{FlowControlCeilings, GateKind, GateLevel, RatePeriod, ResolvedGroup};

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

/// The level of a waiter and the `node` column of its rows: a step's node, or `''` for the run's start (T022-022).
fn level_of(node: Option<&NodeId>) -> (GateLevel, &str) {
    node.map_or((GateLevel::Run, ""), |node| {
        (GateLevel::Step, node.as_str())
    })
}

/// The namespace's ceiling for a window of `per`, as a bound parameter; without ceilings no bound.
pub(super) fn ceiling_of(ceilings: Option<&FlowControlCeilings>, per: RatePeriod) -> i64 {
    match ceilings {
        Some(ceilings) => {
            let ceiling = match per {
                RatePeriod::Second => ceilings.rate_per_second,
                RatePeriod::Minute => ceilings.rate_per_minute,
                RatePeriod::Hour => ceilings.rate_per_hour,
            };
            i64::try_from(ceiling).unwrap_or(i64::MAX)
        }
        None => i64::MAX,
    }
}

/// The namespace's `concurrency_limit`, as a bound parameter; without ceilings no bound.
pub(super) fn concurrency_ceiling(ceilings: Option<&FlowControlCeilings>) -> i64 {
    ceilings.map_or(i64::MAX, |ceilings| {
        i64::try_from(ceilings.concurrency_limit).unwrap_or(i64::MAX)
    })
}

/// The limit `queued` is evaluated against now: the lower of its filed limit and the namespace's current ceiling, of its
/// window's length for a slot (D022-071; D022-009), `concurrency_limit` for a permit (D022-074).
fn effective_limit(queued: &Queued, ceilings: Option<&FlowControlCeilings>) -> i64 {
    let own = i64::from(queued.limit);
    match queued.kind {
        GateKind::RateLimit => match period_of(queued.period_ms) {
            Some(per) => own.min(ceiling_of(ceilings, per)),
            None => own,
        },
        GateKind::Concurrency => own.min(concurrency_ceiling(ceilings)),
    }
}

/// The window length of a filed `period_ms`.
const fn period_of(period_ms: i32) -> Option<RatePeriod> {
    match period_ms {
        1_000 => Some(RatePeriod::Second),
        60_000 => Some(RatePeriod::Minute),
        3_600_000 => Some(RatePeriod::Hour),
        _ => None,
    }
}

impl PgRunTx {
    /// The namespace's ceilings, read in the decision's transaction (decisions D022-071 and D022-074); none for a step
    /// without waiter rows.
    async fn ceilings(
        &mut self,
        queued: &[Queued],
    ) -> Result<Option<FlowControlCeilings>, StoreError> {
        if queued.is_empty() {
            return Ok(None);
        }
        crate::quotas::step_gate_ceilings(&mut self.tx, self.info.tenant_id, self.info.namespace_id)
            .await
            .map(Some)
    }

    /// The waiter rows of `node`'s attempt `attempt` (of the run's start when `node` is `None`), rate first, under one sorted lock of their keys, and the database
    /// clock read after the locks (D022-027).
    async fn locked_queue(
        &mut self,
        node: Option<&NodeId>,
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
                  WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = $5 AND node = $3 AND attempt = $4 \
                 UNION ALL \
                 SELECT 'CONCURRENCY', group_name, key_digest, declared_limit, 0, enqueued_at, deadline_at \
                   FROM concurrency_waiters \
                  WHERE tenant_id = $1 AND run_id = $2 AND permit_scope = $5 AND node = $3 AND attempt = $4 \
                 ORDER BY 1 DESC",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(level_of(node).1)
            .bind(attempt_column)
            .bind(level_of(node).0.as_str())
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
        let declared = match node {
            Some(node) => self.state.declared_gates(node),
            None => sagat_domain::execution::DeclaredGates::of(self.def.run_flow_control()),
        }
        .ok_or_else(|| corrupt("a queued waiter whose level declares no gate"))?;
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
                    level: level_of(node).0,
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
    /// ahead of it are below its effective limit (ADR-0089 decision 4). A waiter ahead is counted over its own window: the
    /// key's grants are counted once for each window length up to the longest its window row has seen (`max_period_ms`,
    /// which is at least every waiter's), and each waiter compared with the count of its own (D022-009; decision D022-075
    /// item 4). The key row is locked.
    async fn admits(
        &mut self,
        queued: &Queued,
        node: Option<&NodeId>,
        now: OffsetDateTime,
        counted_at: OffsetDateTime,
        ceilings: Option<&FlowControlCeilings>,
    ) -> Result<bool, StoreError> {
        let statement = match queued.kind {
            GateKind::Concurrency => {
                "WITH taken AS (SELECT holders::bigint AS n FROM concurrency_semaphores \
                                 WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = $11 \
                                   AND group_name = $4 AND key_digest = $5) \
                 SELECT taken.n, (SELECT count(*) FROM (SELECT 1 FROM concurrency_waiters w \
                                   JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 \
                                    AND w.permit_scope = $11 AND w.group_name = $4 AND w.key_digest = $5 \
                                    AND (w.enqueued_at, w.run_id, w.node) < ($6, $7, $8) \
                                    AND w.deadline_at > $12 \
                                    AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused \
                                    AND least(w.declared_limit, $16::bigint) > taken.n AND $9::bigint >= 0 \
                                    AND $17::timestamptz IS NOT NULL \
                                    AND $13::bigint >= 0 AND $14::bigint >= 0 AND $15::bigint >= 0 LIMIT $10) ahead) \
                   FROM taken"
            }
            // a step's grant counts while it is unleased and for its window after its lease (data-model, Rate limits)
            GateKind::RateLimit => {
                "WITH counted AS ( \
                   SELECT p.ms, (SELECT count(*) FROM rate_limit_grants g \
                                  WHERE g.tenant_id = k.tenant_id AND g.namespace_id = k.namespace_id \
                                    AND g.pool = k.pool AND g.limit_scope = k.limit_scope \
                                    AND g.group_name = k.group_name AND g.key_digest = k.key_digest \
                                    AND (g.leased_at IS NULL \
                                         OR g.leased_at > least($17::timestamptz, $12) \
                                                          - make_interval(secs => p.ms / 1000.0))) AS n \
                     FROM (VALUES (1000), (60000), (3600000)) p(ms), rate_limit_windows k \
                    WHERE k.tenant_id = $1 AND k.namespace_id = $2 AND k.pool = $3 AND k.limit_scope = $11 \
                      AND k.group_name = $4 AND k.key_digest = $5 \
                      AND p.ms <= greatest(k.max_period_ms, $9::bigint)) \
                 SELECT (SELECT c.n FROM counted c WHERE c.ms = $9::bigint), \
                        (SELECT count(*) FROM (SELECT 1 FROM rate_limit_waiters w \
                                   JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                                   LEFT JOIN counted c ON c.ms = w.period_ms \
                                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 \
                                    AND w.limit_scope = $11 AND w.group_name = $4 AND w.key_digest = $5 \
                                    AND (w.enqueued_at, w.run_id, w.node) < ($6, $7, $8) \
                                    AND w.deadline_at > $12 \
                                    AND r.status IN ('RUNNING', 'WAITING') AND NOT r.paused \
                                    AND least(w.declared_limit, CASE w.period_ms WHEN 1000 THEN $13 \
                                              WHEN 60000 THEN $14 WHEN 3600000 THEN $15 \
                                              ELSE w.declared_limit END) > coalesce(c.n, 0) \
                                    AND $16::bigint >= 0 LIMIT $10) ahead)"
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
            .bind(level_of(node).1)
            .bind(i64::from(queued.period_ms))
            // no more of the queue ahead is read than can matter: the waiter's own limit
            .bind(i64::from(queued.limit))
            .bind(level_of(node).0.as_str())
            // the decision's instant, the one its grant is journaled at: the deadlines are checked at it, and a window's
            // grants are counted at the earlier of it and `counted_at` ($17), which counts every grant made since and
            // keeps those that aged out meanwhile (never an over-grant; correctness review Major 2 of 2026-11-20)
            .bind(now)
            // the namespace's current ceiling of each window length (D022-071): a waiter, ahead or deciding, is
            // evaluated against the lower of its filed limit and the ceiling of its own window's length
            .bind(ceiling_of(ceilings, RatePeriod::Second))
            .bind(ceiling_of(ceilings, RatePeriod::Minute))
            .bind(ceiling_of(ceilings, RatePeriod::Hour))
            // and its current `concurrency_limit` (D022-074)
            .bind(concurrency_ceiling(ceilings))
            // the grants of a window are counted as of the pass's selection of the key (decision D022-073 item 2)
            .bind(counted_at)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(taken + ahead < effective_limit(queued, ceilings))
    }

    /// See [`sagat_application::execution::RunTx::grant_queued_step`].
    pub(super) async fn grant_from_queue(
        &mut self,
        node: Option<&NodeId>,
        attempt: u32,
        counted_at: OffsetDateTime,
    ) -> Result<Option<Vec<GateEvent>>, StoreError> {
        let (queued, now) = self.locked_queue(node, attempt, true).await?;
        if queued.iter().any(|q| q.deadline_at <= now) {
            return Ok(None);
        }
        let ceilings = self.ceilings(&queued).await?;
        for q in &queued {
            if !self
                .admits(q, node, now, counted_at, ceilings.as_ref())
                .await?
            {
                return Ok(None);
            }
        }
        // the limit each grant is decided with, recorded in its row and its journal (decision D022-075 item 7)
        let decided = |q: &Queued| {
            i32::try_from(effective_limit(q, ceilings.as_ref()))
                .map_err(|_| corrupt("declared limit"))
        };
        let attempt_column = i32::try_from(attempt).map_err(|_| corrupt("attempt"))?;
        let mut events = Vec::with_capacity(queued.len());
        // the rows of the grant, rate first (the order the waiter rows were read in)
        for q in &queued {
            let change = match q.kind {
                GateKind::Concurrency => {
                    self.move_row(
                        "DELETE FROM concurrency_waiters WHERE tenant_id = $1 AND run_id = $2 AND permit_scope = $5 \
                         AND node = $3 AND attempt = $4",
                        node,
                        attempt_column,
                    )
                    .await?;
                    sqlx::query(
                        "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, \
                         pool, group_name, key_digest, declared_limit, acquired_at) \
                         VALUES ($1, $2, $3, $11, $4, $5, $6, $7, $8, $9, $10)",
                    )
                    .bind(self.info.tenant_id.as_uuid())
                    .bind(self.info.namespace_id.as_uuid())
                    .bind(self.info.run_id.as_uuid())
                    .bind(level_of(node).1)
                    .bind(attempt_column)
                    .bind(GatePool::Ordinary.as_str())
                    .bind(q.group.as_str())
                    .bind(q.digest.as_slice())
                    .bind(decided(q)?)
                    .bind(now)
                    .bind(level_of(node).0.as_str())
                    .execute(&mut *self.tx)
                    .await
                    .map_err(store_error)?;
                    GateChange::ConcurrencyAcquired
                }
                GateKind::RateLimit => {
                    self.move_row(
                        "DELETE FROM rate_limit_waiters WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = $5 \
                         AND node = $3 AND attempt = $4",
                        node,
                        attempt_column,
                    )
                    .await?;
                    sqlx::query(
                        "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                         group_name, key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
                         VALUES ($1, $2, $3, $4, $5, $6, $12, $7, $8, $9, $10, $11, \
                                 CASE WHEN $12 = 'RUN' THEN $11 END, run_shard($3))",
                    )
                    .bind(self.info.tenant_id.as_uuid())
                    .bind(self.info.namespace_id.as_uuid())
                    .bind(self.info.run_id.as_uuid())
                    .bind(level_of(node).1)
                    .bind(attempt_column)
                    .bind(GatePool::Ordinary.as_str())
                    .bind(q.group.as_str())
                    .bind(q.digest.as_slice())
                    .bind(decided(q)?)
                    .bind(q.period_ms)
                    .bind(now)
                    .bind(level_of(node).0.as_str())
                    .execute(&mut *self.tx)
                    .await
                    .map_err(store_error)?;
                    // the waiters behind it wake when this slot may first stop counting (correctness review Major 4)
                    super::readiness_gate::window_due_after_grant(
                        &mut self.tx,
                        (
                            self.info.tenant_id,
                            self.info.namespace_id,
                            level_of(node).0,
                            &q.group,
                        ),
                        &q.digest,
                        now,
                    )
                    .await?;
                    GateChange::RateGranted {
                        limit: u32::try_from(decided(q)?).map_err(|_| corrupt("declared limit"))?,
                        per: period(q.period_ms)?,
                        granted_at_us: i64::try_from(now.unix_timestamp_nanos() / 1_000)
                            .map_err(|_| corrupt("instant"))?,
                        from_queue: true,
                    }
                }
            };
            let outcomes = if node.is_some() {
                &mut self.gate_outcomes
            } else {
                &mut self.run_gate_outcomes
            };
            outcomes.push((q.kind, GateOutcome::GrantedFromQueue));
            events.push(GateEvent {
                target: node.map_or(GateTarget::Run, |node| GateTarget::Step {
                    node: node.clone(),
                    attempt,
                }),
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
        node: Option<&NodeId>,
        attempt: u32,
    ) -> Result<GateKind, StoreError> {
        // read without the keys' locks (correctness review M2, 2026-11-19): the rows go with the events, under the one
        // sorted lock of the round or of `persist`, so a gated failure route cannot lock a key out of order; the gate
        // named is the one that refused at the read (rate first, FR-022-052), which a concurrent commit may change
        let (queued, now) = self.locked_queue(node, attempt, false).await?;
        let ceilings = self.ceilings(&queued).await?;
        // rate first (FR-022-052): the first gate that refuses it now, else the first it waits for
        for q in &queued {
            if !self.admits(q, node, now, now, ceilings.as_ref()).await? {
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
        node: Option<&NodeId>,
        attempt: i32,
    ) -> Result<(), StoreError> {
        let deleted = sqlx::query(delete)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.run_id.as_uuid())
            .bind(level_of(node).1)
            .bind(attempt)
            .bind(level_of(node).0.as_str())
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
