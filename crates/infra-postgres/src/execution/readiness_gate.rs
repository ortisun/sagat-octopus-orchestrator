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
//! A gate admits at once only when its key has no eligible waiter and its counted holders or grants are below the
//! acquirer's own limit: strict admission order (decision D022-073, narrowing the reservation rule of D022-021 and
//! ADR-0089 decision 4), so a new arrival is never granted before a queued waiter that could be. Eligible: not past its
//! deadline, of a live run that is not paused, and below its own window's count by the lower of its filed limit and the
//! namespace's current ceiling. The grant routine (T022-097) serves the queue, in admission order.
//!
//! The rows of a decision are written here, under the keys' locks, before `persist` writes the journal and the
//! projections; `persist` gives back what the planner releases ([`PgRunTx::persist_gate_returns`]) before it touches
//! a usage row, so a semaphore or a window is always locked before the usage rows of slot 8b (D022-038).

use sagat_application::flow_control::{FlowControlRefusal, StepGateKeys};
use sagat_application::keyed_digest::{KeyPurpose, KeyedDigestError};
use sagat_domain::execution::StartCause;
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
/// The most grants a window may hold for a decision to reclaim it (decision D022-066 item 1): the reclaim deletes them in
/// the deciding transaction, so its work is bounded; a window of more waits for the purge (gap G022-018).
const RECLAIM_GRANTS: i32 = 64;

/// One gate of one step or of the run's start, as this transaction decides it: the one shape both levels are decided
/// in ([`PgRunTx::decide_filed`], G022-012).
struct Pending {
    /// The step, or `None` for the run's start (T022-022, T022-072).
    node: Option<NodeId>,
    /// The attempt the decision is for: the step's attempt plus one; 0 for the run's start.
    attempt: u32,
    kind: GateKind,
    group: sagat_domain::workflow::ResolvedGroup,
    /// The effective limit: the lower of the declaration and the namespace's ceiling (FR-022-034, FR-022-047).
    limit: u32,
    /// The declaration the run's version pins, before any ceiling: what bounds the waiter, under the namespace's current
    /// ceiling, at every later decision (decisions D022-076 item 7, D022-078 item 1).
    pinned: u32,
    /// The window of a rate limit, `None` for a concurrency permit.
    per: Option<sagat_domain::workflow::RatePeriod>,
    /// The effective queue timeout.
    timeout: std::time::Duration,
    /// Its key, once filed.
    filed: Option<FiledKey>,
}

impl Pending {
    const fn level(&self) -> GateLevel {
        if self.node.is_some() {
            GateLevel::Step
        } else {
            GateLevel::Run
        }
    }

    /// The `node` column of its rows: the node, or `''` for the run's own row.
    fn node_column(&self) -> &str {
        self.node.as_ref().map_or("", NodeId::as_str)
    }

    fn target(&self) -> GateTarget {
        self.node
            .as_ref()
            .map_or(GateTarget::Run, |node| GateTarget::Step {
                node: node.clone(),
                attempt: self.attempt,
            })
    }
}

/// What the start of a run adds to the shared decision ([`PgRunTx::decide_filed`]): a queue it may not join, and the
/// ceilings a run waiter meets (D022-049, D022-052, D022-053).
#[derive(Clone, Copy)]
struct StartRules {
    cause: StartCause,
    ceilings: sagat_domain::workflow::FlowControlCeilings,
}

/// A key row and the window its grants were counted for: (rate, level, group, digest, period in milliseconds).
type CountKey = (bool, GateLevel, String, [u8; 32], i64);

fn count_key(gate: &Pending, filed: &FiledKey) -> CountKey {
    (
        gate.kind == GateKind::RateLimit,
        gate.level(),
        gate.group.as_str().to_owned(),
        filed.digest,
        period_ms(gate),
    )
}

/// One gate a transaction gives back, with the key row it is under.
struct GateReturn {
    table: KeyTable,
    change: GateChange,
    /// The step, or `None` for the run's start (T022-022).
    node: Option<NodeId>,
    attempt: u32,
    key: ExistingKey,
}

impl GateReturn {
    /// The `node` column of its row: the node, or `''` for a run's own row.
    fn node_column(&self) -> &str {
        self.node.as_ref().map_or("", NodeId::as_str)
    }
}

/// The counts of one locked key, kept as this transaction adds rows under it.
#[derive(Clone, Copy, Default)]
struct KeyCount {
    /// Holders of a semaphore; grants that count of a window (for the period they were read for).
    taken: i64,
    waiters: i64,
    /// Whether the key has an eligible waiter (decision D022-073): read only when it has waiters and room
    /// ([`PgRunTx::eligible_waiters`]), `None` until then.
    eligible: Option<bool>,
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
        // the namespace's ceilings bound what a declaration asks (G1199, T022-077; FR-022-047, FR-022-054)
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
            let flow = node.flow_control().clone();
            for gate in self.gates_of(
                &ring,
                versions,
                &ceilings,
                &flow,
                Some(&step.node),
                step.attempt,
                step.concurrency.as_deref(),
                step.rate.as_deref(),
            )? {
                let (kind, key, gate) = gate;
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
                pending.push((index, gate));
            }
        }
        let refused = self
            .refused_by_ceilings(&pending, &semaphores, &windows, &ceilings, versions)
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
        // a pause or a resume makes the keys of the run's queued steps due after this round: they are locked here, in
        // the same sorted pass (correctness review M1 of 2026-11-20)
        let rearmed = self.rearmed_keys(pending_events).await?;
        let returned = |table: KeyTable| -> Vec<ExistingKey> {
            returns
                .iter()
                .filter(|r| r.table == table)
                .map(|r| r.key.clone())
                .chain(
                    rearmed
                        .iter()
                        .filter(|(t, _)| *t == table)
                        .map(|(_, key)| key.clone()),
                )
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
        let decisions = self
            .decide_filed(pending, &filed_semaphores, &filed_windows, None, &ceilings)
            .await?;
        Ok(sagat_application::flow_control::StepGateAnswer {
            decisions,
            refused: Vec::new(),
        })
    }

    /// The gates `flow` declares at one level (a step's node, or the run's start when `node` is `None`), each with the
    /// key to file and its effective limit and timeout, rate first: the one place both levels turn a declaration into
    /// what is decided (G022-012).
    #[allow(clippy::too_many_arguments)]
    fn gates_of(
        &self,
        ring: &sagat_application::keyed_digest::KeyRing,
        versions: sagat_application::keyed_digest::KeyVersions,
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
        flow: &sagat_domain::workflow::FlowControl,
        node: Option<&NodeId>,
        attempt: u32,
        concurrency: Option<&[u8]>,
        rate: Option<&[u8]>,
    ) -> Result<Vec<(GateKind, KeyToFile, Pending)>, StoreError> {
        let (tenant, namespace) = (self.info.tenant_id, self.info.namespace_id);
        let level = if node.is_some() {
            GateLevel::Step
        } else {
            GateLevel::Run
        };
        let declared = [
            flow.rate_limit().map(|policy| {
                (
                    GateKind::RateLimit,
                    policy.group(),
                    ceilings.rate_limit(policy),
                    policy.limit(),
                    Some(policy.per()),
                    policy.queue_timeout(),
                    rate,
                )
            }),
            flow.concurrency().map(|policy| {
                (
                    GateKind::Concurrency,
                    policy.group(),
                    ceilings.concurrency_limit(policy),
                    policy.limit(),
                    None,
                    policy.queue_timeout(),
                    concurrency,
                )
            }),
        ];
        let mut gates = Vec::new();
        for (kind, group, limit, pinned, per, timeout, canonical) in declared.into_iter().flatten()
        {
            let canonical = canonical.ok_or_else(|| StoreError::Internal {
                detail: "a declared gate came without its key".into(),
            })?;
            let purpose = match kind {
                GateKind::Concurrency => KeyPurpose::Concurrency,
                GateKind::RateLimit => KeyPurpose::RateLimit,
            };
            let candidates = ring
                .candidates(versions, purpose, tenant, namespace, canonical)
                .map_err(key_refusal)?;
            let group = group.resolve(self.def.name(), node);
            let key = KeyToFile {
                tenant,
                namespace,
                pool: GatePool::Ordinary,
                level,
                shard: key_shard(level, &group, candidates.active.as_bytes()),
                group: group.clone(),
                candidates,
            };
            gates.push((
                kind,
                key,
                Pending {
                    node: node.cloned(),
                    attempt,
                    kind,
                    group,
                    limit,
                    pinned,
                    per,
                    timeout: ceilings.queue_timeout(timeout),
                    filed: None,
                },
            ));
        }
        Ok(gates)
    }

    /// Decides filed gates under their keys' locks, one target (a step's attempt, or the run's start) at a time, in
    /// target order: every gate of a target admits it (holders or counted grants plus the key's waiters below its own
    /// limit, the reservation rule of D022-021) and it is granted every one, or it waits for every one with the earlier
    /// of their deadlines (FR-022-044, D022-006). A start adds its own rules (`start`): a reset never waits, and a run
    /// waiter meets the namespace's `queued_runs` and the key's waiter ceiling, rate first (FR-022-052). The one decision
    /// of both levels and both gates (G022-012).
    async fn decide_filed(
        &mut self,
        pending: Vec<(usize, Pending)>,
        filed_semaphores: &[FiledKey],
        filed_windows: &[FiledKey],
        start: Option<StartRules>,
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
    ) -> Result<Vec<GateEvent>, StoreError> {
        let mut by_target: BTreeMap<(Option<NodeId>, u32), Vec<Pending>> = BTreeMap::new();
        for (index, mut gate) in pending {
            let filed = match gate.kind {
                GateKind::Concurrency => filed_semaphores,
                GateKind::RateLimit => filed_windows,
            };
            gate.filed = filed.iter().find(|key| key.index == index).cloned();
            by_target
                .entry((gate.node.clone(), gate.attempt))
                .or_default()
                .push(gate);
        }
        // the grant instant is the database clock after the keys' locks (D022-027)
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        let mut counts: BTreeMap<CountKey, KeyCount> = BTreeMap::new();
        let mut events = Vec::new();
        // a start that may not queue (a reset, a pending reset) is decided by the reservation rule, not by strict order:
        // it is refused only when its key's counted holders or grants plus the eligible waiters leave it no room
        // (FR-022-018, FR-022-041; decision D022-076 item 3)
        let may_queue = start.is_none_or(|rules| rules.cause.may_queue());
        for ((node, _), gates) in by_target {
            // the gates that do not admit the target now, in the order of `gates` (rate first): a full key, or one with an
            // eligible waiter, which is ahead of a new arrival (strict admission order, decision D022-073); and whether
            // each gate had room, so that a target queued only by that order makes its key due at once (item 4)
            let mut refusing = Vec::new();
            let mut room = Vec::with_capacity(gates.len());
            for gate in &gates {
                let count = self.key_count(&mut counts, gate, now).await?;
                let has_room = count.taken < i64::from(gate.limit);
                room.push(has_room);
                let refuses = if !has_room {
                    true
                } else if may_queue {
                    self.eligible_ahead(&mut counts, gate, now, ceilings)
                        .await?
                } else {
                    let reserved = self
                        .reserved_for_waiters(&counts, gate, now, ceilings)
                        .await?;
                    count.taken + reserved >= i64::from(gate.limit)
                };
                if refuses {
                    refusing.push(gate.kind);
                }
            }
            let admits = refusing.is_empty();
            if let (false, Some(rules)) = (admits, start) {
                self.start_may_wait(&gates, &counts, rules, &refusing)
                    .await?;
            }
            let deadline = gates.iter().map(|gate| gate.timeout).min().ok_or_else(|| {
                StoreError::Internal {
                    detail: "a gated target declares no gate".into(),
                }
            })?;
            let deadline = now
                .checked_add(
                    time::Duration::try_from(deadline).map_err(|_| corrupt("queue timeout"))?,
                )
                .ok_or_else(|| corrupt("queue deadline"))?;
            for (gate, has_room) in gates.iter().zip(room) {
                let filed = gate.filed.as_ref().ok_or_else(|| StoreError::Internal {
                    detail: "a gate's key was not filed".into(),
                })?;
                let change = if admits {
                    let waiting = counts
                        .get(&count_key(gate, filed))
                        .is_some_and(|count| count.waiters > 0);
                    self.grant(gate, filed, now, waiting).await?
                } else {
                    self.queue(gate, filed, now, deadline, has_room).await?
                };
                let outcomes = if node.is_some() {
                    &mut self.gate_outcomes
                } else {
                    &mut self.run_gate_outcomes
                };
                outcomes.push((gate.kind, GateOutcome::of(change)));
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
                let (rate, level, group, digest, _) = count_key(gate, filed);
                let mut moved = false;
                for ((r, l, g, d, _), entry) in &mut counts {
                    if *r == rate && *l == level && *g == group && *d == digest {
                        moved = true;
                        if admits {
                            entry.taken += 1;
                        } else {
                            entry.waiters += 1;
                            // a waiter its own limit leaves room for is eligible, for every window length of the key
                            if has_room {
                                entry.eligible = Some(true);
                            }
                        }
                    }
                }
                if !moved {
                    return Err(corrupt("flow-control key count"));
                }
                events.push(GateEvent {
                    target: gate.target(),
                    gate: GateRef {
                        pool: GatePool::Ordinary,
                        group: gate.group.clone(),
                        key_label: KeyLabel::of_digest(&filed.digest),
                    },
                    change,
                });
            }
        }
        Ok(events)
    }

    /// Whether a start that its gates do not admit now may wait (T022-022, T022-072): a reset never waits (018's plan,
    /// row 17) and is refused by the first gate in `refusing` (rate first, FR-022-052, D022-069:
    /// `RATE_LIMIT_SLOT_UNAVAILABLE`, else `CONCURRENCY_PERMIT_UNAVAILABLE`), counted `refused_reset` by each family the
    /// start declares; a run waiter meets the namespace's `queued_runs` (exact: every transaction that adds a run waiter holds
    /// the namespace's admission lock, D022-051) and each key's waiter ceiling, counted under the key's lock. Refused
    /// rate first (FR-022-052): `RATE_LIMIT_QUEUE_FULL` when the start declares a rate limit, else
    /// `CONCURRENCY_QUEUE_FULL` for the key and `FLOW_CONTROL_LIMIT` for the namespace (`contracts/errors.md`).
    async fn start_may_wait(
        &mut self,
        gates: &[Pending],
        counts: &BTreeMap<CountKey, KeyCount>,
        rules: StartRules,
        refusing: &[GateKind],
    ) -> Result<(), StoreError> {
        let declared = sagat_domain::execution::DeclaredGates {
            concurrency: gates.iter().any(|gate| gate.kind == GateKind::Concurrency),
            rate: gates.iter().any(|gate| gate.kind == GateKind::RateLimit),
        };
        if !rules.cause.may_queue() {
            return Err(reset_refused(declared, refusing));
        }
        let (tenant, namespace) = (self.info.tenant_id, self.info.namespace_id);
        // a queued run is an active run with a ceiling of its own (D1182): the namespace's, then each key's
        let queued_runs: i64 = sqlx::query_scalar(crate::flow_control::QUEUED_RUNS_SQL)
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        // rate first: the waiters of each key, under its lock (gates are rate first)
        for gate in gates {
            let filed = gate
                .filed
                .as_ref()
                .ok_or_else(|| corrupt("a gate's key was not filed"))?;
            let waiters = counts
                .get(&count_key(gate, filed))
                .map_or(0, |count| count.waiters);
            rules
                .ceilings
                .admits_waiter(
                    u64::try_from(queued_runs).unwrap_or(u64::MAX),
                    u64::try_from(waiters).unwrap_or(u64::MAX),
                    false,
                )
                .map_err(|limit| start_refused(limit, declared, Some(gate.kind), rules.cause))?;
        }
        Ok(())
    }

    /// The steps of this round a namespace ceiling refuses `FLOW_CONTROL_LIMIT` (G1199, T022-077): a ceiling of zero (the
    /// effective limit or queue timeout), or a new step key beyond the live-keys ceiling of its kind (FR-022-054;
    /// semaphores `concurrency_live_keys`, D022-043; windows `rate_limit_live_keys`). The keys that exist cost one
    /// statement a kind; only a round that would create a key counts the namespace's keys of that kind, under the versions
    /// in force only: a row a forced retirement left (ADR-0093 decision 5) is being replaced and drains on its own, so
    /// the ceiling is passed by at most those rows meanwhile. Not serialized between transactions: concurrent rounds pass
    /// it by at most the new keys the others create (G022-002, D022-047, O1173).
    async fn refused_by_ceilings(
        &mut self,
        pending: &[(usize, Pending)],
        semaphores: &[KeyToFile],
        windows: &[KeyToFile],
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
        versions: sagat_application::keyed_digest::KeyVersions,
    ) -> Result<Vec<(NodeId, sagat_domain::execution::FailureCode)>, StoreError> {
        let code = sagat_domain::execution::FailureCode::flow_control_limit;
        let mut refused: Vec<NodeId> = pending
            .iter()
            .filter(|(_, gate)| gate.limit == 0 || gate.timeout.is_zero())
            .filter_map(|(_, gate)| gate.node.clone())
            .collect();
        for (kind, table, keys) in [
            (GateKind::Concurrency, KeyTable::Semaphores, semaphores),
            (GateKind::RateLimit, KeyTable::Windows, windows),
        ] {
            if keys.is_empty() {
                continue;
            }
            let existing = self.existing_keys(table, GateLevel::Step, keys).await?;
            // a key the round would create, once each (two steps may share one), in the order the steps are decided
            let mut new_keys: Vec<(String, [u8; 32])> = Vec::new();
            let mut new_of_step: Vec<(NodeId, (String, [u8; 32]))> = Vec::new();
            for (index, gate) in pending {
                let Some(node) = &gate.node else { continue };
                if gate.kind != kind || refused.contains(node) || existing.contains(index) {
                    continue;
                }
                let key = &keys[*index];
                let id = (
                    key.group.as_str().to_owned(),
                    *key.candidates.active.as_bytes(),
                );
                if !new_keys.contains(&id) {
                    new_keys.push(id.clone());
                }
                new_of_step.push((node.clone(), id));
            }
            if new_keys.is_empty() {
                continue;
            }
            let mut live = self.live_keys(table, versions).await?;
            let ceiling = match table {
                KeyTable::Semaphores => ceilings.live_keys,
                KeyTable::Windows => ceilings.rate_live_keys,
            };
            // a window row the namespace's purge backlog keeps makes room for the keys of this round (D022-066 item 1)
            let lacking = u64::try_from(new_keys.len())
                .unwrap_or(u64::MAX)
                .saturating_add(live)
                .saturating_sub(ceiling);
            if table == KeyTable::Windows && lacking > 0 {
                let wanted = usize::try_from(lacking)
                    .unwrap_or(usize::MAX)
                    .min(new_keys.len());
                live = live.saturating_sub(self.reclaim_windows(wanted, versions, keys).await?);
            }
            let room = usize::try_from(ceiling.saturating_sub(live)).unwrap_or(usize::MAX);
            let admitted = &new_keys[..new_keys.len().min(room)];
            for (node, id) in new_of_step {
                if !admitted.contains(&id) && !refused.contains(&node) {
                    refused.push(node);
                }
            }
        }
        refused.sort();
        refused.dedup();
        Ok(refused.into_iter().map(|node| (node, code())).collect())
    }

    /// Which of `keys` (by position) already have a row of `level` in `table`, under either digest in force.
    async fn existing_keys(
        &mut self,
        table: KeyTable,
        level: GateLevel,
        keys: &[KeyToFile],
    ) -> Result<Vec<usize>, StoreError> {
        let (mut groups, mut digests, mut owners) = (Vec::new(), Vec::new(), Vec::new());
        for (index, key) in keys.iter().enumerate() {
            for digest in
                std::iter::once(&key.candidates.active).chain(key.candidates.retiring.as_ref())
            {
                groups.push(key.group.as_str().to_owned());
                digests.push(digest.as_bytes().to_vec());
                owners.push(i32::try_from(index).map_err(|_| corrupt("key index"))?);
            }
        }
        let statement = match table {
            KeyTable::Semaphores => {
                "SELECT DISTINCT k.owner FROM unnest($3::text[], $4::bytea[], $5::integer[]) AS k(g, d, owner) \
                  WHERE EXISTS (SELECT 1 FROM concurrency_semaphores s \
                                 WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.pool = 'ORDINARY' \
                                   AND s.permit_scope = $6 AND s.group_name = k.g AND s.key_digest = k.d)"
            }
            KeyTable::Windows => {
                "SELECT DISTINCT k.owner FROM unnest($3::text[], $4::bytea[], $5::integer[]) AS k(g, d, owner) \
                  WHERE EXISTS (SELECT 1 FROM rate_limit_windows s \
                                 WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.pool = 'ORDINARY' \
                                   AND s.limit_scope = $6 AND s.group_name = k.g AND s.key_digest = k.d)"
            }
        };
        let existing: Vec<i32> = sqlx::query_scalar(statement)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(&groups)
            .bind(&digests)
            .bind(&owners)
            .bind(level.as_str())
            .fetch_all(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(existing
            .into_iter()
            .filter_map(|owner| usize::try_from(owner).ok())
            .collect())
    }

    /// The namespace's key rows of `table` under the digest versions in force (ADR-0093 decision 5), both levels.
    async fn live_keys(
        &mut self,
        table: KeyTable,
        versions: sagat_application::keyed_digest::KeyVersions,
    ) -> Result<u64, StoreError> {
        let in_force: Vec<i16> = std::iter::once(versions.active)
            .chain(versions.retiring)
            .map(|v| i16::try_from(v).map_err(|_| corrupt("digest key version")))
            .collect::<Result<_, _>>()?;
        let statement = match table {
            KeyTable::Semaphores => {
                "SELECT count(*) FROM concurrency_semaphores WHERE tenant_id = $1 AND namespace_id = $2 \
                 AND key_version = ANY($3)"
            }
            // every window row, as the semaphores: a row whose grants aged out still counts until it goes, and a decision that
            // lacks room reclaims its own namespace's purgeable windows first (`reclaim_windows`), so no namespace waits
            // for another tenant's purge backlog and the rows stay bounded by the ceiling (decision D022-066 item 1)
            KeyTable::Windows => {
                "SELECT count(*) FROM rate_limit_windows WHERE tenant_id = $1 AND namespace_id = $2 \
                 AND key_version = ANY($3)"
            }
        };
        let live: i64 = sqlx::query_scalar(statement)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(&in_force)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(u64::try_from(live).unwrap_or(u64::MAX))
    }

    /// Reclaims, for a decision that lacks room under `rate_limit_live_keys`, at most `wanted` windows of this namespace
    /// that no waiter and only grants the purge may delete keep (decision D022-066 item 1; FR-022-056's boundary, the
    /// purge's own predicate), under the digest versions in force, each of at most [`RECLAIM_GRANTS`] grants and none of
    /// `own` (the keys the decision files, under either digest). Returns how many go at the commit: their grants are
    /// deleted here and `rate_limit_window_prune` deletes each window then. The windows are locked `SKIP LOCKED`, so the
    /// reclaim never waits and adds no lock order; their grants are read again by a second statement, whose snapshot is
    /// taken under those locks (a grant is written only under its window's lock), so a window granted since the first
    /// statement's snapshot is kept.
    async fn reclaim_windows(
        &mut self,
        wanted: usize,
        versions: sagat_application::keyed_digest::KeyVersions,
        own: &[KeyToFile],
    ) -> Result<u64, StoreError> {
        if wanted == 0 {
            return Ok(0);
        }
        let in_force: Vec<i16> = std::iter::once(versions.active)
            .chain(versions.retiring)
            .map(|v| i16::try_from(v).map_err(|_| corrupt("digest key version")))
            .collect::<Result<_, _>>()?;
        let (mut own_groups, mut own_digests) = (Vec::new(), Vec::new());
        for key in own {
            for digest in
                std::iter::once(&key.candidates.active).chain(key.candidates.retiring.as_ref())
            {
                own_groups.push(key.group.as_str().to_owned());
                own_digests.push(digest.as_bytes().to_vec());
            }
        }
        let locked: Vec<(String, String, String, Vec<u8>)> = sqlx::query_as(concat!(
            "SELECT w.pool, w.limit_scope, w.group_name, w.key_digest FROM rate_limit_windows w \
              WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.key_version = ANY($3) AND w.waiters = 0 \
                AND w.live_grants BETWEEN 1 AND $6 \
                AND NOT EXISTS (SELECT 1 FROM unnest($4::text[], $5::bytea[]) AS k(g, d) \
                                 WHERE k.g = w.group_name AND k.d = w.key_digest) \
                AND NOT EXISTS (SELECT 1 FROM rate_limit_grants g \
                                 WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                                   AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name \
                                   AND g.key_digest = w.key_digest AND NOT (",
            purgeable_grant!("g", "w", "clock_timestamp()"),
            ")) ORDER BY w.updated_at LIMIT $7 FOR UPDATE SKIP LOCKED"
        ))
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(&in_force)
        .bind(&own_groups)
        .bind(&own_digests)
        .bind(RECLAIM_GRANTS)
        .bind(i64::try_from(wanted).unwrap_or(i64::MAX))
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error)?;
        if locked.is_empty() {
            return Ok(0);
        }
        let (mut pools, mut scopes, mut groups, mut digests) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (pool, scope, group, digest) in locked {
            pools.push(pool);
            scopes.push(scope);
            groups.push(group);
            digests.push(digest);
        }
        let reclaimed: i64 = sqlx::query_scalar(concat!(
            "WITH v AS ( \
               SELECT w.tenant_id, w.namespace_id, w.pool, w.limit_scope, w.group_name, w.key_digest \
                 FROM unnest($3::text[], $4::text[], $5::text[], $6::bytea[]) AS k(pool, scope, g, d) \
                 JOIN rate_limit_windows w ON w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = k.pool \
                  AND w.limit_scope = k.scope AND w.group_name = k.g AND w.key_digest = k.d \
                WHERE w.waiters = 0 \
                  AND NOT EXISTS (SELECT 1 FROM rate_limit_grants g \
                                   WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id \
                                     AND g.pool = w.pool AND g.limit_scope = w.limit_scope \
                                     AND g.group_name = w.group_name AND g.key_digest = w.key_digest AND NOT (",
            purgeable_grant!("g", "w", "clock_timestamp()"),
            "))), \
             gone AS ( \
               DELETE FROM rate_limit_grants g USING v \
                WHERE g.tenant_id = v.tenant_id AND g.namespace_id = v.namespace_id AND g.pool = v.pool \
                  AND g.limit_scope = v.limit_scope AND g.group_name = v.group_name AND g.key_digest = v.key_digest \
               RETURNING 1) \
             SELECT count(*) FROM v"
        ))
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.namespace_id.as_uuid())
        .bind(&pools)
        .bind(&scopes)
        .bind(&groups)
        .bind(&digests)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(store_error)?;
        Ok(u64::try_from(reclaimed).unwrap_or(0))
    }

    /// Whether an eligible waiter of `gate`'s key is ahead of a new arrival that has room (decision D022-073): nothing to
    /// read for a key without waiters; else read once per key and period in this transaction, and set by the waiters this
    /// transaction queues.
    async fn eligible_ahead(
        &mut self,
        counts: &mut BTreeMap<CountKey, KeyCount>,
        gate: &Pending,
        now: OffsetDateTime,
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
    ) -> Result<bool, StoreError> {
        let filed = gate.filed.as_ref().ok_or_else(|| StoreError::Internal {
            detail: "a gate's key was not filed".into(),
        })?;
        let slot = count_key(gate, filed);
        let count = counts.get(&slot).copied().unwrap_or_default();
        if count.waiters == 0 {
            return Ok(false);
        }
        if let Some(eligible) = count.eligible {
            return Ok(eligible);
        }
        let eligible = self.eligible_waiters(gate, filed, now, ceilings, 1).await? > 0;
        if let Some(entry) = counts.get_mut(&slot) {
            entry.eligible = Some(eligible);
        }
        Ok(eligible)
    }

    /// How many eligible waiters of `gate`'s key a start that may not queue leaves room for (decision D022-076 item 3; the
    /// reservation rule of ADR-0089 decision 4): none to read for a key without waiters; else counted up to the start's
    /// own limit, which is all that can matter.
    async fn reserved_for_waiters(
        &mut self,
        counts: &BTreeMap<CountKey, KeyCount>,
        gate: &Pending,
        now: OffsetDateTime,
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
    ) -> Result<i64, StoreError> {
        let filed = gate.filed.as_ref().ok_or_else(|| StoreError::Internal {
            detail: "a gate's key was not filed".into(),
        })?;
        let waiters = counts
            .get(&count_key(gate, filed))
            .map_or(0, |count| count.waiters);
        if waiters == 0 {
            return Ok(0);
        }
        self.eligible_waiters(gate, filed, now, ceilings, i64::from(gate.limit))
            .await
    }

    /// How many eligible waiters the key of `gate` has, at most `up_to` (decision D022-073; ADR-0089 decision 4): not past
    /// their deadline at `now`, of a live run that is not paused, and below their own window's count by the lower of their
    /// declaration and the namespace's current ceiling (decisions D022-071, D022-074, D022-075 item 4, D022-076 item 7).
    /// Read only when the key has waiters and room for `gate`; the key row is locked. The waiters are read in queue order
    /// up to `up_to` eligible ones.
    async fn eligible_waiters(
        &mut self,
        gate: &Pending,
        filed: &FiledKey,
        now: OffsetDateTime,
        ceilings: &sagat_domain::workflow::FlowControlCeilings,
        up_to: i64,
    ) -> Result<i64, StoreError> {
        let statement = match gate.kind {
            GateKind::Concurrency => {
                concat!(
                    "SELECT count(*) FROM (SELECT 1 FROM concurrency_waiters w \
                                  JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                                 WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 \
                                   AND w.permit_scope = $6 AND w.group_name = $4 AND w.key_digest = $5 \
                                   AND ",
                    eligible_gate_waiter!("w", "r", "$7"),
                    " \
                                   AND least(coalesce(w.pinned_limit, w.declared_limit), $11::bigint) > \
                                       (SELECT s.holders FROM concurrency_semaphores s \
                                         WHERE s.tenant_id = w.tenant_id AND s.namespace_id = w.namespace_id \
                                           AND s.pool = w.pool AND s.permit_scope = w.permit_scope \
                                           AND s.group_name = w.group_name AND s.key_digest = w.key_digest) \
                                   AND $8::bigint >= 0 AND $9::bigint >= 0 AND $10::bigint >= 0 \
                                 ORDER BY w.enqueued_at, w.run_id, w.node LIMIT $12) eligible"
                )
            }
            // each waiter counts the key's grants over its own window (D022-009): the count is taken once for each window
            // length up to the longest the window row has seen, which is at least every waiter's
            GateKind::RateLimit => {
                concat!(
                    "WITH counted AS ( \
                   SELECT p.ms, (SELECT count(*) FROM rate_limit_grants g \
                                  WHERE g.tenant_id = k.tenant_id AND g.namespace_id = k.namespace_id \
                                    AND g.pool = k.pool AND g.limit_scope = k.limit_scope \
                                    AND g.group_name = k.group_name AND g.key_digest = k.key_digest \
                                    AND (g.leased_at IS NULL \
                                         OR g.leased_at > $7 - make_interval(secs => p.ms / 1000.0))) AS n \
                     FROM (SELECT 1000 AS ms UNION ALL SELECT 60000 UNION ALL SELECT 3600000) p, rate_limit_windows k \
                    WHERE k.tenant_id = $1 AND k.namespace_id = $2 AND k.pool = $3 AND k.limit_scope = $6 \
                      AND k.group_name = $4 AND k.key_digest = $5 AND p.ms <= k.max_period_ms) \
                 SELECT count(*) FROM (SELECT 1 FROM rate_limit_waiters w \
                                  JOIN runs r ON r.tenant_id = w.tenant_id AND r.id = w.run_id \
                                  LEFT JOIN counted c ON c.ms = w.period_ms \
                                 WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 \
                                   AND w.limit_scope = $6 AND w.group_name = $4 AND w.key_digest = $5 \
                                   AND ",
                    eligible_gate_waiter!("w", "r", "$7"),
                    " \
                                   AND least(coalesce(w.pinned_limit, w.declared_limit), \
                                             CASE w.period_ms WHEN 1000 THEN $8::bigint \
                                             WHEN 60000 THEN $9::bigint WHEN 3600000 THEN $10::bigint \
                                             ELSE w.declared_limit END) > coalesce(c.n, 0) \
                                   AND $11::bigint >= 0 \
                                 ORDER BY w.enqueued_at, w.run_id, w.node LIMIT $12) eligible"
                )
            }
        };
        sqlx::query_scalar(statement)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(GatePool::Ordinary.as_str())
            .bind(gate.group.as_str())
            .bind(filed.digest.as_slice())
            .bind(gate.level().as_str())
            .bind(now)
            .bind(super::gate_queue::ceiling_of(
                Some(ceilings),
                sagat_domain::workflow::RatePeriod::Second,
            ))
            .bind(super::gate_queue::ceiling_of(
                Some(ceilings),
                sagat_domain::workflow::RatePeriod::Minute,
            ))
            .bind(super::gate_queue::ceiling_of(
                Some(ceilings),
                sagat_domain::workflow::RatePeriod::Hour,
            ))
            .bind(super::gate_queue::concurrency_ceiling(Some(ceilings)))
            .bind(up_to)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)
    }

    /// The holders (or the grants that count for `gate`'s window) and the waiters of a key, read once per key and
    /// period in this transaction, under its lock.
    async fn key_count(
        &mut self,
        counts: &mut BTreeMap<CountKey, KeyCount>,
        gate: &Pending,
        now: OffsetDateTime,
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
                 WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = $7 \
                   AND group_name = $4 AND key_digest = $5 AND $6::bigint >= 0 AND $8::timestamptz IS NOT NULL",
            ),
            // a step's grant counts while it is unleased and for its window after its lease; a run's for its window after
            // its grant, which is its lease instant (data-model, Rate limits)
            GateKind::RateLimit => sqlx::query_as::<_, (i64, i64)>(
                "SELECT (SELECT count(*) FROM rate_limit_grants g \
                          WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id AND g.pool = w.pool \
                            AND g.limit_scope = w.limit_scope AND g.group_name = w.group_name \
                            AND g.key_digest = w.key_digest \
                            AND (g.leased_at IS NULL \
                                 OR g.leased_at > $8 - make_interval(secs => $6::bigint / 1000.0))), \
                        w.waiters::bigint \
                   FROM rate_limit_windows w \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 AND w.limit_scope = $7 \
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
            .bind(gate.level().as_str())
            .bind(now)
            .fetch_one(&mut *self.tx)
            .await
            .map_err(store_error)?;
        let count = KeyCount {
            taken,
            waiters,
            eligible: None,
        };
        counts.insert(slot, count);
        Ok(count)
    }

    async fn grant(
        &mut self,
        gate: &Pending,
        filed: &FiledKey,
        now: OffsetDateTime,
        waiting: bool,
    ) -> Result<GateChange, StoreError> {
        let attempt_column = i32::try_from(gate.attempt).map_err(|_| corrupt("attempt"))?;
        let limit = i32::try_from(gate.limit).map_err(|_| corrupt("declared limit"))?;
        match (gate.kind, gate.per) {
            (GateKind::Concurrency, _) => {
                sqlx::query(
                    "INSERT INTO concurrency_holders (tenant_id, namespace_id, run_id, holder_scope, node, attempt, pool, \
                     group_name, key_digest, declared_limit, acquired_at) \
                     VALUES ($1, $2, $3, $11, $4, $5, $6, $7, $8, $9, $10)",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(gate.node_column())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(now)
                .bind(gate.level().as_str())
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                Ok(GateChange::ConcurrencyAcquired)
            }
            (GateKind::RateLimit, Some(per)) => {
                // a run's slot counts from its grant, which is its lease instant; a step's from its attempt's lease, which
                // the lease pass records (data-model, `rate_limit_grants`; T022-072, T022-074)
                sqlx::query(
                    "INSERT INTO rate_limit_grants (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                     group_name, key_digest, declared_limit, period_ms, granted_at, leased_at, shard) \
                     VALUES ($1, $2, $3, $4, $5, $6, $12, $7, $8, $9, $10, $11, \
                             CASE WHEN $12 = 'RUN' THEN $11 END, run_shard($3))",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(gate.node_column())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(i32::try_from(period_ms(gate)).map_err(|_| corrupt("period"))?)
                .bind(now)
                .bind(gate.level().as_str())
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                // a key with waiters that this grant fills must wake when the grant may stop counting (correctness
                // review, Major 4, of 2026-11-20): only then, so the uncontended path costs nothing more
                if waiting {
                    window_due_after_grant(
                        &mut self.tx,
                        (
                            self.info.tenant_id,
                            self.info.namespace_id,
                            gate.level(),
                            &gate.group,
                        ),
                        &filed.digest,
                        now,
                    )
                    .await?;
                }
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
        gate: &Pending,
        filed: &FiledKey,
        now: OffsetDateTime,
        deadline: OffsetDateTime,
        has_room: bool,
    ) -> Result<GateChange, StoreError> {
        let attempt_column = i32::try_from(gate.attempt).map_err(|_| corrupt("attempt"))?;
        let limit = i32::try_from(gate.limit).map_err(|_| corrupt("declared limit"))?;
        let pinned = i32::try_from(gate.pinned).map_err(|_| corrupt("declared limit"))?;
        let deadline_us = unix_micros(deadline)?;
        let level = gate.level();
        match (gate.kind, gate.per) {
            (GateKind::Concurrency, _) => {
                self.key_due(
                    KeyTable::Semaphores,
                    level,
                    &gate.group,
                    &filed.digest,
                    has_room,
                    0,
                )
                .await?;
                sqlx::query(
                    "INSERT INTO concurrency_waiters (tenant_id, namespace_id, run_id, node, permit_scope, attempt, pool, \
                     group_name, key_digest, declared_limit, enqueued_at, deadline_at, shard, pinned_limit) \
                     VALUES ($1, $2, $3, $4, $12, $5, $6, $7, $8, $9, $10, $11, run_shard($3), $13)",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(gate.node_column())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(now)
                .bind(deadline)
                .bind(level.as_str())
                .bind(pinned)
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?;
                Ok(GateChange::ConcurrencyQueued {
                    queue_deadline_us: deadline_us,
                })
            }
            (GateKind::RateLimit, Some(per)) => {
                self.key_due(
                    KeyTable::Windows,
                    level,
                    &gate.group,
                    &filed.digest,
                    has_room,
                    period_ms(gate),
                )
                .await?;
                sqlx::query(
                    "INSERT INTO rate_limit_waiters (tenant_id, namespace_id, run_id, node, attempt, pool, limit_scope, \
                     group_name, key_digest, declared_limit, period_ms, enqueued_at, deadline_at, shard, pinned_limit) \
                     VALUES ($1, $2, $3, $4, $5, $6, $13, $7, $8, $9, $10, $11, $12, run_shard($3), $14)",
                )
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.namespace_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(gate.node_column())
                .bind(attempt_column)
                .bind(GatePool::Ordinary.as_str())
                .bind(gate.group.as_str())
                .bind(filed.digest.as_slice())
                .bind(limit)
                .bind(i32::try_from(period_ms(gate)).map_err(|_| corrupt("period"))?)
                .bind(now)
                .bind(deadline)
                .bind(level.as_str())
                .bind(pinned)
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
        let rearmed = self.rearmed_keys(events).await?;
        if returns.is_empty() && rearmed.is_empty() {
            return Ok(());
        }
        // the returns and the keys a pause or a resume makes due, in one sorted pass, before any of them is written
        // (D022-038; correctness review M1 of 2026-11-20)
        for table in [KeyTable::Semaphores, KeyTable::Windows] {
            let rows: Vec<ExistingKey> = returns
                .iter()
                .filter(|r| r.table == table)
                .map(|r| r.key.clone())
                .chain(
                    rearmed
                        .iter()
                        .filter(|(t, _)| *t == table)
                        .map(|(_, key)| key.clone()),
                )
                .collect();
            if !rows.is_empty() {
                lock_existing(&mut self.tx, table, &rows).await?;
            }
        }
        self.apply_returns(returns).await?;
        if !rearmed.is_empty() {
            self.rearm_queued_keys().await?;
        }
        Ok(())
    }

    /// The key rows of the run's queued steps when `events` pause or resume a run of a gated definition: what
    /// [`Self::rearm_queued_keys`] makes due, to be locked with the transaction's other keys in the one order.
    async fn rearmed_keys(
        &mut self,
        events: &[RunEvent],
    ) -> Result<Vec<(KeyTable, ExistingKey)>, StoreError> {
        let pausing = events
            .iter()
            .any(|event| matches!(event, RunEvent::RunPaused | RunEvent::RunResumed));
        if !pausing
            || (self.def.gated_nodes().next().is_none() && !self.def.run_flow_control().is_gated())
        {
            return Ok(Vec::new());
        }
        // a queued run's start is skipped while it is paused, like a queued step (ADR-0089 decision 4; T022-022)
        let rows: Vec<(String, String, String, String, Vec<u8>)> = sqlx::query_as(
            "SELECT 'S', permit_scope, pool, group_name, key_digest FROM concurrency_waiters \
              WHERE tenant_id = $1 AND run_id = $2 \
             UNION \
             SELECT 'W', limit_scope, pool, group_name, key_digest FROM rate_limit_waiters \
              WHERE tenant_id = $1 AND run_id = $2",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .fetch_all(&mut *self.tx)
        .await
        .map_err(store_error)?;
        rows.into_iter()
            .map(|(table, scope, pool, group, digest)| {
                Ok((
                    if table == "S" {
                        KeyTable::Semaphores
                    } else {
                        KeyTable::Windows
                    },
                    ExistingKey {
                        tenant: self.info.tenant_id,
                        namespace: self.info.namespace_id,
                        pool: GatePool::parse(&pool).ok_or_else(|| corrupt("waiter pool"))?,
                        level: if scope == "RUN" {
                            GateLevel::Run
                        } else {
                            GateLevel::Step
                        },
                        group: sagat_domain::workflow::ResolvedGroup::parse(&group)
                            .map_err(|_| corrupt("waiter group"))?,
                        digest: digest.try_into().map_err(|_| corrupt("waiter key"))?,
                    },
                ))
            })
            .collect()
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
            // a step's rows name its node and attempt; the run's own rows (its start's gates, T022-022) name neither
            let (node, attempt, level) = match &gate.target {
                GateTarget::Step { node, attempt } => {
                    (Some(node.clone()), *attempt, GateLevel::Step)
                }
                GateTarget::Run => (None, 0, GateLevel::Run),
            };
            let (table, read) = match gate.change {
                GateChange::ConcurrencyReleased => (
                    KeyTable::Semaphores,
                    "SELECT key_digest FROM concurrency_holders WHERE tenant_id = $1 AND run_id = $2 \
                     AND holder_scope = $5 AND node = $3 AND attempt = $4",
                ),
                GateChange::ConcurrencyDequeued | GateChange::ConcurrencyWaitTimedOut => (
                    KeyTable::Semaphores,
                    "SELECT key_digest FROM concurrency_waiters WHERE tenant_id = $1 AND run_id = $2 \
                     AND permit_scope = $5 AND node = $3 AND attempt = $4",
                ),
                // the domain gives back only a slot of an attempt that is not leased; a lease instant on its row can only be a
                // withdrawn lease's that this commit clears after the returns (decision D022-138, ADR-0096 decision 6)
                GateChange::RateReturned => (
                    KeyTable::Windows,
                    "SELECT key_digest FROM rate_limit_grants WHERE tenant_id = $1 AND run_id = $2 \
                     AND limit_scope = $5 AND node = $3 AND attempt = $4",
                ),
                GateChange::RateDequeued | GateChange::RateWaitTimedOut => (
                    KeyTable::Windows,
                    "SELECT key_digest FROM rate_limit_waiters WHERE tenant_id = $1 AND run_id = $2 \
                     AND limit_scope = $5 AND node = $3 AND attempt = $4",
                ),
                // decisions wrote their rows under the keys' locks
                GateChange::ConcurrencyQueued { .. }
                | GateChange::ConcurrencyAcquired
                | GateChange::RateQueued { .. }
                | GateChange::RateGranted { .. } => continue,
            };
            let identity = (node.clone(), attempt, gate.change.name());
            if self.gate_returns_applied.contains(&identity) {
                continue;
            }
            let attempt_column = i32::try_from(attempt).map_err(|_| corrupt("attempt"))?;
            let digest: Option<Vec<u8>> = sqlx::query_scalar(read)
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(node.as_ref().map_or("", NodeId::as_str))
                .bind(attempt_column)
                .bind(level.as_str())
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(store_error)?;
            let digest: [u8; 32] = digest
                .and_then(|d| d.try_into().ok())
                .ok_or_else(|| corrupt("the row a flow-control event gives back"))?;
            returns.push(GateReturn {
                table,
                change: gate.change,
                node,
                attempt,
                key: ExistingKey {
                    tenant: self.info.tenant_id,
                    namespace: self.info.namespace_id,
                    pool: gate.gate.pool,
                    level,
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
                    "DELETE FROM concurrency_holders WHERE tenant_id = $1 AND run_id = $2 AND holder_scope = $5 \
                     AND node = $3 AND attempt = $4"
                }
                GateChange::ConcurrencyDequeued | GateChange::ConcurrencyWaitTimedOut => {
                    "DELETE FROM concurrency_waiters WHERE tenant_id = $1 AND run_id = $2 AND permit_scope = $5 \
                     AND node = $3 AND attempt = $4"
                }
                GateChange::RateReturned => {
                    "DELETE FROM rate_limit_grants WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = $5 \
                     AND node = $3 AND attempt = $4"
                }
                GateChange::RateDequeued | GateChange::RateWaitTimedOut => {
                    "DELETE FROM rate_limit_waiters WHERE tenant_id = $1 AND run_id = $2 AND limit_scope = $5 \
                     AND node = $3 AND attempt = $4"
                }
                _ => return Err(corrupt("flow-control return")),
            };
            let deleted = sqlx::query(delete)
                .bind(self.info.tenant_id.as_uuid())
                .bind(self.info.run_id.as_uuid())
                .bind(r.node_column())
                .bind(i32::try_from(r.attempt).map_err(|_| corrupt("attempt"))?)
                .bind(r.key.level.as_str())
                .execute(&mut *self.tx)
                .await
                .map_err(store_error)?
                .rows_affected();
            if deleted != 1 {
                return Err(corrupt("the row a flow-control event gives back"));
            }
            // a permit released, a slot returned unleased, a waiter gone: each may let a waiter of the key through
            self.key_is_eligible(r.table, &r.key).await?;
            let outcomes = if r.node.is_some() {
                &mut self.gate_outcomes
            } else {
                &mut self.run_gate_outcomes
            };
            outcomes.push((r.change.kind(), GateOutcome::of(r.change)));
            self.gate_returns_applied
                .insert((r.node, r.attempt, r.change.name()));
        }
        Ok(())
    }
}

impl PgRunTx {
    /// Decides the gates of a new run's start in its admission transaction (T10 and transaction T19 of the plan, T022-022,
    /// T022-072; FR-022-018, FR-022-021, FR-022-041, FR-022-044): after the run's row (slot 6 of ADR-0050's merged order)
    /// the start's semaphore (slot 7) and then its window are filed and locked, then counted, by the one decision of
    /// both levels ([`Self::decide_filed`], G022-012): the run is granted every gate it declares when each key's holders
    /// or counted grants plus its waiters are below the acquirer's limit (a new arrival never takes a permit or a slot
    /// ahead of a queued run, D022-021), else it waits in every key's queue until the earliest of its effective queue
    /// timeouts (D022-006). A run's slot counts from its grant (its lease instant). Nothing is written when a ceiling
    /// refuses: a key beyond the namespace's live keys of its kind or a zero effective limit or timeout
    /// (`FLOW_CONTROL_LIMIT`), a full namespace (`queued_runs`) or a full key, rate first (`RATE_LIMIT_QUEUE_FULL` for a
    /// start that declares a rate limit, else `FLOW_CONTROL_LIMIT` and `CONCURRENCY_QUEUE_FULL`). The live keys are read
    /// without a namespace lock: admissions that each create a key at the same moment can pass them by at most the keys
    /// the others create (G022-002, OD-2026-11-19-4, OD-2026-11-20-3). `queued_runs` is exact: every admission holds the
    /// namespace's active-run quota lock (`ensure_room_for_run`) from before its runs row to its commit, and the count is a
    /// fresh statement after it (READ COMMITTED), so the admissions that could queue in a namespace are decided one after
    /// the other (D022-051, G022-011).
    ///
    /// The same decision is taken later for a delayed start at its release ([`StartCause::Release`]: the namespace's
    /// admission lock is taken first, as an admission holds it, so `queued_runs` stays exact; a ceiling's refusal is
    /// deferred by the release pass, not counted here) and for a reset right after `run.reset` ([`StartCause::Reset`]: a
    /// fresh permit and slot, or a refusal rate first, `RATE_LIMIT_SLOT_UNAVAILABLE` then
    /// `CONCURRENCY_PERMIT_UNAVAILABLE`, counted `refused_reset` by each family the start declares; never a wait)
    /// (D022-052, D022-053, D022-069).
    ///
    /// # Errors
    /// The refusals above; `BINARY_CAPABILITY_MISSING` without a digest key ring, and for a definition with a `forEach`,
    /// `while` or `repeatUntil` node (G1197) whatever asks; [`StoreError::Unavailable`] when no digest key version is
    /// active.
    pub(super) async fn decide_start_gates(
        &mut self,
        keys: &sagat_application::flow_control::RunGateKeys,
    ) -> Result<Vec<GateEvent>, StoreError> {
        let Some(ring) = self.flow_control_keys.clone() else {
            // an admission and a reset (whose run is created by `PgRunStore::create`) reach this only past the admission
            // point, which refuses a gate its process does not serve (`flow_control_admission`): reaching it without the
            // ring is a defect, not a refusal, so the admission point's guard is proved on its own (the mutation of
            // 2026-11-20 that dropped it survived behind this one). A release has no admission point: this is its guard.
            if keys.cause != StartCause::Release {
                return Err(StoreError::Internal {
                    detail: "an admission reached a start gate its process does not serve".into(),
                });
            }
            return Err(StoreError::FlowControl(
                FlowControlRefusal::BinaryCapabilityMissing,
            ));
        };
        let flow = self.def.run_flow_control().clone();
        // fail closed for every cause, a reset or a release included (reviews of 2026-11-20, security minor 4):
        // collections and loops, whose finish paths would never give the permit back (G1197)
        let kinds = self.def.projected_state();
        if kinds.collections || kinds.loops {
            return Err(StoreError::FlowControl(
                FlowControlRefusal::BinaryCapabilityMissing,
            ));
        }
        if flow.concurrency().is_some() != keys.concurrency.is_some()
            || flow.rate_limit().is_some() != keys.rate.is_some()
            || !flow.is_gated()
        {
            return Err(StoreError::Internal {
                detail: "a gated start came without its keys".into(),
            });
        }
        let (tenant, namespace) = (self.info.tenant_id, self.info.namespace_id);
        // the release of a start delay may queue a run outside its creation: it takes the namespace's admission lock
        // first, as every admission holds it from before its runs row, so that `queued_runs` stays exact (D022-051,
        // D022-052); an admission holds it already, and a reset never queues
        if keys.cause == StartCause::Release {
            crate::quotas::lock_run_admission(&mut self.tx, tenant, namespace).await?;
        }
        let versions = key_versions(&mut self.tx).await?;
        let ceilings = crate::quotas::step_gate_ceilings(&mut self.tx, tenant, namespace).await?;
        let cause = keys.cause;
        let declared = sagat_domain::execution::DeclaredGates::of(&flow)
            .ok_or_else(|| corrupt("a gated start that declares no gate"))?;
        let (mut semaphores, mut windows, mut pending) = (Vec::new(), Vec::new(), Vec::new());
        for (kind, key, gate) in self.gates_of(
            &ring,
            versions,
            &ceilings,
            &flow,
            None,
            0,
            keys.concurrency.as_deref(),
            keys.rate.as_deref(),
        )? {
            // a zero effective limit or timeout refuses before anything is filed (FR-022-034: zero is not unlimited)
            if gate.limit == 0 {
                return Err(StoreError::FlowControl(FlowControlRefusal::Limit {
                    limit: match gate.per {
                        None => "concurrency_limit",
                        Some(sagat_domain::workflow::RatePeriod::Second) => "rate_limit_per_second",
                        Some(sagat_domain::workflow::RatePeriod::Minute) => "rate_limit_per_minute",
                        Some(sagat_domain::workflow::RatePeriod::Hour) => "rate_limit_per_hour",
                    },
                }));
            }
            if gate.timeout.is_zero() {
                return Err(StoreError::FlowControl(FlowControlRefusal::Limit {
                    limit: "flow_control_queue_timeout_ms",
                }));
            }
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
            pending.push((index, gate));
        }
        // a key the start would create meets the namespace's live-keys ceiling of its kind (FR-022-054, G1199, T022-077),
        // counted on the versions in force only (ADR-0093 decision 5)
        for (table, keys_of) in [
            (KeyTable::Semaphores, &semaphores),
            (KeyTable::Windows, &windows),
        ] {
            if keys_of.is_empty()
                || !self
                    .existing_keys(table, GateLevel::Run, keys_of)
                    .await?
                    .is_empty()
            {
                continue;
            }
            let mut live = self.live_keys(table, versions).await?;
            if table == KeyTable::Windows && ceilings.admits_rate_key(live).is_err() {
                // the start's one window: a row the namespace's purge backlog keeps makes room (D022-066 item 1)
                live = live.saturating_sub(self.reclaim_windows(1, versions, keys_of).await?);
            }
            match table {
                KeyTable::Semaphores => ceilings.admits_key(live),
                KeyTable::Windows => ceilings.admits_rate_key(live),
            }
            .map_err(|limit| start_refused(limit, declared, None, cause))?;
        }
        let filed_semaphores =
            file_keys_with(&mut self.tx, KeyTable::Semaphores, &semaphores, &[]).await?;
        let filed_windows = file_keys_with(&mut self.tx, KeyTable::Windows, &windows, &[]).await?;
        self.decide_filed(
            pending,
            &filed_semaphores,
            &filed_windows,
            Some(StartRules { cause, ceilings }),
            &ceilings,
        )
        .await
    }
}

impl PgRunTx {
    /// A key gains a waiter: its hint is set so that the next pass that may grant the waiter visits the key (correctness
    /// review Major 1 of 2026-11-19; decision D022-076 items 1 and 2), early and never late.
    async fn key_due(
        &mut self,
        table: KeyTable,
        level: GateLevel,
        group: &sagat_domain::workflow::ResolvedGroup,
        digest: &[u8; 32],
        has_room: bool,
        period_ms: i64,
    ) -> Result<(), StoreError> {
        // a waiter queued while its gate had room (behind an eligible waiter, strict order) can be granted by the next
        // pass: the hint is written to the statement's instant whatever it was, so a pass in progress on the key, which
        // listed its waiters before this one and rests the key only while its hint is the one it selected, keeps the key
        // due (decision D022-076 item 1). A waiter queued at a full key: an absent hint becomes now; a semaphore keeps the
        // hint it found (a release sets it); a window lowers it to the first instant a grant it counts for the new
        // waiter's own window ages out of that window (an unleased grant from now: it frees by a return or a lease, each
        // of which sets the hint), so a hint computed for a longer window never holds a shorter one back (item 2)
        let statement = match table {
            KeyTable::Semaphores => {
                "UPDATE concurrency_semaphores SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = $6 \
                    AND group_name = $4 AND key_digest = $5 AND ($7 OR next_eligible_at IS NULL) \
                    AND $8::bigint >= 0"
            }
            KeyTable::Windows => {
                "UPDATE rate_limit_windows w \
                    SET next_eligible_at = CASE WHEN $7 OR w.next_eligible_at IS NULL THEN clock_timestamp() \
                          ELSE least(w.next_eligible_at, \
                                     (SELECT min(coalesce(g.leased_at, clock_timestamp()) \
                                                 + make_interval(secs => $8::bigint / 1000.0)) \
                                        FROM rate_limit_grants g \
                                       WHERE g.tenant_id = w.tenant_id AND g.namespace_id = w.namespace_id \
                                         AND g.pool = w.pool AND g.limit_scope = w.limit_scope \
                                         AND g.group_name = w.group_name AND g.key_digest = w.key_digest \
                                         AND (g.leased_at IS NULL \
                                              OR g.leased_at > clock_timestamp() \
                                                               - make_interval(secs => $8::bigint / 1000.0)))) END, \
                        updated_at = clock_timestamp() \
                  WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = $3 AND w.limit_scope = $6 \
                    AND w.group_name = $4 AND w.key_digest = $5"
            }
        };
        sqlx::query(statement)
            .bind(self.info.tenant_id.as_uuid())
            .bind(self.info.namespace_id.as_uuid())
            .bind(GatePool::Ordinary.as_str())
            .bind(group.as_str())
            .bind(digest.as_slice())
            .bind(level.as_str())
            .bind(has_room)
            .bind(period_ms)
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
              WHERE w.tenant_id = $1 AND w.run_id = $2 \
                AND s.tenant_id = w.tenant_id AND s.namespace_id = w.namespace_id AND s.pool = w.pool \
                AND s.permit_scope = w.permit_scope AND s.group_name = w.group_name AND s.key_digest = w.key_digest \
                AND s.next_eligible_at IS NULL",
            "UPDATE rate_limit_windows s SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
               FROM rate_limit_waiters w \
              WHERE w.tenant_id = $1 AND w.run_id = $2 \
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

    /// Clears the lease instant of the grant of `node`'s attempt `attempt` after its lease was withdrawn (decisions D022-138
    /// and D022-139): the slot counts as unleased again, without end, until the lease pass records the attempt's next lease.
    /// It runs after this commit's gate returns (`persist`, step 0), so a window row is never locked after this grant row,
    /// as everywhere else (D022-038), and it finds no row when this commit gave the slot back. The write always makes a new
    /// row version, the instant recorded or not, so that a lease pass that read the row before this commit does not write
    /// the withdrawn lease on its recheck.
    pub(super) async fn clear_withdrawn_lease(
        &mut self,
        node: &NodeId,
        attempt: u32,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE rate_limit_grants SET leased_at = NULL WHERE tenant_id = $1 AND run_id = $2 \
               AND limit_scope = 'STEP' AND node = $3 AND attempt = $4",
        )
        .bind(self.info.tenant_id.as_uuid())
        .bind(self.info.run_id.as_uuid())
        .bind(node.as_str())
        .bind(i32::try_from(attempt).map_err(|_| corrupt("attempt"))?)
        .execute(&mut *self.tx)
        .await
        .map_err(store_error)?;
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
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND permit_scope = $6 \
                    AND group_name = $4 AND key_digest = $5 AND waiters > 0"
            }
            KeyTable::Windows => {
                "UPDATE rate_limit_windows SET next_eligible_at = clock_timestamp(), updated_at = clock_timestamp() \
                  WHERE tenant_id = $1 AND namespace_id = $2 AND pool = $3 AND limit_scope = $6 \
                    AND group_name = $4 AND key_digest = $5 AND waiters > 0"
            }
        };
        sqlx::query(statement)
            .bind(key.tenant.as_uuid())
            .bind(key.namespace.as_uuid())
            .bind(key.pool.as_str())
            .bind(key.group.as_str())
            .bind(key.digest.as_slice())
            .bind(key.level.as_str())
            .execute(&mut *self.tx)
            .await
            .map_err(store_error)?;
        Ok(())
    }
}

/// The stored projection of a step's gates, or `NULL` for a step that has none.
pub(super) fn gates_projection(step: &StepState) -> Result<Option<serde_json::Value>, StoreError> {
    project_gates(step.gates())
}

/// The stored projection of the gates of a run's start (`runs.flow_control_gates`, T022-022), or `NULL` for a run that
/// has none. The same shape as a step's, every attempt 0.
pub(super) fn run_gates_projection(gates: &Gates) -> Result<Option<serde_json::Value>, StoreError> {
    project_gates(gates)
}

fn project_gates(gates: &Gates) -> Result<Option<serde_json::Value>, StoreError> {
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
/// precedence of FR-022-052, data-model "Steps and their gates"). A step that a pause gave its gates back waits to ask
/// again on resume (FR-022-046): its first declared gate, rate first. Any other step that waits with no wait journaled
/// is refused: a commit never leaves a gated step undecided (invariant 11).
pub(super) fn gate_waiting_kind(
    step: &StepState,
    declared: Option<sagat_domain::execution::DeclaredGates>,
    waits_for_resume: bool,
) -> Result<&'static str, StoreError> {
    let kind = match step.gates().waiting().next() {
        Some((kind, _)) => Some(kind),
        // `RunState::waits_for_resume`, the domain's one predicate (ADR-0096)
        None if waits_for_resume => declared.and_then(|declared| declared.kinds().next()),
        None => None,
    };
    match kind {
        Some(GateKind::RateLimit) => Ok("RATE_LIMIT"),
        Some(GateKind::Concurrency) => Ok("CONCURRENCY"),
        None => Err(StoreError::Internal {
            detail: "a gated step would be written waiting without a decision of its gates".into(),
        }),
    }
}

/// A rate grant was added to a window that has waiters: the window's hint moves no later than `now` plus the shortest
/// window its waiters count over (an unleased grant frees by a return, which sets the hint, or by a lease from now on;
/// a run's grant counts from `now`), so a waiter refused by this grant's slot is visited when it may first count no more
/// (correctness review Major 4 of 2026-11-20; plan "The grant pass, by key"). The window row is the caller's, locked.
pub(super) async fn window_due_after_grant(
    tx: &mut sqlx::PgConnection,
    (tenant, namespace, level, group): (
        sagat_domain::identity::TenantId,
        sagat_domain::identity::NamespaceId,
        GateLevel,
        &sagat_domain::workflow::ResolvedGroup,
    ),
    digest: &[u8; 32],
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE rate_limit_windows w \
            SET next_eligible_at = least(w.next_eligible_at, \
                    $6 + make_interval(secs => (SELECT min(x.period_ms) FROM rate_limit_waiters x \
                                                 WHERE x.tenant_id = w.tenant_id AND x.namespace_id = w.namespace_id \
                                                   AND x.pool = w.pool AND x.limit_scope = w.limit_scope \
                                                   AND x.group_name = w.group_name AND x.key_digest = w.key_digest) \
                                        / 1000.0)), \
                updated_at = clock_timestamp() \
          WHERE w.tenant_id = $1 AND w.namespace_id = $2 AND w.pool = 'ORDINARY' AND w.limit_scope = $3 \
            AND w.group_name = $4 AND w.key_digest = $5 AND w.waiters > 0",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(level.as_str())
    .bind(group.as_str())
    .bind(digest.as_slice())
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(store_error)?;
    Ok(())
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
fn key_shard(
    level: GateLevel,
    group: &sagat_domain::workflow::ResolvedGroup,
    digest: &[u8; 32],
) -> i16 {
    let mut hash = Sha256::new();
    hash.update(b"sagat.flow-control.key-shard.v1\0");
    hash.update(GatePool::Ordinary.as_str().as_bytes());
    hash.update(b"\0");
    hash.update(level.as_str().as_bytes());
    hash.update(b"\0");
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
    /// Granted by the grant routine, with how long the waiter waited (from its queueing to its grant) and, for a slot of
    /// the pass's own rate-limit key, the grant's lag (from the instant the key became due, or the waiter's queueing if
    /// later, to the grant), both in microseconds of the database clock (T022-082; decision D022-083).
    GrantedFromQueue {
        waited_us: Option<i64>,
        lag_us: Option<i64>,
    },
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
                    Self::GrantedFromQueue {
                        waited_us: None,
                        lag_us: None,
                    }
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

/// The refusal of a reset whose gates do not admit it (a reset never waits; D022-053, D022-069): the first gate in
/// `refusing` names it, rate first (FR-022-052), `RATE_LIMIT_SLOT_UNAVAILABLE` or `CONCURRENCY_PERMIT_UNAVAILABLE`, and each
/// family the start declares counts it `refused_reset`, since the refused transaction rolls back.
pub(super) fn reset_refused(
    declared: sagat_domain::execution::DeclaredGates,
    refusing: &[GateKind],
) -> StoreError {
    use sagat_telemetry::flow_control::{
        ConcurrencyOutcome, FlowControlScope, RateLimitOutcome, record_concurrency,
        record_rate_limit,
    };
    if declared.concurrency {
        record_concurrency(FlowControlScope::Run, ConcurrencyOutcome::RefusedReset);
    }
    if declared.rate {
        record_rate_limit(FlowControlScope::Run, RateLimitOutcome::RefusedReset);
    }
    match refusing.first() {
        Some(GateKind::RateLimit) => StoreError::FlowControl(FlowControlRefusal::SlotUnavailable),
        Some(GateKind::Concurrency) => {
            StoreError::FlowControl(FlowControlRefusal::PermitUnavailable)
        }
        None => corrupt("a reset refused by no gate"),
    }
}

/// A run's start refused by a ceiling (T022-022): the typed refusal of `contracts/errors.md`, counted at once as
/// `concurrency_acquisitions_total{scope="run",outcome="refused_full"}` (FR-022-033), since the refused transaction rolls
/// back and nothing of it is counted at commit (architecture review M3 of 2026-11-20).
pub(super) fn start_refused(
    limit: sagat_domain::workflow::FlowControlLimit,
    declared: sagat_domain::execution::DeclaredGates,
    key_of: Option<GateKind>,
    cause: StartCause,
) -> StoreError {
    use sagat_domain::workflow::FlowControlLimit;
    use sagat_telemetry::flow_control::{
        ConcurrencyOutcome, FlowControlScope, RateLimitOutcome, record_concurrency,
        record_rate_limit,
    };
    if cause != StartCause::Release {
        // each family counts the refusal of a start that declares its gate
        if declared.concurrency {
            record_concurrency(FlowControlScope::Run, ConcurrencyOutcome::RefusedFull);
        }
        if declared.rate {
            record_rate_limit(FlowControlScope::Run, RateLimitOutcome::RefusedFull);
        }
    }
    // a full key is named by its own kind (`key_of`, the gate whose key's buffer is full; correctness review minor 1 and
    // architecture review Major 6 of 2026-11-20); a full namespace is `RATE_LIMIT_QUEUE_FULL` for a start that declares a rate
    // limit, rate first (FR-022-052, `contracts/errors.md` US07)
    StoreError::FlowControl(match limit {
        FlowControlLimit::KeyWaiters if key_of == Some(GateKind::RateLimit) => {
            FlowControlRefusal::RateLimitQueueFull
        }
        FlowControlLimit::QueuedRuns if declared.rate => FlowControlRefusal::RateLimitQueueFull,
        FlowControlLimit::KeyWaiters => FlowControlRefusal::ConcurrencyQueueFull,
        FlowControlLimit::QueuedRuns => FlowControlRefusal::Limit {
            limit: "queued_runs",
        },
        FlowControlLimit::RateLiveKeys => FlowControlRefusal::Limit {
            limit: "rate_limit_live_keys",
        },
        FlowControlLimit::LiveKeys | FlowControlLimit::NodeDeclarations { .. } => {
            FlowControlRefusal::Limit {
                limit: "concurrency_live_keys",
            }
        }
    })
}

/// Records the decisions of a committed transaction at `scope` (G022-001; FR-022-033, FR-022-049).
pub(super) fn record_gate_outcomes(
    outcomes: &[(GateKind, GateOutcome)],
    scope: sagat_telemetry::flow_control::FlowControlScope,
) {
    use sagat_telemetry::flow_control::{
        ConcurrencyOutcome, RateLimitOutcome, record_concurrency, record_concurrency_wait,
        record_grant_lag, record_rate_limit, record_rate_limit_wait,
    };
    #[allow(clippy::cast_precision_loss)] // a histogram's observation
    let seconds = |us: i64| us.max(0) as f64 / 1_000_000.0;
    for (kind, outcome) in outcomes {
        // the wait of a waiter granted from the queue, and the lag of a slot's grant (plan "Observability"; T022-082)
        if let GateOutcome::GrantedFromQueue { waited_us, lag_us } = outcome {
            if let Some(waited) = waited_us {
                match kind {
                    GateKind::Concurrency => record_concurrency_wait(scope, seconds(*waited)),
                    GateKind::RateLimit => record_rate_limit_wait(scope, seconds(*waited)),
                }
            }
            if let (GateKind::RateLimit, Some(lag)) = (kind, lag_us) {
                record_grant_lag(seconds(*lag));
            }
        }
        match kind {
            GateKind::Concurrency => {
                let label = match outcome {
                    GateOutcome::Granted => Some(ConcurrencyOutcome::Acquired),
                    GateOutcome::Queued => Some(ConcurrencyOutcome::Queued),
                    GateOutcome::GrantedFromQueue { .. } => Some(ConcurrencyOutcome::Granted),
                    GateOutcome::TimedOut => Some(ConcurrencyOutcome::TimedOut),
                    GateOutcome::Dequeued => Some(ConcurrencyOutcome::Dequeued),
                    // a release is not an acquisition: the vocabulary has no outcome for it
                    GateOutcome::GivenBack => None,
                };
                if let Some(label) = label {
                    record_concurrency(scope, label);
                }
            }
            GateKind::RateLimit => record_rate_limit(
                scope,
                match outcome {
                    GateOutcome::Granted => RateLimitOutcome::Granted,
                    GateOutcome::Queued => RateLimitOutcome::Queued,
                    GateOutcome::GrantedFromQueue { .. } => RateLimitOutcome::GrantedFromQueue,
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
    use super::{GateOutcome, record_gate_outcomes, reset_refused, start_refused};
    use sagat_domain::execution::{DeclaredGates, StartCause};

    const CONCURRENCY: DeclaredGates = DeclaredGates {
        concurrency: true,
        rate: false,
    };

    /// A reset is never buffered (D022-053, D022-069): the first gate that does not admit it names the refusal, rate
    /// first (FR-022-052), and each family the start declares counts it `refused_reset`, a rate-only reset no concurrency
    /// refusal.
    #[test]
    fn a_refused_reset_is_named_by_its_first_refusing_gate_and_counted_by_each_declared_family() {
        use sagat_application::error::StoreError;
        use sagat_application::flow_control::FlowControlRefusal;
        let both = DeclaredGates {
            concurrency: true,
            rate: true,
        };
        let rate = DeclaredGates {
            concurrency: false,
            rate: true,
        };
        let permit = DeclaredGates {
            concurrency: true,
            rate: false,
        };
        let (telemetry, recorder) =
            sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
        let reasons = metrics::with_local_recorder(&recorder, || {
            [
                (rate, vec![GateKind::RateLimit]),
                // both refuse: rate first (the gates are in that order)
                (both, vec![GateKind::RateLimit, GateKind::Concurrency]),
                (both, vec![GateKind::Concurrency]),
                (permit, vec![GateKind::Concurrency]),
            ]
            .map(
                |(declared, refusing)| match reset_refused(declared, &refusing) {
                    StoreError::FlowControl(reason) => reason,
                    other => panic!("{other:?}"),
                },
            )
        });
        assert_eq!(
            reasons,
            [
                FlowControlRefusal::SlotUnavailable,
                FlowControlRefusal::SlotUnavailable,
                FlowControlRefusal::PermitUnavailable,
                FlowControlRefusal::PermitUnavailable,
            ]
        );
        let rendered = telemetry.render();
        let counted = |family: &str, n: &str| {
            rendered.lines().any(|line| {
                line.starts_with(family)
                    && line.contains("scope=\"run\"")
                    && line.contains("outcome=\"refused_reset\"")
                    && line.ends_with(n)
            })
        };
        assert!(counted("rate_limit_decisions_total", " 3"), "{rendered}");
        assert!(
            counted("concurrency_acquisitions_total", " 3"),
            "{rendered}"
        );
    }

    /// A start that declares a rate limit is refused rate first (FR-022-052, `contracts/errors.md` US07): its key's full
    /// buffer and the namespace's full `queued_runs` are `RATE_LIMIT_QUEUE_FULL`, its live keys `FLOW_CONTROL_LIMIT`
    /// naming `rate_limit_live_keys`; each declared family counts the refusal once.
    #[test]
    fn a_refused_rate_limited_start_is_refused_rate_first_and_counted_by_each_family() {
        use sagat_application::error::StoreError;
        use sagat_application::flow_control::FlowControlRefusal;
        use sagat_domain::workflow::FlowControlLimit;
        let both = DeclaredGates {
            concurrency: true,
            rate: true,
        };
        let rate = DeclaredGates {
            concurrency: false,
            rate: true,
        };
        let (telemetry, recorder) =
            sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
        let reasons = metrics::with_local_recorder(&recorder, || {
            [
                (
                    FlowControlLimit::KeyWaiters,
                    rate,
                    Some(GateKind::RateLimit),
                ),
                (
                    FlowControlLimit::QueuedRuns,
                    rate,
                    Some(GateKind::RateLimit),
                ),
                (
                    FlowControlLimit::KeyWaiters,
                    both,
                    Some(GateKind::RateLimit),
                ),
                (FlowControlLimit::RateLiveKeys, rate, None),
                (FlowControlLimit::LiveKeys, both, None),
                // a dual-gated start whose concurrency key is full names that key, not the rate buffer
                (
                    FlowControlLimit::KeyWaiters,
                    both,
                    Some(GateKind::Concurrency),
                ),
                // a full namespace is the rate refusal of a start that declares a rate limit, whichever key counted it
                (
                    FlowControlLimit::QueuedRuns,
                    both,
                    Some(GateKind::Concurrency),
                ),
            ]
            .map(|(limit, declared, key_of)| {
                match start_refused(limit, declared, key_of, StartCause::Admission) {
                    StoreError::FlowControl(reason) => reason,
                    other => panic!("{other:?}"),
                }
            })
        });
        assert_eq!(
            reasons,
            [
                FlowControlRefusal::RateLimitQueueFull,
                FlowControlRefusal::RateLimitQueueFull,
                FlowControlRefusal::RateLimitQueueFull,
                FlowControlRefusal::Limit {
                    limit: "rate_limit_live_keys"
                },
                FlowControlRefusal::Limit {
                    limit: "concurrency_live_keys"
                },
                FlowControlRefusal::ConcurrencyQueueFull,
                FlowControlRefusal::RateLimitQueueFull,
            ]
        );
        let rendered = telemetry.render();
        let counted = |family: &str, n: &str| {
            rendered.lines().any(|line| {
                line.starts_with(family)
                    && line.contains("scope=\"run\"")
                    && line.contains("outcome=\"refused_full\"")
                    && line.ends_with(n)
            })
        };
        assert!(counted("rate_limit_decisions_total", " 7"), "{rendered}");
        assert!(
            counted("concurrency_acquisitions_total", " 4"),
            "{rendered}"
        );
    }

    /// A refused start is counted at run scope as `refused_full` and maps each ceiling to its typed refusal.
    #[test]
    fn a_refused_start_is_counted_and_typed() {
        use sagat_application::flow_control::FlowControlRefusal;
        use sagat_domain::workflow::FlowControlLimit;
        let (telemetry, recorder) =
            sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
        let refusals = metrics::with_local_recorder(&recorder, || {
            [
                FlowControlLimit::KeyWaiters,
                FlowControlLimit::QueuedRuns,
                FlowControlLimit::LiveKeys,
            ]
            .map(|limit| {
                start_refused(
                    limit,
                    CONCURRENCY,
                    Some(GateKind::Concurrency),
                    StartCause::Admission,
                )
            })
        });
        // a release's refusal is deferred and counted by its pass (reviews of 2026-11-20, correctness M2): not here
        let deferred = metrics::with_local_recorder(&recorder, || {
            start_refused(
                FlowControlLimit::KeyWaiters,
                CONCURRENCY,
                Some(GateKind::Concurrency),
                StartCause::Release,
            )
        });
        assert!(matches!(
            deferred,
            sagat_application::error::StoreError::FlowControl(
                FlowControlRefusal::ConcurrencyQueueFull
            )
        ));
        let reasons: Vec<_> = refusals
            .iter()
            .map(|e| match e {
                sagat_application::error::StoreError::FlowControl(r) => *r,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            reasons,
            [
                FlowControlRefusal::ConcurrencyQueueFull,
                FlowControlRefusal::Limit {
                    limit: "queued_runs"
                },
                FlowControlRefusal::Limit {
                    limit: "concurrency_live_keys"
                },
            ]
        );
        let rendered = telemetry.render();
        assert!(
            rendered
                .lines()
                .any(|line| line.starts_with("concurrency_acquisitions_total")
                    && line.contains("scope=\"run\"")
                    && line.contains("outcome=\"refused_full\"")
                    && line.ends_with(" 3")),
            "{rendered}"
        );
    }
    use sagat_domain::workflow::GateKind;

    /// T022-082: a waiter granted from the queue records how long it waited, by family and scope, and a slot's grant its
    /// lag, in the closed vocabulary; a grant at once and a grant without the readings record neither.
    #[test]
    fn a_grant_from_the_queue_records_its_wait_and_a_slot_its_lag() {
        let (telemetry, recorder) =
            sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
        let from_queue = GateOutcome::GrantedFromQueue {
            waited_us: Some(2_500_000),
            lag_us: Some(750_000),
        };
        metrics::with_local_recorder(&recorder, || {
            record_gate_outcomes(
                &[
                    (GateKind::Concurrency, from_queue),
                    (GateKind::RateLimit, from_queue),
                    (GateKind::RateLimit, GateOutcome::Granted),
                ],
                sagat_telemetry::flow_control::FlowControlScope::Run,
            );
        });
        let rendered = telemetry.render();
        let line = |prefix: &str| {
            rendered
                .lines()
                .find(|line| line.starts_with(prefix))
                .unwrap_or_else(|| panic!("{prefix}: {rendered}"))
                .to_owned()
        };
        assert!(
            line("concurrency_wait_seconds_sum{scope=\"run\"}").ends_with(" 2.5"),
            "{rendered}"
        );
        assert!(
            line("concurrency_wait_seconds_count{scope=\"run\"}").ends_with(" 1"),
            "{rendered}"
        );
        assert!(
            line("rate_limit_wait_seconds_sum{scope=\"run\"}").ends_with(" 2.5"),
            "{rendered}"
        );
        assert!(
            line("rate_limit_grant_lag_seconds_sum").ends_with(" 0.75"),
            "{rendered}"
        );
        assert!(
            line("rate_limit_grant_lag_seconds_count").ends_with(" 1"),
            "{rendered}"
        );
    }

    /// G022-001: what a committed transaction decided is counted once per gate, step scope, in the closed vocabulary;
    /// a released permit is not an acquisition and is not counted as one.
    #[test]
    fn each_committed_gate_outcome_is_counted_in_the_closed_vocabulary() {
        let (telemetry, recorder) =
            sagat_telemetry::Metrics::detached(std::time::Duration::from_secs(60));
        let all = [
            GateOutcome::Granted,
            GateOutcome::Queued,
            GateOutcome::GrantedFromQueue {
                waited_us: None,
                lag_us: None,
            },
            GateOutcome::TimedOut,
            GateOutcome::Dequeued,
            GateOutcome::GivenBack,
        ];
        let outcomes: Vec<_> = [GateKind::Concurrency, GateKind::RateLimit]
            .into_iter()
            .flat_map(|kind| all.map(|outcome| (kind, outcome)))
            .collect();
        metrics::with_local_recorder(&recorder, || {
            record_gate_outcomes(
                &outcomes,
                sagat_telemetry::flow_control::FlowControlScope::Step,
            );
        });
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
