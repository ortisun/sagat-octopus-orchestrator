use std::time::Duration;

use sagat_application::definitions::{Applied, DefinitionStore, Idempotency};
use sagat_application::entropy::Entropy;
use sagat_application::error::StoreError;
use sagat_application::execution::{
    Actor, DefinitionCache, Effects, Engine, EngineError, ExpressionEvaluator, ParentLink, RunInfo,
    RunTx,
};
use sagat_application::maintenance::{
    EndedChild, LeaseLoss, MaintenanceStore, PendingChild, RunRef, StepRef,
};
use sagat_application::quotas::RunLimits;
use sagat_application::runs::{Lock, NewRun, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::RunId;
use sagat_domain::execution::{
    self, ChildOutcome, ExecutionError, Expiry, FailureCode, Progress, RunStatus, StopReason,
};
use sagat_domain::identity::TenantId;
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

/// Delivered outbox messages deleted per housekeeping round.
const PURGE_BATCH: u32 = 5_000;
/// Batches one housekeeping call may purge (about 100 000 messages).
const PURGE_ROUNDS: u32 = 20;

/// Tunable limits of the passes.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerSettings {
    /// Candidates examined per pass per tick.
    pub batch: u32,
    /// A `READY` step with no notification for this long gets a fresh one.
    pub rehint_after: Duration,
    /// Idempotency records older than this are purged.
    pub idempotency_retention: Duration,
    /// Delivered outbox messages older than this are purged.
    pub outbox_retention: Duration,
}

impl Default for SchedulerSettings {
    fn default() -> Self {
        Self {
            batch: 200,
            rehint_after: Duration::from_secs(30),
            idempotency_retention: Duration::from_secs(7 * 24 * 3600),
            outbox_retention: Duration::from_secs(24 * 3600),
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
    pub children_started: u32,
    pub children_finished: u32,
    pub children_cancelled: u32,
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
            + self.children_started
            + self.children_finished
            + self.children_cancelled
            > 0
    }
}

/// What must still be true under the run's lock for a scheduler action to proceed.
enum Condition<'a> {
    Always,
    /// The attempt on `node` is still over: its lease expired (or its own deadline passed) *now*, by
    /// the database clock, not merely when the candidate query ran.
    LeaseLost {
        node: &'a sagat_domain::workflow::NodeId,
        cause: LeaseLoss,
    },
}

impl Condition<'_> {
    async fn holds<Tx: RunTx>(&self, tx: &mut Tx) -> Result<bool, StoreError> {
        match self {
            Self::Always => Ok(true),
            Self::LeaseLost { node, cause } => {
                let now = tx.now();
                Ok(tx
                    .lease_deadlines(node)
                    .await?
                    .is_some_and(|d| match cause {
                        LeaseLoss::Expired => d.expires_at <= now,
                        LeaseLoss::TimedOut => d.attempt_deadline_at.is_some_and(|at| at <= now),
                    }))
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Needed to start child runs (it parses the child workflow's definition); without it the
    /// scheduler leaves `child` steps alone.
    definitions: Option<DefinitionCache>,
}

impl<S, E, R> std::fmt::Debug for Scheduler<S, E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

impl<S, E, R> Scheduler<S, E, R>
where
    S: RunStore + MaintenanceStore + DefinitionStore,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub const fn new(store: S, engine: Engine<E>, entropy: R, settings: SchedulerSettings) -> Self {
        Self {
            store,
            engine,
            entropy,
            settings,
            definitions: None,
        }
    }

    /// Lets the scheduler start the child runs that `child` steps reserve.
    #[must_use]
    pub fn with_children(mut self, definitions: DefinitionCache) -> Self {
        self.definitions = Some(definitions);
        self
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
        self.children_start(&mut report).await;
        self.children_finish(&mut report).await;
        self.children_cancel(&mut report).await;
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
        // The records scripts keep for a while (executions, test runs) end with their retention.
        let mut scripts = 0u64;
        for _ in 0..PURGE_ROUNDS {
            match self.store.purge_script_records(PURGE_BATCH).await {
                Ok(n) => {
                    scripts += n;
                    if n < u64::from(PURGE_BATCH) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not purge script records");
                    break;
                }
            }
        }
        if scripts > 0 {
            tracing::info!(removed = scripts, "purged script records past their retention");
        }
        // One bounded statement at a time, so a backlog never holds a long one, repeated until the
        // backlog is gone (or this call has done its share): the purge must outrun what is written.
        let mut removed = 0u64;
        for _ in 0..PURGE_ROUNDS {
            match self
                .store
                .purge_outbox(self.settings.outbox_retention, PURGE_BATCH)
                .await
            {
                Ok(n) => {
                    removed += n;
                    if n < u64::from(PURGE_BATCH) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not purge the outbox");
                    break;
                }
            }
        }
        if removed > 0 {
            tracing::info!(removed, "purged delivered outbox messages");
        }
    }

    /// Applies `command` to a run under its row lock and commits, provided `condition` still holds
    /// there. The candidate queries read without a lock, so what made a run a candidate may have
    /// changed since (a heartbeat renewed the lease, a retry was already released): the check
    /// is repeated under the lock, and a candidate that no longer qualifies is skipped.
    async fn apply_if<F>(
        &self,
        tenant: TenantId,
        run: RunId,
        kind: &'static str,
        condition: Condition<'_>,
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
            if !condition.holds(&mut tx).await.map_err(EngineError::Store)? {
                return Ok(Outcome::Skipped);
            }
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

    /// [`Self::apply_if`] with nothing to re-check: the domain command itself refuses what is stale.
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
        self.apply_if(tenant, run, kind, Condition::Always, effects, command)
            .await
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
                .apply_if(
                    lost.step.tenant_id,
                    lost.step.run_id,
                    "lease",
                    Condition::LeaseLost {
                        node: &lost.step.node,
                        cause: lost.cause,
                    },
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
    // ------------------------------------------------------------------ child runs (FR-002-009)

    /// Creates the child runs that `child` steps reserved. Each is created, started and its
    /// reservation claimed in one transaction under the parent's lock, so a child exists exactly
    /// when its parent step was still waiting for it.
    async fn children_start(&self, report: &mut Report) {
        let Some(definitions) = &self.definitions else {
            return;
        };
        let pending = Self::candidates(
            self.store.pending_children(self.settings.batch).await,
            "pending children",
            report,
        )
        .await;
        for child in pending {
            let outcome = match self.start_child(definitions, &child).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    tracing::error!(%error, run = %child.step.run_id, node = %child.step.node, "starting a child run failed");
                    Outcome::Failed
                }
            };
            metrics::counter!("scheduler_actions_total", "kind" => "child_start", "outcome" => match outcome {
                Outcome::Done => "done",
                Outcome::Skipped => "skipped",
                Outcome::Failed => "failed",
            })
            .increment(1);
            Self::tally(report, outcome, |r| r.children_started += 1);
        }
    }

    async fn start_child(
        &self,
        definitions: &DefinitionCache,
        pending: &PendingChild,
    ) -> Result<Outcome, EngineError> {
        let tenant = pending.step.tenant_id;
        let parent_id = pending.step.run_id;
        let node = &pending.step.node;
        let Some(parent) = self.store.get(tenant, parent_id).await? else {
            return Ok(Outcome::Skipped);
        };
        let Some(parent_version) = self
            .store
            .get_version_by_id(tenant, parent.info.version_id)
            .await?
        else {
            return Ok(Outcome::Skipped);
        };
        let parent_definition = definitions
            .get_or_parse(parent.info.version_id, &parent_version.document)
            .map_err(|_| StoreError::Internal {
                detail: "a stored version no longer parses".into(),
            })?;
        let Some(parent_node) = parent_definition.node(node) else {
            return Ok(Outcome::Skipped);
        };
        let NodeKind::Child {
            workflow,
            version,
            cancel_policy,
            ..
        } = parent_node.kind()
        else {
            return Ok(Outcome::Skipped);
        };

        // The child workflow lives in the parent's namespace; the lookup is scoped by tenant and
        // namespace, so a same-named workflow elsewhere can never be reached.
        let target = match self
            .store
            .find_workflow_by_name(tenant, parent.info.namespace_id, workflow)
            .await?
        {
            Some(record) => {
                let version_record = match version {
                    Some(number) => {
                        self.store
                            .get_version(tenant, record.workflow.id(), *number)
                            .await?
                    }
                    None => match record.workflow.active_version() {
                        Some(id) => self.store.get_version_by_id(tenant, id).await?,
                        None => None,
                    },
                };
                version_record.map(|v| (record, v))
            }
            None => None,
        };
        let Some((workflow_record, version_record)) = target else {
            // The child cannot exist: tell the parent step now instead of waiting forever.
            return Ok(self
                .finish_with(
                    tenant,
                    parent_id,
                    node.clone(),
                    pending.child,
                    ChildOutcome::Failed(FailureCode::child_not_found()),
                )
                .await);
        };
        let definition = definitions
            .get_or_parse(version_record.version.id(), &version_record.document)
            .map_err(|_| StoreError::Internal {
                detail: "a stored version no longer parses".into(),
            })?;

        // A child never outlives its parent's deadline, and its own timeout can only shorten it.
        let own = parent_node
            .timeout()
            .and_then(|t| time::Duration::try_from(t).ok())
            .and_then(|t| pending.as_of.checked_add(t));
        let deadline_at = match (parent.info.deadline_at, own) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let child_id = pending.child;
        let info = RunInfo {
            run_id: child_id,
            tenant_id: tenant,
            namespace_id: parent.info.namespace_id,
            workflow_id: workflow_record.workflow.id(),
            version_id: version_record.version.id(),
            started_by: sagat_domain::identity::Subject::engine()
                .as_str()
                .to_owned(),
            correlation_id: parent.info.correlation_id.clone(),
            trace_id: parent.info.trace_id.clone(),
            deadline_at,
            created_at: pending.as_of,
            parent: Some(ParentLink {
                run: parent_id,
                node: node.clone(),
                cancel_policy: *cancel_policy,
            }),
            depth: parent.info.depth.saturating_add(1),
            root: parent.info.root,
        };
        let audit = AuditRecord::engine(
            tenant,
            AuditAction::RunStarted,
            ResourceType::Run,
            child_id.to_string(),
            format!("child of run {parent_id}, node {node}"),
            parent.info.trace_id.clone(),
            pending.as_of,
        );
        let limits = RunLimits::of(&self.store, tenant, parent.info.namespace_id).await?;
        let new = NewRun {
            limits,
            info,
            definition: std::sync::Arc::clone(&definition),
            input: pending.input.clone(),
            audit,
            idempotency: Idempotency {
                scope: format!("child:{parent_id}"),
                // (The dedupe table requires 8 to 160 characters, like an `Idempotency-Key`.)
                key: format!("child-start:{node}"),
                fingerprint: child_id.as_uuid().as_bytes().to_vec(),
            },
        };
        match self.store.create(new).await {
            Ok(Applied::Replayed(_)) => Ok(Outcome::Skipped),
            // The parent step no longer waits for this run (cancelled, or created by a peer).
            Err(StoreError::MissingReference) => Ok(Outcome::Skipped),
            // Either way the step fails instead of waiting. Waiting for room would leave the
            // blocked reservations at the head of the oldest-first queue, where a full namespace of
            // one tenant would keep every other tenant's children from starting (G32, feature
            // 010); the failure code lets the workflow tell the two apart and route on it.
            Err(StoreError::QuotaExceeded { what }) => Ok({
                let code = if what == sagat_application::execution::TREE_LIMIT {
                    // The tree is as large as a tree may get.
                    FailureCode::child_limit_exceeded()
                } else {
                    // The namespace or the tenant is at its quota of active runs.
                    FailureCode::child_quota_exceeded()
                };
                self.finish_with(
                    tenant,
                    parent_id,
                    node.clone(),
                    pending.child,
                    ChildOutcome::Failed(code),
                )
                .await
            }),
            Err(other) => Err(other.into()),
            Ok(Applied::Done(mut tx)) => {
                let effects = Effects::by(Actor::System("scheduler"), parent.info.trace_id.clone());
                self.engine
                    .run(&mut tx, effects, |def: &WorkflowDefinition, state, now| {
                        let (started, progress) = execution::start_run(def, child_id, now)?;
                        *state = started;
                        Ok(((), progress))
                    })
                    .await?;
                tx.commit().await?;
                metrics::counter!("child_runs_started_total").increment(1);
                Ok(Outcome::Done)
            }
        }
    }

    /// Reports a child's outcome to the parent step that waits for it.
    async fn finish_with(
        &self,
        tenant: TenantId,
        parent: RunId,
        node: sagat_domain::workflow::NodeId,
        child: RunId,
        outcome: ChildOutcome,
    ) -> Outcome {
        self.apply(
            tenant,
            parent,
            "child_finish",
            self.effects(),
            move |def, state, now| {
                Ok((
                    (),
                    execution::finish_child(def, state, &node, child, outcome, now)?,
                ))
            },
        )
        .await
    }

    /// Tells parents that the child they wait for has ended.
    async fn children_finish(&self, report: &mut Report) {
        let ended = Self::candidates(
            self.store.ended_children(self.settings.batch).await,
            "ended children",
            report,
        )
        .await;
        for EndedChild {
            tenant_id,
            child,
            parent,
            node,
            status,
        } in ended
        {
            // Every way a run can end, and nothing else: a status added later must be mapped here
            // (a run that ended in a way nobody reports would keep its parent waiting for ever, and
            // sit at the head of the list of ended children).
            let outcome = match status {
                RunStatus::Succeeded => ChildOutcome::Succeeded,
                RunStatus::Failed => ChildOutcome::Failed(FailureCode::child_failed()),
                RunStatus::Cancelled => ChildOutcome::Failed(FailureCode::child_cancelled()),
                RunStatus::TimedOut => ChildOutcome::Failed(FailureCode::child_timed_out()),
                RunStatus::Compensated => ChildOutcome::Failed(FailureCode::child_compensated()),
                RunStatus::Partial => ChildOutcome::Failed(FailureCode::child_partial()),
                RunStatus::CompensationFailed => {
                    ChildOutcome::Failed(FailureCode::child_compensation_failed())
                }
                RunStatus::Running
                | RunStatus::Waiting
                | RunStatus::Paused
                | RunStatus::Cancelling
                | RunStatus::Compensating
                | RunStatus::ManualReview => continue,
            };
            let result = self
                .finish_with(tenant_id, parent, node.clone(), child, outcome)
                .await;
            if result == Outcome::Skipped {
                tracing::warn!(%parent, %node, %child, "an ended child could not be reported to its parent step");
            }
            Self::tally(report, result, |r| r.children_finished += 1);
        }
    }

    /// Stops the running children of a parent that was stopped under a policy that cancels them.
    async fn children_cancel(&self, report: &mut Report) {
        let doomed = Self::candidates(
            self.store.children_to_cancel(self.settings.batch).await,
            "children to cancel",
            report,
        )
        .await;
        for RunRef { tenant_id, run_id } in doomed {
            let outcome = self
                .apply(
                    tenant_id,
                    run_id,
                    "child_cancel",
                    self.effects(),
                    |def, state, now| {
                        Ok((
                            (),
                            execution::stop_run(def, state, StopReason::Cancelled, None, now)?,
                        ))
                    },
                )
                .await;
            Self::tally(report, outcome, |r| r.children_cancelled += 1);
        }
    }
}
