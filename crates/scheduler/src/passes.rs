use std::time::Duration;

use sagat_application::entropy::Entropy;
use sagat_application::execution::{
    Actor, Effects, Engine, EngineError, ExpressionEvaluator, RunTx,
};
use sagat_application::maintenance::{LeaseLoss, MaintenanceStore, RunRef, StepRef};
use sagat_application::runs::{Lock, RunStore};
use sagat_domain::execution::RunId;
use sagat_domain::execution::{self, ExecutionError, Expiry, Progress, StopReason};
use sagat_domain::identity::TenantId;
use time::OffsetDateTime;

/// Tunable limits of the passes.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerSettings {
    /// Candidates examined per pass per tick.
    pub batch: u32,
    /// A `READY` step with no notification for this long gets a fresh one.
    pub rehint_after: Duration,
    /// Idempotency records older than this are purged.
    pub idempotency_retention: Duration,
}

impl Default for SchedulerSettings {
    fn default() -> Self {
        Self {
            batch: 200,
            rehint_after: Duration::from_secs(30),
            idempotency_retention: Duration::from_secs(7 * 24 * 3600),
        }
    }
}

/// What one tick did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub timers_fired: u32,
    pub waits_timed_out: u32,
    pub retries_released: u32,
    pub leases_expired: u32,
    pub deadlines_enforced: u32,
    pub hints_renewed: u32,
    /// Candidates another scheduler or a caller had already handled.
    pub skipped: u32,
    /// Candidates that failed for a reason other than losing a race (logged).
    pub failed: u32,
}

impl Report {
    /// Whether anything at all changed.
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.timers_fired
            + self.waits_timed_out
            + self.retries_released
            + self.leases_expired
            + self.deadlines_enforced
            + self.hints_renewed
            > 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Done,
    Skipped,
    Failed,
}

/// The scheduler over a store, an evaluator and a source of jitter.
pub struct Scheduler<S, E, R> {
    store: S,
    engine: Engine<E>,
    entropy: R,
    settings: SchedulerSettings,
}

impl<S, E, R> std::fmt::Debug for Scheduler<S, E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

impl<S, E, R> Scheduler<S, E, R>
where
    S: RunStore + MaintenanceStore,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub const fn new(store: S, engine: Engine<E>, entropy: R, settings: SchedulerSettings) -> Self {
        Self {
            store,
            engine,
            entropy,
            settings,
        }
    }

    /// Runs every pass once.
    pub async fn tick(&self) -> Report {
        let mut report = Report::default();
        self.timers(&mut report).await;
        self.wait_deadlines(&mut report).await;
        self.retries(&mut report).await;
        self.leases(&mut report).await;
        self.deadlines(&mut report).await;
        self.hints(&mut report).await;
        report
    }

    /// Purges expired idempotency records and worker sessions. Run it rarely.
    pub async fn housekeeping(&self) {
        match self
            .store
            .purge_idempotency(self.settings.idempotency_retention)
            .await
        {
            Ok(n) if n > 0 => tracing::info!(removed = n, "purged idempotency records"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not purge idempotency records"),
        }
        match self.store.purge_sessions().await {
            Ok(n) if n > 0 => tracing::info!(removed = n, "purged expired worker sessions"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not purge worker sessions"),
        }
    }

    /// Applies `command` to a run under its row lock and commits.
    async fn apply<F>(
        &self,
        tenant: TenantId,
        run: RunId,
        kind: &'static str,
        effects: Effects,
        command: F,
    ) -> Outcome
    where
        F: FnOnce(
            &sagat_domain::workflow::WorkflowDefinition,
            &mut sagat_domain::execution::RunState,
            OffsetDateTime,
        ) -> Result<((), Progress), ExecutionError>,
    {
        let outcome = async {
            let Some(mut tx) = self
                .store
                .begin(tenant, run, Lock::Wait)
                .await
                .map_err(EngineError::Store)?
            else {
                return Ok(Outcome::Skipped);
            };
            match self.engine.run(&mut tx, effects, command).await {
                Ok(()) => {
                    tx.commit().await.map_err(EngineError::Store)?;
                    Ok(Outcome::Done)
                }
                // The domain refused: the state moved since the candidate query (a worker
                // answered, a signal arrived, the run was cancelled). Dropping `tx` rolls back.
                Err(EngineError::Execution(error))
                    if !matches!(error, ExecutionError::Invariant(_)) =>
                {
                    Ok(Outcome::Skipped)
                }
                Err(other) => Err(other),
            }
        }
        .await;
        let outcome = outcome.unwrap_or_else(|error: EngineError| {
            tracing::error!(%error, %run, kind, "a scheduler action failed");
            Outcome::Failed
        });
        metrics::counter!("scheduler_actions_total", "kind" => kind, "outcome" => match outcome {
            Outcome::Done => "done",
            Outcome::Skipped => "skipped",
            Outcome::Failed => "failed",
        })
        .increment(1);
        outcome
    }

    fn effects(&self) -> Effects {
        Effects::by(Actor::System("scheduler"), None)
    }

    fn tally(report: &mut Report, outcome: Outcome, done: impl FnOnce(&mut Report)) {
        match outcome {
            Outcome::Done => done(report),
            Outcome::Skipped => report.skipped += 1,
            Outcome::Failed => report.failed += 1,
        }
    }

    async fn candidates<T>(
        result: Result<Vec<T>, sagat_application::error::StoreError>,
        what: &'static str,
        report: &mut Report,
    ) -> Vec<T> {
        result.unwrap_or_else(|error| {
            tracing::warn!(%error, what, "a scheduler query failed");
            report.failed += 1;
            Vec::new()
        })
    }

    async fn timers(&self, report: &mut Report) {
        let due = Self::candidates(
            self.store.due_timers(self.settings.batch).await,
            "due timers",
            report,
        )
        .await;
        for StepRef {
            tenant_id,
            run_id,
            node,
        } in due
        {
            let outcome = self
                .apply(
                    tenant_id,
                    run_id,
                    "timer",
                    self.effects(),
                    move |def, state, now| Ok(((), execution::fire_timer(def, state, &node, now)?)),
                )
                .await;
            Self::tally(report, outcome, |r| r.timers_fired += 1);
        }
    }

    async fn wait_deadlines(&self, report: &mut Report) {
        let due = Self::candidates(
            self.store.due_wait_deadlines(self.settings.batch).await,
            "wait deadlines",
            report,
        )
        .await;
        for StepRef {
            tenant_id,
            run_id,
            node,
        } in due
        {
            let outcome = self
                .apply(
                    tenant_id,
                    run_id,
                    "wait_timeout",
                    self.effects(),
                    move |def, state, now| {
                        Ok(((), execution::time_out_wait(def, state, &node, now)?))
                    },
                )
                .await;
            Self::tally(report, outcome, |r| r.waits_timed_out += 1);
        }
    }

    async fn retries(&self, report: &mut Report) {
        let due = Self::candidates(
            self.store.due_retries(self.settings.batch).await,
            "due retries",
            report,
        )
        .await;
        for StepRef {
            tenant_id,
            run_id,
            node,
        } in due
        {
            let outcome = self
                .apply(
                    tenant_id,
                    run_id,
                    "retry",
                    self.effects(),
                    move |def, state, now| Ok(((), execution::retry_due(def, state, &node, now)?)),
                )
                .await;
            Self::tally(report, outcome, |r| r.retries_released += 1);
        }
    }

    async fn leases(&self, report: &mut Report) {
        let lost = Self::candidates(
            self.store.lost_leases(self.settings.batch).await,
            "lost leases",
            report,
        )
        .await;
        for lost in lost {
            let jitter = self.entropy.unit();
            let cause = match lost.cause {
                LeaseLoss::Expired => Expiry::LeaseExpired,
                LeaseLoss::TimedOut => Expiry::AttemptTimeout,
            };
            let (node, fence) = (lost.step.node.clone(), lost.fence);
            let outcome = self
                .apply(
                    lost.step.tenant_id,
                    lost.step.run_id,
                    "lease",
                    self.effects(),
                    move |def, state, now| {
                        Ok((
                            (),
                            execution::expire_attempt(
                                def, state, &node, fence, cause, jitter, now,
                            )?,
                        ))
                    },
                )
                .await;
            Self::tally(report, outcome, |r| r.leases_expired += 1);
        }
    }

    async fn deadlines(&self, report: &mut Report) {
        let overdue = Self::candidates(
            self.store.overdue_runs(self.settings.batch).await,
            "overdue runs",
            report,
        )
        .await;
        for RunRef { tenant_id, run_id } in overdue {
            let outcome = self
                .apply(
                    tenant_id,
                    run_id,
                    "deadline",
                    self.effects(),
                    |def, state, now| {
                        Ok((
                            (),
                            execution::stop_run(def, state, StopReason::TimedOut, None, now)?,
                        ))
                    },
                )
                .await;
            Self::tally(report, outcome, |r| r.deadlines_enforced += 1);
        }
    }

    async fn hints(&self, report: &mut Report) {
        let stale = Self::candidates(
            self.store
                .stale_hints(self.settings.rehint_after, self.settings.batch)
                .await,
            "stale hints",
            report,
        )
        .await;
        for StepRef {
            tenant_id,
            run_id,
            node,
        } in stale
        {
            let mut effects = self.effects();
            effects.rehint = vec![node];
            let outcome = self
                .apply(tenant_id, run_id, "rehint", effects, |_, _, _| {
                    Ok((
                        (),
                        Progress {
                            events: Vec::new(),
                            pending: None,
                        },
                    ))
                })
                .await;
            Self::tally(report, outcome, |r| r.hints_renewed += 1);
        }
    }
}
