//! The withdrawal of leases that never reached a handler (feature 006, decisions D006-001 to D006-018 of
//! `specs/006-sdk-multilanguage/research.md`; ADR-0098).
//!
//! A lease is withdrawn only while the platform requires `lease-withdrawal-v1` of control planes and engines (the gate
//! keeps the recorded requirements), in one fenced transaction per run (root leases) or per run and instance path (scoped
//! leases), and only while its step is still `LEASED` with the generation and the lease-token hash the server issued.
//! Anything else is counted and changes nothing: the lease lapses as it did before.
use std::collections::BTreeMap;

use sagat_domain::execution::{InstancePath, WithdrawalCause};

use super::*;
use crate::instances::{Capability, InstanceKind, ServingGate};

/// The code a host reports through `FailStep` for a lease it received and will not run (`sdks/SPEC.md` §6.2, decision
/// D006-005).
pub const TASK_NOT_STARTED: &str = "TASK_NOT_STARTED";

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
    /// The platform does not require `lease-withdrawal-v1` yet: the lease lapses as before.
    Lapsed,
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
            Self::Error => "error",
        }
    }
}

fn count(cause: WithdrawalCause, outcome: WithdrawalOutcome, n: u64) {
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

    /// Whether the platform requires `lease-withdrawal-v1` of every control plane and engine, as this process last read.
    #[must_use]
    pub fn withdraws_leases(&self) -> bool {
        self.lease_withdrawal
            .as_ref()
            .is_some_and(|gate| gate.requires(Capability::LeaseWithdrawalV1, &InstanceKind::ALL))
    }
}

impl<S, C, E, R> WorkerService<S, C, E, R>
where
    S: RunStore + WorkerStore + DefinitionStore + PathExecutionStore,
    C: Clock,
    E: ExpressionEvaluator,
    R: Entropy,
{
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
        if !self.withdraws_leases() {
            count(cause, WithdrawalOutcome::Lapsed, total as u64);
            return vec![WithdrawalOutcome::Lapsed; total];
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
        let mut outcomes = Vec::with_capacity(leases.len());
        let mut withdrawn = false;
        for lease in leases {
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
            let node = lease.instance.node().clone();
            let fence = lease.fence;
            let effects = Effects::by(Actor::System("lease-withdrawal"), None);
            match self
                .engine
                .run(&mut tx, effects, move |def, state, now| {
                    Ok((
                        (),
                        execution::withdraw_lease(def, state, &node, fence, cause, now)?,
                    ))
                })
                .await
            {
                Ok(()) => {
                    withdrawn = true;
                    outcomes.push(WithdrawalOutcome::Withdrawn);
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
            let snapshot = tx.snapshot();
            match execution::withdraw_path_instance(
                &snapshot.definition,
                &mut snapshot.state,
                &lease.instance,
                lease.fence,
                cause,
            ) {
                Ok(progress) => {
                    let effects = Effects::by(Actor::System("lease-withdrawal"), None);
                    tx.persist_addressed(&progress.events, &[], effects).await?;
                    withdrawn = true;
                    outcomes.push(WithdrawalOutcome::Withdrawn);
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
