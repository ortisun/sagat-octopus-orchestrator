//! The readiness gate of feature 022 (T022-096; T28 of the plan; FR-022-051; D022-020, D022-038, D022-044).
//!
//! A step of a node that declares `concurrency` or `rateLimit` leaves the reducer `WAITING` for its gates whatever made
//! it ready (a worker completion, the retry pass, a timer, a signal, an approval, an update, a finished child, an
//! operator's retry, the run's start, a resume). The planner then asks for the decision, and this transaction decides
//! every such step at once: it reads the digest key versions in force, files and locks the keys (semaphores, then
//! windows, each sorted, [`crate::flow_control::file_keys`]) and, for each step in node order, either grants every
//! gate it declares (holder and grant rows, `concurrency.acquired`, `rate_limit.granted`) or queues it for every one
//! (waiter rows, `*.queued` with the earlier of the two deadlines): both or neither (FR-022-044).
//!
//! A gate admits at once only when its counted holders or grants, plus the waiters already queued for the key, are
//! below the acquirer's own limit (the reservation rule of D022-021 for the waiter that would be last): a new arrival
//! never takes a permit or a slot ahead of a queued waiter. The grant routine (T022-097) serves the queue.
//!
//! The rows of a decision are written here, under the keys' locks, before `persist` writes the journal and the
//! projections; `persist` gives back what the planner releases ([`PgRunTx::persist_gate_returns`]) before it touches
//! a usage row, so a semaphore or a window is always locked before the usage rows of slot 8b (D022-038).

use sagat_application::flow_control::{FlowControlRefusal, StepGateKeys};
use sagat_application::keyed_digest::{KeyPurpose, KeyedDigestError};
use sagat_domain::execution::{
    GateChange, GateEvent, GatePhase, GatePool, GateRef, GateSlot, GateTarget, Gates, KeyLabel,
    StepState,
};
use sagat_domain::workflow::{GateKind, GateLevel};
use sha2::{Digest as _, Sha256};

use super::*;
use crate::flow_control::{
    ExistingKey, FiledKey, KeyTable, KeyToFile, file_keys_with, key_versions, lock_existing,
};

/// The longest a gate projection of one step may be (two slots of a 130-character group).
const MAX_PROJECTION_BYTES: usize = 1024;

/// One gate of one step, as this transaction decides it.
struct Pending {
    node: NodeId,
    attempt: u32,
    kind: GateKind,
    group: sagat_domain::workflow::ResolvedGroup,
    limit: u32,
    /// The window of a rate limit, `None` for a concurrency permit.
    per: Option<sagat_domain::workflow::RatePeriod>,
    timeout: std::time::Duration,
    /// Its key, once filed.
    filed: Option<FiledKey>,
}

/// A key row and the window its grants were counted for: (rate, group, digest, period in milliseconds).
type CountKey = (bool, String, [u8; 32], i64);

fn count_key(gate: &Pending, filed: &FiledKey) -> CountKey {
    (
        gate.kind == GateKind::RateLimit,
        gate.group.as_str().to_owned(),
        filed.digest,
        period_ms(gate),
    )
}

/// One gate a transaction gives back, with the key row it is under.
struct GateReturn {
    table: KeyTable,
    change: GateChange,
    node: NodeId,
    attempt: u32,
    key: ExistingKey,
}

/// The counts of one locked key, kept as this transaction adds rows under it.
#[derive(Clone, Copy, Default)]
struct KeyCount {
    /// Holders of a semaphore; grants that count of a window (for the period they were read for).
    taken: i64,
    waiters: i64,
}

impl PgRunTx {
    /// Decides the gates of `steps` under one sorted lock of their keys (see the module).
    ///
    /// # Errors
    /// `BINARY_CAPABILITY_MISSING` when this store has no digest key ring (a process that does not run the gate);
    /// [`StoreError::Unavailable`] when no digest key version is active or a key row vanished during a rotation
    /// (retryable); a store error.
    pub(super) async fn decide_gates(
        &mut self,
        steps: &[StepGateKeys],
        pending_events: &[RunEvent],
    ) -> Result<sagat_application::flow_control::StepGateAnswer, StoreError> {
        let Some(ring) = self.flow_control_keys.clone() else {
            return Err(StoreError::FlowControl(
                FlowControlRefusal::BinaryCapabilityMissing,
            ));
        };
        let tenant = self.info.tenant_id;
        let namespace = self.info.namespace_id;
        let versions = key_versions(&mut self.tx).await?;
        // the namespace's ceilings bound what a declaration asks (G1199; FR-022-047, FR-022-054)
        let ceilings = crate::quotas::step_gate_ceilings(&mut self.tx, tenant, namespace).await?;
        let mut pending = Vec::new();
        let (mut semaphores, mut windows) = (Vec::new(), Vec::new());
        for step in steps {
            let node = self
                .def
                .node(&step.node)
                .ok_or_else(|| StoreError::Internal {
                    detail: "a gated step is not in its definition".into(),
                })?;
            let flow = node.flow_control();
            let declared = [
                (
                    GateKind::RateLimit,
                    flow.rate_limit().map(|policy| {
                        (
                            policy.group(),
                            policy.limit(),
                            Some(policy.per()),
                            policy.queue_timeout(),
                        )
                    }),
                    step.rate.as_deref(),
                ),
                (
                    GateKind::Concurrency,
                    flow.concurrency().map(|policy| {
                        (policy.group(), policy.limit(), None, policy.queue_timeout())
                    }),
                    step.concurrency.as_deref(),
                ),
            ];
            for (kind, policy, canonical) in declared {
                let Some((group, limit, per, timeout)) = policy else {
                    continue;
                };
                let canonical = canonical.ok_or_else(|| StoreError::Internal {
                    detail: "a declared gate of a step came without its key".into(),
                })?;
                let purpose = match kind {
                    GateKind::Concurrency => KeyPurpose::Concurrency,
                    GateKind::RateLimit => KeyPurpose::RateLimit,
                };
                let candidates = ring
                    .candidates(versions, purpose, tenant, namespace, canonical)
                    .map_err(key_refusal)?;
                let group = group.resolve(self.def.name(), Some(&step.node));
                let key = KeyToFile {
                    tenant,
                    namespace,
                    pool: GatePool::Ordinary,
                    level: GateLevel::Step,
                    shard: key_shard(&group, candidates.active.as_bytes()),
                    group: group.clone(),
                    candidates,
                };
                let index = match kind {
                    GateKind::Concurrency => {
                        semaphores.push(key);
                        semaphores.len() - 1
                    }
                    GateKind::RateLimit => {
                        windows.push(key);
                        windows.len() - 1
                    }
                };
                let limit = match kind {
                    GateKind::Concurrency => {
                        u32::try_from(u64::from(limit).min(ceilings.concurrency_limit))
                            .unwrap_or(limit)
                    }
                    // a namespace ceiling of a rate limit is T022-077's
                    GateKind::RateLimit => limit,
                };
                pending.push((
                    index,
                    Pending {
                        node: step.node.clone(),
                        attempt: step.attempt,
                        kind,
                        group,
                        limit,
                        per,
                        timeout: ceilings.queue_timeout(timeout),
                        filed: None,
                    },
                ));
            }
        }
        let refused = self
            .refused_by_ceilings(&pending, &semaphores, &ceilings, versions)
            .await?;
        if !refused.is_empty() {
            // nothing is filed: the domain fails these steps and the next round decides the others
            return Ok(sagat_application::flow_control::StepGateAnswer {
                decisions: Vec::new(),
                refused,
            });
        }
        // what this transaction gives back is locked in the same pass and given back first, so that the counts below
        // see it (a permit released and a successor decided on the same key in one commit) and no key row is locked
        // twice or out of order: semaphores before windows, each sorted (D022-038)
        let returns = self.pending_returns(pending_events).await?;
        let returned = |table: KeyTable| -> Vec<ExistingKey> {
            returns
                .iter()
                .filter(|r| r.table == table)
                .map(|r| r.key.clone())
                .collect()
        };
        let filed_semaphores = file_keys_with(
            &mut self.tx,
            KeyTable::Semaphores,
            &semaphores,
            &returned(KeyTable::Semaphores),
        )
        .await?;
        let filed_windows = file_keys_with(
            &mut self.tx,
            KeyTable::Windows,
            &windows,
            &returned(KeyTable::Windows),
        )
        .await?;
        self.apply_returns(returns).await?;
        for (index, gate) in &mut pending {
            let filed = match gate.kind {
                GateKind::Concurrency => &filed_semaphores,
                GateKind::RateLimit => &filed_windows,
            };
            gate.filed = filed.iter().find(|key| key.index == *index).cloned();
        }
        // the grant instant is the database clock after the keys' locks (D022-027)
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        let mut counts: BTreeMap<CountKey, KeyCount> = BTreeMap::new();
        let mut events = Vec::new();
        let mut by_step: BTreeMap<(NodeId, u32), Vec<Pending>> = BTreeMap::new();
        for (_, gate) in pending {
            by_step
                .entry((gate.node.clone(), gate.attempt))
                .or_default()
                .push(gate);
        }
        for ((node, attempt), gates) in by_step {
            let mut admits = true;
            for gate in &gates {
                let count = self.key_count(&mut counts, gate).await?;
                admits &= count.taken + count.waiters < i64::from(gate.limit);
            }
            let deadline = gates.iter().map(|gate| gate.timeout).min().ok_or_else(|| {
                StoreError::Internal {
                    detail: "a gated step declares no gate".into(),
                }
            })?;
            let deadline = now
                .checked_add(
                    time::Duration::try_from(deadline).map_err(|_| corrupt("queue timeout"))?,
                )
                .ok_or_else(|| corrupt("queue deadline"))?;
            for gate in &gates {
                let filed = gate.filed.as_ref().ok_or_else(|| StoreError::Internal {
                    detail: "a gate's key was not filed".into(),
                })?;
                let change = if admits {
                    self.grant(&node, attempt, gate, filed, now).await?
                } else {
                    self.queue(&node, attempt, gate, filed, now, deadline)
                        .await?
                };
                self.gate_outcomes
                    .push((gate.kind, GateOutcome::of(change)));
                // the span names the group, never the key (FR-022-033)
                let outcome = if admits { "granted" } else { "queued" };
                match gate.kind {
                    GateKind::Concurrency => tracing::info_span!(
                        sagat_telemetry::FlowControlSpan::ConcurrencyAcquire.as_str(),
                        "sagat.concurrency.group" = gate.group.as_str(),
                        "sagat.outcome" = outcome,
                    )
                    .in_scope(|| ()),
                    GateKind::RateLimit => tracing::info_span!(
                        sagat_telemetry::FlowControlSpan::RateLimitAdmit.as_str(),
                        "sagat.rate_limit.group" = gate.group.as_str(),
                        "sagat.outcome" = outcome,
                    )
                    .in_scope(|| ()),
                }
                // the row is the same whatever the window its grants were counted for: every count of it moves
                let (rate, group, digest, _) = count_key(gate, filed);
                let mut moved = false;
                for ((r, g, d, _), entry) in &mut counts {
                    if *r == rate && *g == group && *d == digest {
                        moved = true;
                        if admits {
                            entry.taken += 1;
                        } else {
                            entry.waiters += 1;
                        }
                    }
                }
                if !moved {
                    return Err(corrupt("flow-control key count"));
                }
                events.push(GateEvent {
                    target: GateTarget::Step {
                        node: node.clone(),
                        attempt,
                    },
                    gate: GateRef {
                        pool: GatePool::Ordinary,
                        group: gate.group.clone(),
                        key_label: KeyLabel::of_digest(&filed.digest),
                    },
                    change,
                });
            }
        }
        Ok(sagat_application::flow_control::StepGateAnswer {
            decisions: events,
            refused: Vec::new(),
        })
    }

    /// The steps of this round a namespace ceiling refuses `FLOW_CONTROL_LIMIT` (G1199): a ceiling of zero (the effective
    /// limit or queue timeout), or a new step key beyond the live-keys ceiling (FR-022-054; counted on the semaphores,
    /// D022-043). The keys that exist cost one statement; only a round that would create a key counts the namespace's
    /// keys, under the versions in force only: a row a forced retirement left (ADR-0093 decision 5) is being replaced and
    /// drains on its own, so the ceiling is passed by at most those rows meanwhile. Not serialized between transactions:
    /// concurrent rounds pass it by at most the new keys the others create (G022-002, D022-047, O1173).
    async fn refused_by_ceilings(
        &mut self,
        pending: &[(usize, Pending)],
        semaphores: &[KeyToFile],
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
        versions: sagat_application::keyed_digest::KeyVersions,
    ) -> Result<Vec<(NodeId, sagat_domain::execution::FailureCode)>, StoreError> {
        let code = sagat_domain::execution::FailureCode::flow_control_limit;
        let mut refused: Vec<NodeId> = pending
            .iter()
            .filter(|(_, gate)| gate.limit == 0 || gate.timeout.is_zero())
            .map(|(_, gate)| gate.node.clone())
            .collect();
        if !semaphores.is_empty() {
            let (mut groups, mut digests, mut owners) = (Vec::new(), Vec::new(), Vec::new());
            for (index, key) in semaphores.iter().enumerate() {
                for digest in
                    std::iter::once(&key.candidates.active).chain(key.candidates.retiring.as_ref())
                {
                    groups.push(key.group.as_str().to_owned());
                    digests.push(digest.as_bytes().to_vec());
                    owners.push(i32::try_from(index).map_err(|_| corrupt("key index"))?);
                }
            }
            let existing: Vec<i32> = sqlx::query_scalar(
                "SELECT DISTINCT k.owner FROM unnest($3::text[], $4::bytea[], $5::integer[]) AS k(g, d, owner) \
                  WHERE EXISTS (SELECT 1 FROM concurrency_semaphores s \
                                 WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.pool = 'ORDINARY' \
                                   AND s.permit_scope = 'STEP' AND s.group_name = k.g AND s.key_digest = k.d)",
            )
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(&groups)
            .bind(&digests)
            .bind(&owners)
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
            // a key the round would create, once each (two steps may share one), in the order the steps are decided
            let mut new_keys: Vec<(String, [u8; 32])> = Vec::new();
            let mut new_of_step: Vec<(NodeId, (String, [u8; 32]))> = Vec::new();
            for (index, gate) in pending {
                if gate.kind != GateKind::Concurrency || refused.contains(&gate.node) {
                    continue;
                }
                let exists = i32::try_from(*index).is_ok_and(|i| existing.contains(&i));
                if !exists {
                    let key = &semaphores[*index];
                    let id = (
                        key.group.as_str().to_owned(),
                        *key.candidates.active.as_bytes(),
                    );
                    if !new_keys.contains(&id) {
                        new_keys.push(id.clone());
                    }
                    new_of_step.push((gate.node.clone(), id));
                }
            }
            if !new_keys.is_empty() {
                let in_force: Vec<i16> = std::iter::once(versions.active)
                    .chain(versions.retiring)
                    .map(|v| i16::try_from(v).map_err(|_| corrupt("digest key version")))
                    .collect::<Result<_, _>>()?;
                let live: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM concurrency_semaphores WHERE tenant_id = $1 AND namespace_id = $2 \
                     AND key_version = ANY($3)",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(&in_force)
                .fetch_one(&mut *self.tx)
                .await
                .map_err(store_error)?;
                let room = usize::try_from(
                    ceilings
                        .live_keys
                        .saturating_sub(u64::try_from(live).unwrap_or(u64::MAX)),
                )
                .unwrap_or(usize::MAX);
                let admitted = &new_keys[..new_keys.len().min(room)];
                for (node, id) in new_of_step {
                    if !admitted.contains(&id) && !refused.contains(&node) {
                        refused.push(node);
                    }
                }
            }
        }
        refused.sort();
        refused.dedup();
        Ok(refused.into_iter().map(|node| (node, code())).collect())
    }

    /// The holders (or the grants that count for `gate`'s window) and the waiters of a key, read once per key and
    /// period in this transaction, under its lock.
    async fn key_count(
        &mut self,
        counts: &mut BTreeMap<CountKey, KeyCount>,
        gate: &Pending,
    ) -> Result<KeyCount, StoreError> {
        let filed = gate.filed.as_ref().ok_or_else(|| StoreError::Internal {
            detail: "a gate's key was not filed".into(),
        })?;
        let slot = count_key(gate, filed);
        if let Some(count) = counts.get(&slot) {
            return Ok(*count);
        }
        let query = match gate.kind {
            GateKind::Concurrency => sqlx::query_as::<_, (i64, i64)>(
                "SELECT holders::bigint, waiters::bigint FROM concurrency_semaphores \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = 'STEP' \
                   AND group_name = $4 AND key_digest = $5 AND $6::bigint >= 0",
            ),
            // a step's grant counts while it is unleased and for its window after its lease (data-model, Rate limits)
            GateKind::RateLimit => sqlx::query_as::<_, (i64, i64)>(
                "SELECT (SELECT count(*) FROM rate_limit_grants g \
                          WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                            AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name \
                            AND g.key_digest = w.key_digest \
                            AND (g.leased_at IS NULL \
                                 OR g.leased_at > clock_timestamp() - make_interval(secs => $6::bigint / 1000.0))), \
                        w.waiters::bigint \
                   FROM rate_limit_windows w \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 AND w.limit_scope = 'STEP' \
                    AND w.group_name = $4 AND w.key_digest = $5",
            ),
        };
        let (taken, waiters) = query
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(GatePool::Ordinary.as_str())
            .bind(gate.group.as_str())
            .bind(filed.digest.as_slice())
            .bind(period_ms(gate))
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        let count = KeyCount { taken, waiters };
        counts.insert(slot, count);
        Ok(count)
    }

    async fn grant(
        &mut self,
        node: &NodeId,
        attempt: u32,
        gate: &Pending,
        filed: &FiledKey,
        now: OffsetDateTime,
    ) -> Result<GateChange, StoreError> {
        let attempt_column = i32::try_from(attempt).map_err(|_| corrupt("attempt"))?;
        let limit = i32::try_from(gate.limit).map_err(|_| corrupt("declared limit"))?;
        match (gate.kind, gate.per) {
            (GateKind::Concurrency, _) => {
                sqlx::query(
                    "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, pool, \
                     group_name, key_digest, declared_limit, acquired_at) \
                     VALUES ($1, $2, $3, 'STEP', $4, $5, $6, $7, $8, $9, $10)",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(node.as_str())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                Ok(GateChange::ConcurrencyAcquired)
            }
            (GateKind::RateLimit, Some(per)) => {
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
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(i32::try_from(period_ms(gate)).map_err(|_| corrupt("period"))?)
                .bind(now)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                Ok(GateChange::RateGranted {
                    limit: gate.limit,
                    per,
                    granted_at_us: unix_micros(now)?,
                    from_queue: false,
                })
            }
            (GateKind::RateLimit, None) => Err(corrupt("rate limit without a window")),
        }
    }

    async fn queue(
        &mut self,
        node: &NodeId,
        attempt: u32,
        gate: &Pending,
        filed: &FiledKey,
        now: OffsetDateTime,
        deadline: OffsetDateTime,
    ) -> Result<GateChange, StoreError> {
        let attempt_column = i32::try_from(attempt).map_err(|_| corrupt("attempt"))?;
        let limit = i32::try_from(gate.limit).map_err(|_| corrupt("declared limit"))?;
        let deadline_us = unix_micros(deadline)?;
        match (gate.kind, gate.per) {
            (GateKind::Concurrency, _) => {
                self.key_is_due(KeyTable::Semaphores, gate, filed).await?;
                sqlx::query(
                    "INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                     group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard) \
                     VALUES ($1, $2, $3, $4, 'STEP', $5, $6, $7, $8, $9, $10, $11, run_shard($3))",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(node.as_str())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(now)
                .bind(deadline)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                Ok(GateChange::ConcurrencyQueued {
                    queue_deadline_us: deadline_us,
                })
            }
            (GateKind::RateLimit, Some(per)) => {
                self.key_is_due(KeyTable::Windows, gate, filed).await?;
                sqlx::query(
                    "INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                     group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard) \
                     VALUES ($1, $2, $3, $4, $5, $6, 'STEP', $7, $8, $9, $10, $11, $12, run_shard($3))",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(node.as_str())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(i32::try_from(period_ms(gate)).map_err(|_| corrupt("period"))?)
                .bind(now)
                .bind(deadline)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                Ok(GateChange::RateQueued {
                    limit: gate.limit,
                    per,
                    queue_deadline_us: deadline_us,
                })
            }
            (GateKind::RateLimit, None) => Err(corrupt("rate limit without a window")),
        }
    }

    /// Gives back, before anything else `persist` writes, what the events of this transaction release and the gate
    /// round has not given back already: the holder row of a permit an attempt no longer needs (`concurrency.released`),
    /// the grant row of a slot a step returns unleased (`rate_limit.returned`) and the waiter rows of a stopped run's
    /// step (`*.dequeued`). The key rows are locked first, semaphores then windows, sorted (D022-038). Nothing for a
    /// transaction without a flow-control event.
    pub(super) async fn persist_gate_returns(
        &mut self,
        events: &[RunEvent],
    ) -> Result<(), StoreError> {
        let returns = self.pending_returns(events).await?;
        if returns.is_empty() {
            return Ok(());
        }
        for table in [KeyTable::Semaphores, KeyTable::Windows] {
            let rows: Vec<ExistingKey> = returns
                .iter()
                .filter(|r| r.table == table)
                .map(|r| r.key.clone())
                .collect();
            if !rows.is_empty() {
                lock_existing(&mut self.tx, table, &rows).await?;
            }
        }
        self.apply_returns(returns).await
    }

    /// The gate returns among `events` that have not been applied yet, each with the key row it is under.
    async fn pending_returns(
        &mut self,
        events: &[RunEvent],
    ) -> Result<Vec<GateReturn>, StoreError> {
        let mut returns = Vec::new();
        for event in events {
            let RunEvent::Gate(gate) = event else {
                continue;
            };
            let GateTarget::Step { node, attempt } = &gate.target else {
                return Err(StoreError::Internal {
                    detail:
                        "a run-level flow-control event reached a build without run-level admission"
                            .into(),
                });
            };
            let (table, read) = match gate.change {
                GateChange::ConcurrencyReleased => (
                    KeyTable::Semaphores,
                    "SELECT key_digest FROM concurrency_holders WHERE tenant_id = $1 AND run_id = $2 \
                     AND holder_scope = 'STEP' AND node = $3 AND attempt = $4",
                ),
                GateChange::ConcurrencyDequeued | GateChange::ConcurrencyWaitTimedOut => (
                    KeyTable::Semaphores,
                    "SELECT key_digest FROM concurrency_waiters WHERE tenant_id = $1 AND run_id = $2 \
                     AND permit_scope = 'STEP' AND node = $3 AND attempt = $4",
                ),
                GateChange::RateReturned => (
                    KeyTable::Windows,
                    "SELECT key_digest FROM rate_limit_grants WHERE tenant_id = $1 AND run_id = $2 \
                     AND limit_scope = 'STEP' AND node = $3 AND attempt = $4 AND leased_at IS NULL",
                ),
                GateChange::RateDequeued | GateChange::RateWaitTimedOut => (
                    KeyTable::Windows,
                    "SELECT key_digest FROM rate_limit_waiters WHERE tenant_id = $1 AND run_id = $2 \
                     AND limit_scope = 'STEP' AND node = $3 AND attempt = $4",
                ),
                // decisions wrote their rows under the keys' locks
                GateChange::ConcurrencyQueued { .. }
                | GateChange::ConcurrencyAcquired
                | GateChange::RateQueued { .. }
                | GateChange::RateGranted { .. } => continue,
            };
            let identity = (node.clone(), *attempt, gate.change.name());
            if self.gate_returns_applied.contains(&identity) {
                continue;
            }
            let attempt_column = i32::try_from(*attempt).map_err(|_| corrupt("attempt"))?;
            let digest: Option<Vec<u8>> = sqlx::query_scalar(read)
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(node.as_str())
                .bind(attempt_column)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(store_error)?;
            let digest: [u8; 32] = digest
                .and_then(|d| d.try_into().ok())
                .ok_or_else(|| corrupt("the row a flow-control event gives back"))?;
            returns.push(GateReturn {
                table,
                change: gate.change,
                node: node.clone(),
                attempt: *attempt,
                key: ExistingKey {
                    tenant: self.info.tenant_id,
                    namespace: self.info.namespace_id,
                    pool: gate.gate.pool,
                    level: GateLevel::Step,
                    group: gate.gate.group.clone(),
                    digest,
                },
            });
        }
        Ok(returns)
    }

    /// Deletes the rows of `returns`, whose key rows this transaction holds, and sets the hint of each key a release
    /// or a dequeue may let a waiter through (early, never late).
    async fn apply_returns(&mut self, returns: Vec<GateReturn>) -> Result<(), StoreError> {
        for r in returns {
            let delete = match r.change {
                GateChange::ConcurrencyReleased => {
                    "DELETE FROM concurrency_holders WHERE tenant_id = $1 AND run_id = $2 AND holder_scope = 'STEP' \
                     AND node = $3 AND attempt = $4"
                }
                GateChange::ConcurrencyDequeued | GateChange::ConcurrencyWaitTimedOut => {
                    "DELETE FROM concurrency_waiters WHERE tenant_id = $1 AND run_id = $2 AND permit_scope = 'STEP' \
                     AND node = $3 AND attempt = $4"
                }
                GateChange::RateReturned => {
                    "DELETE FROM rate_limit_grants WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = 'STEP' \
                     AND node = $3 AND attempt = $4 AND leased_at IS NULL"
                }
                GateChange::RateDequeued | GateChange::RateWaitTimedOut => {
                    "DELETE FROM rate_limit_waiters WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = 'STEP' \
                     AND node = $3 AND attempt = $4"
                }
                _ => return Err(corrupt("flow-control return")),
            };
            let deleted = sqlx::query(delete)
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(r.node.as_str())
                .bind(i32::try_from(r.attempt).map_err(|_| corrupt("attempt"))?)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?
                .rows_affected();
            if deleted != 1 {
                return Err(corrupt("the row a flow-control event gives back"));
            }
            // a permit released, a slot returned unleased, a waiter gone: each may let a waiter of the key through
            self.key_is_eligible(r.table, &r.key).await?;
            self.gate_outcomes
                .push((r.change.kind(), GateOutcome::of(r.change)));
            self.gate_returns_applied
                .insert((r.node, r.attempt, r.change.name()));
        }
        Ok(())
    }
}

impl PgRunTx {
    /// A key gains a waiter: unless it is due already, it is due now (correctness review M1, 2026-11-19). The readiness
    /// gate counts every waiter of the key and the grant routine only the eligible ones, so a new waiter behind ineligible
    /// ones may be grantable at once; the next pass decides, and rests the key if not (early, never late).
    async fn key_is_due(
        &mut self,
        table: KeyTable,
        gate: &Pending,
        filed: &FiledKey,
    ) -> Result<(), StoreError> {
        let statement = match table {
            KeyTable::Semaphores => {
                "UPDATE concurrency_semaphores SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = 'STEP' \
                    AND group_name = $4 AND key_digest = $5 AND next_eligible_at IS NULL"
            }
            KeyTable::Windows => {
                "UPDATE rate_limit_windows SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND limit_scope = 'STEP' \
                    AND group_name = $4 AND key_digest = $5 AND next_eligible_at IS NULL"
            }
        };
        sqlx::query(statement)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(GatePool::Ordinary.as_str())
            .bind(gate.group.as_str())
            .bind(filed.digest.as_slice())
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    /// A run paused or resumed: the keys of its queued steps are due now, since a paused waiter is neither granted nor
    /// counted ahead (ADR-0089 decision 4): its pause may let the waiters behind it through, and its resume makes it
    /// grantable again (security review 2, correctness M1 and M4, 2026-11-19).
    pub(super) async fn rearm_queued_keys(&mut self) -> Result<(), StoreError> {
        for statement in [
            "UPDATE concurrency_semaphores s SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
               FROM concurrency_waiters w \
              WHERE w.tenant_id = $1 AND w.run_id = $2 AND w.permit_scope = 'STEP' \
                AND s.tenant_id = w.tenant_id AND s.namespace_id = w.namespace_id AND s.pool = w.pool \
                AND s.permit_scope = w.permit_scope AND s.group_name = w.group_name AND s.key_digest = w.key_digest \
                AND s.next_eligible_at IS NULL",
            "UPDATE rate_limit_windows s SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
               FROM rate_limit_waiters w \
              WHERE w.tenant_id = $1 AND w.run_id = $2 AND w.limit_scope = 'STEP' \
                AND s.tenant_id = w.tenant_id AND s.namespace_id = w.namespace_id AND s.pool = w.pool \
                AND s.limit_scope = w.limit_scope AND s.group_name = w.group_name AND s.key_digest = w.key_digest \
                AND s.next_eligible_at IS NULL",
        ] {
            sqlx::query(statement)
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    /// Sets the next-eligible hint of a step key that has waiters to now (early, never late, ADR-0089).
    async fn key_is_eligible(
        &mut self,
        table: KeyTable,
        key: &ExistingKey,
    ) -> Result<(), StoreError> {
        let statement = match table {
            KeyTable::Semaphores => {
                "UPDATE concurrency_semaphores SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = 'STEP' \
                    AND group_name = $4 AND key_digest = $5 AND waiters > 0"
            }
            KeyTable::Windows => {
                "UPDATE rate_limit_windows SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND limit_scope = 'STEP' \
                    AND group_name = $4 AND key_digest = $5 AND waiters > 0"
            }
        };
        sqlx::query(statement)
            .bind(key.tenant.as_uuid())
            .bind(key.namespace.as_uuid())
            .bind(key.pool.as_str())
            .bind(key.group.as_str())
            .bind(key.digest.as_slice())
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(())
    }
}

/// The stored projection of a step's gates, or `NULL` for a step that has none.
pub(super) fn gates_projection(step: &StepState) -> Result<Option<serde_json::Value>, StoreError> {
    let gates = step.gates();
    if gates.concurrency().is_none() && gates.rate().is_none() {
        return Ok(None);
    }
    let slot = |slot: Option<&GateSlot>| {
        slot.map(|slot| {
            let (phase, deadline) = match slot.phase() {
                GatePhase::Queued { deadline_us } => ("QUEUED", Some(deadline_us)),
                GatePhase::Held => ("HELD", None),
                GatePhase::Ended => ("ENDED", None),
                GatePhase::TimedOut => ("TIMED_OUT", None),
                GatePhase::Dequeued => ("DEQUEUED", None),
            };
            serde_json::json!({
                "phase": phase,
                "attempt": slot.attempt(),
                "pool": slot.gate().pool.as_str(),
                "group": slot.gate().group.as_str(),
                "key": slot.gate().key_label.as_str(),
                "deadlineUs": deadline,
            })
        })
    };
    let value = serde_json::json!({
        "concurrency": slot(gates.concurrency()),
        "rate": slot(gates.rate()),
    });
    if value.to_string().len() > MAX_PROJECTION_BYTES {
        return Err(corrupt("step gate projection"));
    }
    Ok(Some(value))
}

/// The gates of a step read back from their projection.
pub(super) fn restore_gates(value: &serde_json::Value) -> Result<Gates, StoreError> {
    let slot =
        |kind: GateKind, value: &serde_json::Value| -> Result<Option<GateSlot>, StoreError> {
            if value.is_null() {
                return Ok(None);
            }
            let text = |name: &str| {
                value
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| corrupt("step gate projection"))
            };
            let attempt = value
                .get("attempt")
                .and_then(serde_json::Value::as_u64)
                .and_then(|a| u32::try_from(a).ok())
                .ok_or_else(|| corrupt("step gate attempt"))?;
            let deadline = value.get("deadlineUs").and_then(serde_json::Value::as_i64);
            let phase = match (text("phase")?, deadline) {
                ("QUEUED", Some(deadline_us)) => GatePhase::Queued { deadline_us },
                ("HELD", None) => GatePhase::Held,
                ("ENDED", None) => GatePhase::Ended,
                ("TIMED_OUT", None) => GatePhase::TimedOut,
                ("DEQUEUED", None) => GatePhase::Dequeued,
                _ => return Err(corrupt("step gate phase")),
            };
            let gate = GateRef {
                pool: GatePool::parse(text("pool")?).ok_or_else(|| corrupt("step gate pool"))?,
                group: sagat_domain::workflow::ResolvedGroup::parse(text("group")?)
                    .map_err(|_| corrupt("step gate group"))?,
                key_label: KeyLabel::parse(text("key")?).map_err(|_| corrupt("step gate key"))?,
            };
            Ok(Some(GateSlot::restore(kind, gate, phase, attempt)))
        };
    Ok(Gates::restore(
        slot(
            GateKind::Concurrency,
            value.get("concurrency").unwrap_or(&serde_json::Value::Null),
        )?,
        slot(
            GateKind::RateLimit,
            value.get("rate").unwrap_or(&serde_json::Value::Null),
        )?,
    ))
}

/// The `waiting_kind` of a step that waits for its gates: the first gate it still waits for, rate first (the
/// precedence of FR-022-052, data-model "Steps and their gates"). A step that waits with no wait journaled is refused:
/// a commit never leaves a gated step undecided (invariant 11).
pub(super) fn gate_waiting_kind(step: &StepState) -> Result<&'static str, StoreError> {
    match step.gates().waiting().next() {
        Some((GateKind::RateLimit, _)) => Ok("RATE_LIMIT"),
        Some((GateKind::Concurrency, _)) => Ok("CONCURRENCY"),
        None => Err(StoreError::Internal {
            detail: "a gated step would be written waiting without a decision of its gates".into(),
        }),
    }
}

fn key_refusal(error: KeyedDigestError) -> StoreError {
    match error {
        // the application bounds a key before it asks; a longer one here is a defect, not the author's
        KeyedDigestError::KeyTooLong => StoreError::Internal {
            detail: "a flow-control key reached the gate unbounded".into(),
        },
        KeyedDigestError::UnknownVersion
        | KeyedDigestError::MalformedKey
        | KeyedDigestError::DuplicateVersion => StoreError::Unavailable {
            detail: "this process lacks the flow-control digest key version in force".into(),
        },
    }
}

/// The home shard of a key (ADR-0089 decision 3): a stable hash of what names its row, modulo the 64 shards.
fn key_shard(group: &sagat_domain::workflow::ResolvedGroup, digest: &[u8; 32]) -> i16 {
    let mut hash = Sha256::new();
    hash.update(b"sagat.flow-control.key-shard.v1\0");
    hash.update(GatePool::Ordinary.as_str().as_bytes());
    hash.update(b"\0STEP\0");
    hash.update(group.as_str().as_bytes());
    hash.update(b"\0");
    hash.update(digest);
    i16::from(hash.finalize()[0] & 63)
}

fn period_ms(gate: &Pending) -> i64 {
    gate.per.map_or(0, |per| {
        i64::try_from(per.duration().as_millis()).unwrap_or(i64::MAX)
    })
}

fn unix_micros(at: OffsetDateTime) -> Result<i64, StoreError> {
    i64::try_from(at.unix_timestamp_nanos() / 1_000).map_err(|_| corrupt("instant"))
}

/// What one gate decision or return of a committed transaction counts as (G022-001): recorded once the transaction
/// commits, in the closed vocabulary of `sagat_telemetry::flow_control`, never with a tenant, a group or a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GateOutcome {
    /// Granted at once by the readiness gate.
    Granted,
    Queued,
    /// Granted by the grant routine.
    GrantedFromQueue,
    TimedOut,
    Dequeued,
    /// A permit released or an unleased slot returned.
    GivenBack,
}

impl GateOutcome {
    pub(super) const fn of(change: GateChange) -> Self {
        match change {
            GateChange::ConcurrencyAcquired => Self::Granted,
            GateChange::RateGranted { from_queue, .. } => {
                if from_queue {
                    Self::GrantedFromQueue
                } else {
                    Self::Granted
                }
            }
            GateChange::ConcurrencyQueued { .. } | GateChange::RateQueued { .. } => Self::Queued,
            GateChange::ConcurrencyWaitTimedOut | GateChange::RateWaitTimedOut => Self::TimedOut,
            GateChange::ConcurrencyDequeued | GateChange::RateDequeued => Self::Dequeued,
            GateChange::ConcurrencyReleased | GateChange::RateReturned => Self::GivenBack,
        }
    }
}

/// Records the step-scope decisions of a committed transaction (G022-001; FR-022-033, FR-022-049).
pub(super) fn record_gate_outcomes(outcomes: &[(GateKind, GateOutcome)]) {
    use sagat_telemetry::flow_control::{
        ConcurrencyOutcome, FlowControlScope, RateLimitOutcome, record_concurrency,
        record_rate_limit,
    };
    for (kind, outcome) in outcomes {
        match kind {
            GateKind::Concurrency => {
                let label = match outcome {
                    GateOutcome::Granted => Some(ConcurrencyOutcome::Acquired),
                    GateOutcome::Queued => Some(ConcurrencyOutcome::Queued),
                    GateOutcome::GrantedFromQueue => Some(ConcurrencyOutcome::Granted),
                    GateOutcome::TimedOut => Some(ConcurrencyOutcome::TimedOut),
                    GateOutcome::Dequeued => Some(ConcurrencyOutcome::Dequeued),
                    // a release is not an acquisition: the vocabulary has no outcome for it
                    GateOutcome::GivenBack => None,
                };
                if let Some(label) = label {
                    record_concurrency(FlowControlScope::Step, label);
                }
            }
            GateKind::RateLimit => record_rate_limit(
                FlowControlScope::Step,
                match outcome {
                    GateOutcome::Granted => RateLimitOutcome::Granted,
                    GateOutcome::Queued => RateLimitOutcome::Queued,
                    GateOutcome::GrantedFromQueue => RateLimitOutcome::GrantedFromQueue,
                    GateOutcome::TimedOut => RateLimitOutcome::TimedOut,
                    GateOutcome::Dequeued => RateLimitOutcome::Dequeued,
                    GateOutcome::GivenBack => RateLimitOutcome::SlotReturned,
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GateOutcome, record_gate_outcomes};
    use sagat_domain::workflow::GateKind;

    /// G022-001: what a committed transaction decided is counted once per gate, step scope, in the closed vocabulary;
    /// a released permit is not an acquisition and is not counted as one.
    #[test]
    fn each_committed_gate_outcome_is_counted_in_the_closed_vocabulary() {
        let (telemetry, recorder) =
            sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
        let all = [
            GateOutcome::Granted,
            GateOutcome::Queued,
            GateOutcome::GrantedFromQueue,
            GateOutcome::TimedOut,
            GateOutcome::Dequeued,
            GateOutcome::GivenBack,
        ];
        let outcomes: Vec<_> = [GateKind::Concurrency, GateKind::RateLimit]
            .into_iter()
            .flat_map(|kind| all.map(|outcome| (kind, outcome)))
            .collect();
        metrics::with_local_recorder(&recorder, || record_gate_outcomes(&outcomes));
        let rendered = telemetry.render();
        let counted = |family: &str, outcome: &str| {
            rendered.lines().any(|line| {
                line.starts_with(family)
                    && line.contains("scope=\"step\"")
                    && line.contains(&format!("outcome=\"{outcome}\""))
                    && line.ends_with(" 1")
            })
        };
        for outcome in ["acquired", "queued", "granted", "timed_out", "dequeued"] {
            assert!(
                counted("concurrency_acquisitions_total", outcome),
                "{outcome}: {rendered}"
            );
        }
        for outcome in [
            "granted",
            "queued",
            "granted_from_queue",
            "timed_out",
            "dequeued",
            "slot_returned",
        ] {
            assert!(
                counted("rate_limit_decisions_total", outcome),
                "{outcome}: {rendered}"
            );
        }
        let concurrency_series = rendered
            .lines()
            .filter(|line| line.starts_with("concurrency_acquisitions_total{"))
            .count();
        assert_eq!(
            concurrency_series, 5,
            "a release is not counted: {rendered}"
        );
        assert!(!rendered.contains("tenant"), "no tenant label: {rendered}");
    }
}
