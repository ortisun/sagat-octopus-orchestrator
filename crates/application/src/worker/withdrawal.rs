//! The withdrawal of leases that never reached a handler (feature 006, decisions D006-001 to D006-018 of
//! `specs/006-sdk-multilanguage/research.md`; ADR-0098).
//!
//! A lease is withdrawn only while the platform requires `lease-withdrawal-v1` of control planes and engines (the gate
//! keeps the recorded requirements, and each transaction reads the requirement again under a share lock a removal waits
//! for, decision D006-032), in one fenced transaction per run (root leases) or per run and instance path (scoped leases),
//! only while its step is still `LEASED` with the generation and the lease-token hash the server issued, and at most
//! [`MAX_WITHDRAWALS_PER_ATTEMPT`] times for one attempt (decision D006-031). Anything else is counted and changes nothing:
//! the lease lapses as it did before, or a host's hand-back is an ordinary failure.
use std::collections::BTreeMap;

use sagat_domain::execution::{InstancePath, MAX_WITHDRAWALS_PER_ATTEMPT, WithdrawalCause};

use super::*;
use crate::instances::{Capability, InstanceKind, ServingGate};

/// The code a host reports through `FailStep` for a lease it received and will not run (`sdks/SPEC.md` §6.2, decision
/// D006-005).
pub const TASK_NOT_STARTED: &str = "TASK_NOT_STARTED";

/// The first protocol minor that reserves [`TASK_NOT_STARTED`] for the host (delivery acknowledgement, 1.8; decision
/// D006-030, ADR-0099). A session of an older minor may forward a handler's code, so its hand-back is an ordinary failure.
pub const HAND_BACK_PROTOCOL_MINOR: u32 = 8;

// Raising this build's ceiling to the minor that reserves the code turns the withdrawal of hand-backs on with no other
// change; before that, the minor must be read for the lease (the session row is overwritten by a new registration of the same
// worker), as ADR-0099 decision 9 says (round-3 security Minor 1).
const _: () = assert!(
    super::PROTOCOL_MINOR < HAND_BACK_PROTOCOL_MINOR,
    "read the holder's minor per lease (ADR-0099 decision 9) before this build negotiates the minor that reserves TASK_NOT_STARTED"
);

/// A lease the server committed that no handler received: what its withdrawal is fenced on.
#[derive(Clone)]
pub struct UndeliveredLease {
    pub tenant_id: TenantId,
    pub run_id: RunId,
    pub instance: ExecutionInstanceId,
    pub step_id: Uuid,
    pub fence: Fence,
    pub lease_token: Token,
}

impl UndeliveredLease {
    /// The lease of `task`, claimed for a principal of `tenant_id`.
    #[must_use]
    pub fn of(tenant_id: TenantId, task: &ClaimedTask) -> Self {
        Self {
            tenant_id,
            run_id: task.run_id,
            instance: task.instance.clone(),
            step_id: task.step_id,
            fence: task.fence,
            lease_token: task.lease_token.clone(),
        }
    }
}

impl std::fmt::Debug for UndeliveredLease {
    /// The lease token is a bearer secret: it is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UndeliveredLease")
            .field("run_id", &self.run_id)
            .field("instance", &self.instance)
            .field("fence", &self.fence)
            .field("lease_token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// What became of one lease handed to the withdrawal (the `outcome` label of `worker_leases_undelivered_total`, decision
/// D006-018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalOutcome {
    /// The step is `READY` again and the attempt was given back.
    Withdrawn,
    /// The step had moved on (a heartbeat started it, it was reported, or the run ended): nothing was changed.
    FenceMissed,
    /// The platform does not require `lease-withdrawal-v1` yet, or the host's session is older than
    /// [`HAND_BACK_PROTOCOL_MINOR`]: the lease lapses as before, or the hand-back is an ordinary failure.
    Lapsed,
    /// The attempt was withdrawn [`MAX_WITHDRAWALS_PER_ATTEMPT`] times already (decision D006-031): a lease not sent lapses,
    /// a hand-back is an ordinary failure.
    Capped,
    /// Feature 022 (decisions D022-157 and D022-158): the step's rate slot had stopped counting and another grant of its
    /// key used the room meanwhile, so withdrawing would let it run in that grant's window: a lease not sent lapses, a
    /// hand-back is an ordinary failure (the attempt is spent, gap G022-042).
    SlotSpent,
    /// This engine runs with `SAGAT_LEASE_WITHDRAWAL=off` (decision D006-032): the lease lapses.
    Disabled,
    /// The store failed: the lease lapses.
    Error,
}

impl WithdrawalOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Withdrawn => "withdrawn",
            Self::FenceMissed => "fence_missed",
            Self::Lapsed => "lapsed",
            Self::Capped => "capped",
            Self::SlotSpent => "slot_spent",
            Self::Disabled => "disabled",
            Self::Error => "error",
        }
    }
}

/// Counts `n` leases of `cause` that ended in `outcome` (`worker_leases_undelivered_total`, decision D006-018): the one place
/// the labels are written.
pub fn count(cause: WithdrawalCause, outcome: WithdrawalOutcome, n: u64) {
    if n > 0 {
        metrics::counter!(
            "worker_leases_undelivered_total",
            "cause" => cause.as_str(),
            "outcome" => outcome.as_str()
        )
        .increment(n);
    }
}

/// A stale or illegal fence is a miss; anything else is a failure of the transaction.
const fn is_fence_miss(error: &ExecutionError) -> bool {
    matches!(
        error,
        ExecutionError::StaleAttempt { .. }
            | ExecutionError::StaleInstanceAttempt { .. }
            | ExecutionError::IllegalTransition { .. }
            | ExecutionError::IllegalInstanceTransition { .. }
            | ExecutionError::RunFinished(_)
            | ExecutionError::UnknownStep(_)
            | ExecutionError::UnknownInstance(_)
    )
}

use sagat_domain::execution::ExecutionError;

impl<S, C, E, R> WorkerService<S, C, E, R> {
    /// Withdraws undelivered leases only while `gate` keeps `lease-withdrawal-v1` required of control planes and engines.
    /// Without a gate (a process with no registry) nothing is ever withdrawn.
    #[must_use]
    pub fn with_lease_withdrawal(mut self, gate: Arc<ServingGate>) -> Self {
        self.lease_withdrawal = Some(gate);
        self
    }

    /// Lets a test judge hand-backs as a build whose ceiling is `ceiling` would (the minor of the session that holds a
    /// lease is never read above this build's own ceiling, ADR-0099 decision 9). Production keeps [`super::PROTOCOL_MINOR`],
    /// so no session of this build is honoured before 1.8 negotiates (gap G006-030).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub const fn with_hand_back_ceiling(mut self, ceiling: u32) -> Self {
        self.hand_back_ceiling = ceiling;
        self
    }

    /// Refuses the capability use cases (secrets, database credentials, blob reads) inside their transaction while `gate`'s
    /// capability window has lapsed (decision D006-100): the same gate the transport asks.
    #[must_use]
    pub fn with_capability_gate(mut self, gate: Arc<ServingGate>) -> Self {
        self.capability_gate = Some(gate);
        self
    }

    /// Stops this process writing new withdrawals when `enabled` is false (`SAGAT_LEASE_WITHDRAWAL=off`, decision D006-032):
    /// its leases lapse as before, and it still lists the capability, because it can still replay the event.
    #[must_use]
    pub const fn with_new_withdrawals(mut self, enabled: bool) -> Self {
        self.lease_withdrawal_disabled = !enabled;
        self
    }

    /// Whether the platform requires `lease-withdrawal-v1` of every control plane and engine, as this process last read,
    /// and this process writes new withdrawals. The transaction of a withdrawal reads the requirement again under a lock.
    #[must_use]
    pub fn withdraws_leases(&self) -> bool {
        self.gate_outcome().is_none()
    }

    /// Why this process withdraws nothing now, or `None` when it may.
    pub(crate) fn gate_outcome(&self) -> Option<WithdrawalOutcome> {
        if self.lease_withdrawal_disabled {
            return Some(WithdrawalOutcome::Disabled);
        }
        let required = self
            .lease_withdrawal
            .as_ref()
            .is_some_and(|gate| gate.requires(Capability::LeaseWithdrawalV1, &InstanceKind::ALL));
        (!required).then_some(WithdrawalOutcome::Lapsed)
    }
}

/// What a host's `TASK_NOT_STARTED` becomes (decisions D006-030, D006-031 and D006-089).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandBackVerdict {
    /// Withdrawn, the attempt given back; the count of the earlier withdrawals of the attempt.
    Withdraw(u32),
    /// The ordinary failure, with the host's `retryable` flag, counted as the outcome.
    Fail(WithdrawalOutcome),
    /// Nothing recorded and `STALE_ATTEMPT` answered, so the lease lapses as before: a session that reserves the code
    /// handed back on a fleet that does not withdraw (the requirement not recorded, or the switch off).
    Lapse(WithdrawalOutcome),
}

/// [`hand_back`]'s refusal sorted (decision D006-089): a refusal for a reason of the fleet (`Lapsed`, `Disabled`) from a session
/// that reserves the code, of a lease still `LEASED` to it, leaves the lease to lapse; any other refusal is the ordinary
/// failure. `stored_minor` is the minor the session of the lease negotiated, as stored; it is judged clamped to `ceiling`,
/// this build's own (ADR-0099 decision 9), so a build below 1.8 never withdraws a hand-back of 1.8. A session stored at a
/// minor that reserves the code, clamped below it, lapses too (decision D006-099): its host's word that nothing ran is not
/// turned into a failure that spends the attempt (a binary rolled back while hosts of 1.8 hand back).
#[must_use]
pub fn hand_back_verdict(
    gate: Option<WithdrawalOutcome>,
    leased: bool,
    stored_minor: Option<u32>,
    ceiling: u32,
    required: bool,
    before: u32,
) -> HandBackVerdict {
    let holder_minor = stored_minor.map(|minor| minor.min(ceiling));
    let reserves =
        |minor: Option<u32>| minor.is_some_and(|minor| minor >= HAND_BACK_PROTOCOL_MINOR);
    match hand_back(gate, leased, holder_minor, required, before) {
        Ok(before) => HandBackVerdict::Withdraw(before),
        Err(outcome @ (WithdrawalOutcome::Lapsed | WithdrawalOutcome::Disabled))
            if leased && reserves(holder_minor) =>
        {
            HandBackVerdict::Lapse(outcome)
        }
        // stored at 1.8 or later, judged below it: this build cannot honour it, and must not fail it (decision D006-099)
        Err(WithdrawalOutcome::Lapsed) if leased && reserves(stored_minor) => {
            HandBackVerdict::Lapse(WithdrawalOutcome::Lapsed)
        }
        Err(outcome) => HandBackVerdict::Fail(outcome),
    }
}

/// What a host's `TASK_NOT_STARTED` becomes (decision D006-030): a withdrawal given the count of the earlier withdrawals of
/// the attempt, or the ordinary failure, with the outcome it is counted as. The reads are the caller's, in its transaction:
/// `gate` is [`WorkerService::withdraws_leases`]'s verdict, `leased` whether the step is still `LEASED` to this very lease,
/// `holder_minor` the minor of the session that holds the lease, `required` the requirement under its lock, `before` the
/// withdrawals of the attempt so far.
///
/// # Errors
/// The outcome that says why the report is an ordinary failure.
pub fn hand_back(
    gate: Option<WithdrawalOutcome>,
    leased: bool,
    holder_minor: Option<u32>,
    required: bool,
    before: u32,
) -> Result<u32, WithdrawalOutcome> {
    if !leased {
        return Err(WithdrawalOutcome::FenceMissed);
    }
    if holder_minor.is_none_or(|minor| minor < HAND_BACK_PROTOCOL_MINOR) {
        return Err(WithdrawalOutcome::Lapsed);
    }
    if let Some(outcome) = gate {
        return Err(outcome);
    }
    if !required {
        return Err(WithdrawalOutcome::Lapsed);
    }
    if before >= MAX_WITHDRAWALS_PER_ATTEMPT {
        return Err(WithdrawalOutcome::Capped);
    }
    Ok(before)
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
    /// The verdict on a root `TASK_NOT_STARTED` (decision D006-030), read in the report's transaction: the step still
    /// `LEASED` to this unexpired lease, the minor of the session that holds it, the requirement under its lock, the
    /// withdrawals of the attempt so far. The cheap checks come first; the lock is taken only when all else holds.
    pub(super) async fn root_hand_back<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<HandBackVerdict, AppError> {
        let hash = Token::hash_of(task.lease_token);
        let leased = tx.parts().1.instance(task.instance).is_some_and(|step| {
            step.status() == StepStatus::Leased
                && step.attempt() == task.fence.attempt
                && step.generation() == task.fence.generation
        }) && tx.lease_unexpired_instance(task.instance, &hash).await?;
        let info = tx.info();
        let step_id = instance_step_id(info.version_id, info.run_id, task.instance);
        // judged never above this build's own ceiling: a session another build registered may store a minor this build does
        // not serve (ADR-0099 decision 9); the stored minor still says what the host promised (decision D006-099)
        let minor = if leased {
            tx.lease_holder_minor(step_id, &hash).await?
        } else {
            None
        };
        let ceiling = self.hand_back_ceiling;
        let gate = self.gate_outcome();
        let cheap = hand_back_verdict(gate, leased, minor, ceiling, true, 0);
        if !matches!(cheap, HandBackVerdict::Withdraw(_)) {
            return Ok(cheap);
        }
        let required = tx.lease_withdrawal_required().await?;
        let before = tx.lease_withdrawals(step_id, task.fence.attempt).await?;
        let verdict = hand_back_verdict(gate, leased, minor, ceiling, required, before);
        if !matches!(verdict, HandBackVerdict::Withdraw(_)) {
            return Ok(verdict);
        }
        // the step's keys locked and its slot judged before the command (feature 022, decisions D022-156 and D022-157): a
        // slot whose room another grant used makes the hand-back the ordinary failure, counted `slot_spent`
        let spent = tx
            .lock_withdrawn_gates(&[(task.instance.node().clone(), task.fence.attempt)])
            .await?;
        if spent.first().copied().unwrap_or(false) {
            return Ok(HandBackVerdict::Fail(WithdrawalOutcome::SlotSpent));
        }
        Ok(verdict)
    }

    /// Whether the session that holds the lease of `task` negotiated a minor that reserves `TASK_NOT_STARTED`, judged at this
    /// build's ceiling (decision D006-100): only such a hand-back passes a cancellation at the report fence.
    pub(super) async fn root_reserves_the_code<Tx: RunTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<bool, AppError> {
        let info = tx.info();
        let step_id = instance_step_id(info.version_id, info.run_id, task.instance);
        Ok(tx
            .lease_holder_minor(step_id, &Token::hash_of(task.lease_token))
            .await?
            .is_some_and(|minor| minor.min(self.hand_back_ceiling) >= HAND_BACK_PROTOCOL_MINOR))
    }

    /// [`Self::root_reserves_the_code`] for a scoped instance, in its path transaction.
    pub(super) async fn path_reserves_the_code<Tx: PathExecutionTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<bool, AppError> {
        let info = &tx.snapshot().info;
        let step_id = instance_step_id(info.version_id, info.run_id, task.instance);
        Ok(tx
            .lease_holder_minor(step_id, &Token::hash_of(task.lease_token))
            .await?
            .is_some_and(|minor| minor.min(self.hand_back_ceiling) >= HAND_BACK_PROTOCOL_MINOR))
    }

    /// [`Self::root_hand_back`] for a scoped instance, in its path transaction.
    pub(super) async fn path_hand_back<Tx: PathExecutionTx>(
        &self,
        tx: &mut Tx,
        task: &TaskRef<'_>,
    ) -> Result<HandBackVerdict, AppError> {
        let hash = Token::hash_of(task.lease_token);
        let leased = tx
            .snapshot()
            .state
            .local_instance(task.instance.node())
            .is_some_and(|step| {
                step.status() == StepStatus::Leased
                    && step.attempt() == task.fence.attempt
                    && step.generation() == task.fence.generation
            })
            && tx.lease_unexpired(task.instance, &hash).await?;
        let info = &tx.snapshot().info;
        let step_id = instance_step_id(info.version_id, info.run_id, task.instance);
        // judged never above this build's own ceiling: a session another build registered may store a minor this build does
        // not serve (ADR-0099 decision 9); the stored minor still says what the host promised (decision D006-099)
        let minor = if leased {
            tx.lease_holder_minor(step_id, &hash).await?
        } else {
            None
        };
        let ceiling = self.hand_back_ceiling;
        let gate = self.gate_outcome();
        let cheap = hand_back_verdict(gate, leased, minor, ceiling, true, 0);
        if !matches!(cheap, HandBackVerdict::Withdraw(_)) {
            return Ok(cheap);
        }
        let required = tx.lease_withdrawal_required().await?;
        let before = tx.lease_withdrawals(step_id, task.fence.attempt).await?;
        Ok(hand_back_verdict(
            gate, leased, minor, ceiling, required, before,
        ))
    }

    /// Withdraws `leases`, which the server committed and never handed to the transport (decision D006-001), and returns
    /// what became of each, in the order given. Never fails: a lease it cannot withdraw lapses as before, and is counted.
    pub async fn withdraw_undelivered(
        &self,
        leases: Vec<UndeliveredLease>,
        cause: WithdrawalCause,
    ) -> Vec<WithdrawalOutcome> {
        let total = leases.len();
        if total == 0 {
            return Vec::new();
        }
        if let Some(outcome) = self.gate_outcome() {
            count(cause, outcome, total as u64);
            return vec![outcome; total];
        }
        // one transaction per run (root) or per run and path (scoped), decision D006-012
        let mut groups: BTreeMap<(Uuid, RunId, InstancePath), Vec<(usize, UndeliveredLease)>> =
            BTreeMap::new();
        for (index, lease) in leases.into_iter().enumerate() {
            groups
                .entry((
                    lease.tenant_id.as_uuid(),
                    lease.run_id,
                    lease.instance.path().clone(),
                ))
                .or_default()
                .push((index, lease));
        }
        let mut outcomes = vec![WithdrawalOutcome::Error; total];
        for ((_, run, path), group) in groups {
            let indices: Vec<usize> = group.iter().map(|(index, _)| *index).collect();
            let leases: Vec<UndeliveredLease> = group.into_iter().map(|(_, lease)| lease).collect();
            let result = if path.is_root() {
                self.withdraw_root(run, &leases, cause).await
            } else {
                self.withdraw_path(run, path, &leases, cause).await
            };
            let group_outcomes = result.unwrap_or_else(|error| {
                tracing::warn!(%error, %run, "withdrawing undelivered leases failed; they lapse");
                vec![WithdrawalOutcome::Error; leases.len()]
            });
            for (index, outcome) in indices.into_iter().zip(group_outcomes) {
                outcomes[index] = outcome;
            }
        }
        for outcome in [
            WithdrawalOutcome::Withdrawn,
            WithdrawalOutcome::FenceMissed,
            WithdrawalOutcome::Lapsed,
            WithdrawalOutcome::Capped,
            WithdrawalOutcome::SlotSpent,
            WithdrawalOutcome::Error,
        ] {
            count(
                cause,
                outcome,
                outcomes.iter().filter(|o| **o == outcome).count() as u64,
            );
        }
        outcomes
    }

    async fn withdraw_root(
        &self,
        run: RunId,
        leases: &[UndeliveredLease],
        cause: WithdrawalCause,
    ) -> Result<Vec<WithdrawalOutcome>, AppError> {
        let Some(tenant) = leases.first().map(|lease| lease.tenant_id) else {
            return Ok(Vec::new());
        };
        let Some(mut tx) = self.store.begin(tenant, run, Lock::Wait).await? else {
            return Ok(vec![WithdrawalOutcome::FenceMissed; leases.len()]);
        };
        // read again under the lock a removal of the requirement waits for (decision D006-032)
        if !tx.lease_withdrawal_required().await? {
            return Ok(vec![WithdrawalOutcome::Lapsed; leases.len()]);
        }
        // every key the commands below may touch, locked once and sorted, and each slot judged there (feature 022,
        // decisions D022-156 and D022-157)
        let gated: Vec<(sagat_domain::workflow::NodeId, u32)> = leases
            .iter()
            .map(|lease| (lease.instance.node().clone(), lease.fence.attempt))
            .collect();
        let spent = tx.lock_withdrawn_gates(&gated).await?;
        let mut outcomes = Vec::with_capacity(leases.len());
        let mut withdrawn = false;
        for (lease, spent) in leases.iter().zip(spent) {
            let info = tx.info();
            let fenced = lease.step_id
                == instance_step_id(info.version_id, info.run_id, &lease.instance)
                && tx
                    .lease_matches_instance(&lease.instance, &lease.lease_token.hash())
                    .await?;
            if !fenced {
                outcomes.push(WithdrawalOutcome::FenceMissed);
                continue;
            }
            // a rate slot whose room another grant used is not made to count again (feature 022, decision D022-157)
            if spent {
                outcomes.push(WithdrawalOutcome::SlotSpent);
                continue;
            }
            let before = tx
                .lease_withdrawals(lease.step_id, lease.fence.attempt)
                .await?;
            let node = lease.instance.node().clone();
            let fence = lease.fence;
            let effects = Effects::by(Actor::System("lease-withdrawal"), None);
            match self
                .engine
                .run(&mut tx, effects, move |def, state, now| {
                    Ok((
                        (),
                        execution::withdraw_lease(def, state, &node, fence, cause, before, now)?,
                    ))
                })
                .await
            {
                Ok(()) => {
                    withdrawn = true;
                    outcomes.push(WithdrawalOutcome::Withdrawn);
                }
                Err(EngineError::Execution(ExecutionError::WithdrawalsSpent { .. })) => {
                    tracing::warn!(%run, node = %lease.instance.node(), attempt = lease.fence.attempt,
                        "an attempt was withdrawn as often as one attempt may be; its lease lapses");
                    outcomes.push(WithdrawalOutcome::Capped);
                }
                Err(EngineError::Execution(error)) if is_fence_miss(&error) => {
                    outcomes.push(WithdrawalOutcome::FenceMissed);
                }
                Err(error) => return Err(engine_error(error)),
            }
        }
        if withdrawn {
            tx.commit().await?;
        }
        Ok(outcomes)
    }

    async fn withdraw_path(
        &self,
        run: RunId,
        path: InstancePath,
        leases: &[UndeliveredLease],
        cause: WithdrawalCause,
    ) -> Result<Vec<WithdrawalOutcome>, AppError> {
        let Some(tenant) = leases.first().map(|lease| lease.tenant_id) else {
            return Ok(Vec::new());
        };
        if !self.store.supports_scoped_worker_protocol() {
            return Ok(vec![WithdrawalOutcome::FenceMissed; leases.len()]);
        }
        let Some(mut tx) = self
            .store
            .begin_current_path(tenant, run, path, Lock::Wait)
            .await?
        else {
            return Ok(vec![WithdrawalOutcome::FenceMissed; leases.len()]);
        };
        if !tx.lease_withdrawal_required().await? {
            return Ok(vec![WithdrawalOutcome::Lapsed; leases.len()]);
        }
        let mut outcomes = Vec::with_capacity(leases.len());
        let mut withdrawn = false;
        for lease in leases {
            let info = &tx.snapshot().info;
            let fenced = lease.step_id
                == instance_step_id(info.version_id, info.run_id, &lease.instance)
                && tx
                    .lease_matches(&lease.instance, &lease.lease_token.hash())
                    .await?;
            if !fenced {
                outcomes.push(WithdrawalOutcome::FenceMissed);
                continue;
            }
            let before = tx
                .lease_withdrawals(lease.step_id, lease.fence.attempt)
                .await?;
            let snapshot = tx.snapshot();
            match execution::withdraw_path_instance(
                &snapshot.definition,
                &mut snapshot.state,
                &lease.instance,
                lease.fence,
                cause,
                before,
            ) {
                Ok(progress) => {
                    let effects = Effects::by(Actor::System("lease-withdrawal"), None);
                    tx.persist_addressed(&progress.events, &[], effects).await?;
                    withdrawn = true;
                    outcomes.push(WithdrawalOutcome::Withdrawn);
                }
                Err(ExecutionError::WithdrawalsSpent { .. }) => {
                    tracing::warn!(%run, node = %lease.instance.node(), attempt = lease.fence.attempt,
                        "an attempt was withdrawn as often as one attempt may be; its lease lapses");
                    outcomes.push(WithdrawalOutcome::Capped);
                }
                Err(error) if is_fence_miss(&error) => {
                    outcomes.push(WithdrawalOutcome::FenceMissed);
                }
                Err(error) => return Err(engine_error(EngineError::Execution(error))),
            }
        }
        if withdrawn {
            tx.commit().await?;
        }
        Ok(outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The verdict on a host's hand-back at its edges (decisions D006-030 and D006-031): the minor that reserves the code
    /// and the one before it, the fourth withdrawal and the fifth, and each reason not to withdraw.
    #[test]
    fn a_hand_back_is_withdrawn_only_from_a_reserving_minor_and_below_the_cap() {
        let open = None;
        assert_eq!(hand_back(open, true, Some(8), true, 0), Ok(0));
        assert_eq!(
            hand_back(open, true, Some(9), true, 3),
            Ok(3),
            "the fourth withdrawal"
        );
        assert_eq!(
            hand_back(open, true, Some(8), true, 4),
            Err(WithdrawalOutcome::Capped),
            "the fifth"
        );
        assert_eq!(
            hand_back(open, true, Some(7), true, 0),
            Err(WithdrawalOutcome::Lapsed),
            "a minor that does not reserve the code"
        );
        assert_eq!(
            hand_back(open, true, None, true, 0),
            Err(WithdrawalOutcome::Lapsed),
            "no live session holds the lease"
        );
        assert_eq!(
            hand_back(open, false, Some(8), true, 0),
            Err(WithdrawalOutcome::FenceMissed),
            "the step started"
        );
        assert_eq!(
            hand_back(open, true, Some(8), false, 0),
            Err(WithdrawalOutcome::Lapsed),
            "the requirement is not recorded"
        );
        for gate in [WithdrawalOutcome::Lapsed, WithdrawalOutcome::Disabled] {
            assert_eq!(hand_back(Some(gate), true, Some(8), true, 0), Err(gate));
        }
    }

    /// Decision D006-089 at its edges: only a refusal for a reason of the fleet, from a session that reserves the code, of a
    /// lease still `LEASED`, lapses; the cap, a started step and an older minor stay the ordinary failure.
    #[test]
    fn a_hand_back_lapses_only_when_the_fleet_does_not_withdraw_for_a_reserving_session() {
        use HandBackVerdict::{Fail, Lapse, Withdraw};
        assert_eq!(
            hand_back_verdict(None, true, Some(8), 8, true, 0),
            Withdraw(0)
        );
        for gate in [WithdrawalOutcome::Lapsed, WithdrawalOutcome::Disabled] {
            assert_eq!(
                hand_back_verdict(Some(gate), true, Some(8), 8, true, 0),
                Lapse(gate)
            );
        }
        assert_eq!(
            hand_back_verdict(None, true, Some(8), 8, false, 0),
            Lapse(WithdrawalOutcome::Lapsed),
            "the requirement is not recorded"
        );
        assert_eq!(
            hand_back_verdict(None, true, Some(8), 8, true, 4),
            Fail(WithdrawalOutcome::Capped)
        );
        assert_eq!(
            hand_back_verdict(None, false, Some(8), 8, false, 0),
            Fail(WithdrawalOutcome::FenceMissed)
        );
        for minor in [Some(7), None] {
            assert_eq!(
                hand_back_verdict(Some(WithdrawalOutcome::Disabled), true, minor, 8, false, 0),
                Fail(WithdrawalOutcome::Lapsed),
                "{minor:?}: the code is not reserved below 1.8"
            );
        }
    }

    /// Decision D006-099 (request (10) of decision D006-059): a session stored at 1.8 under a build whose ceiling is 6 is judged
    /// at 6, so it is never withdrawn, and it lapses rather than spending the attempt; a session stored at 6 is the ordinary
    /// failure as before, and the ceiling that reserves the code still withdraws.
    #[test]
    fn a_hand_back_stored_at_1_8_that_the_build_cannot_honour_lapses() {
        use HandBackVerdict::{Fail, Lapse, Withdraw};
        assert_eq!(
            hand_back_verdict(None, true, Some(8), 6, true, 0),
            Lapse(WithdrawalOutcome::Lapsed),
            "stored 8, ceiling 6"
        );
        assert_eq!(
            hand_back_verdict(None, true, Some(9), 6, true, 4),
            Lapse(WithdrawalOutcome::Lapsed),
            "never withdrawn, whatever the count"
        );
        assert_eq!(
            hand_back_verdict(None, true, Some(6), 6, true, 0),
            Fail(WithdrawalOutcome::Lapsed),
            "stored 6: the code may be a handler's"
        );
        assert_eq!(
            hand_back_verdict(None, true, Some(8), 8, true, 0),
            Withdraw(0)
        );
        assert_eq!(
            hand_back_verdict(None, false, Some(8), 6, true, 0),
            Fail(WithdrawalOutcome::FenceMissed),
            "a started step stays the ordinary failure"
        );
        assert_eq!(
            hand_back_verdict(None, true, None, 6, true, 0),
            Fail(WithdrawalOutcome::Lapsed),
            "no live session holds the lease"
        );
    }

    #[test]
    fn the_outcomes_are_a_closed_set_of_labels() {
        let labels: Vec<&str> = [
            WithdrawalOutcome::Withdrawn,
            WithdrawalOutcome::FenceMissed,
            WithdrawalOutcome::Lapsed,
            WithdrawalOutcome::Capped,
            WithdrawalOutcome::SlotSpent,
            WithdrawalOutcome::Disabled,
            WithdrawalOutcome::Error,
        ]
        .iter()
        .map(|outcome| outcome.as_str())
        .collect();
        assert_eq!(
            labels,
            [
                "withdrawn",
                "fence_missed",
                "lapsed",
                "capped",
                "slot_spent",
                "disabled",
                "error"
            ]
        );
    }
}
