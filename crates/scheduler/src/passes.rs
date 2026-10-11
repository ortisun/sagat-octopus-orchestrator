use std::future::Future;
use std::hash::Hash;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use sagat_application::definitions::{Applied, DefinitionStore, Idempotency};
use sagat_application::entropy::Entropy;
use sagat_application::error::StoreError;
use sagat_application::execution::{
    Actor, DefinitionCache, Effects, Engine, EngineError, ExpressionEvaluator, ParentLink, RunInfo,
    RunTx,
};
use sagat_application::maintenance::{
    EndedChild, LeaseLoss, MaintenanceStore, PendingChild, RunRef, StepRef, Window,
};
use sagat_application::quotas::RunLimits;
use sagat_application::runs::{Lock, NewRun, RunStore};
use sagat_application::shards::Lease;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::execution::RunId;
use sagat_domain::execution::{
    self, ChildOutcome, ExecutionError, Expiry, FailureCode, Progress, RunStatus, StopReason,
};
use sagat_domain::identity::TenantId;
use sagat_domain::sharding::{Shard, ShardSet};
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use time::OffsetDateTime;
use tracing::Instrument;

use crate::fairness::{self, Weights};
use crate::shards::{Placement, Snapshot};

/// Delivered outbox messages deleted per housekeeping round.
const PURGE_BATCH: u32 = 5_000;
/// Batches one housekeeping call may purge (about 100 000 messages).
const PURGE_ROUNDS: u32 = 20;

/// Tunable limits of the passes.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerSettings {
    /// Candidates examined per pass per tick.
    pub batch: u32,
    /// Whether a pass takes its candidates fairly between tenants (the default). Off, it takes the
    /// oldest `batch` of the shards it holds, as before feature 010: a diagnostic, to measure what
    /// fairness does (`tests/performance/scalability-ha`), not a mode to run in.
    pub fair: bool,
    /// Candidates of one pass that are acted on at the same time (each in its own transaction).
    /// One at a time costs a round trip of latency per candidate; this is what lets a replica
    /// fire the timers of a burst at the rate the database allows instead (`FR-010-002`).
    pub concurrency: usize,
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
            fair: true,
            concurrency: 8,
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
    /// Candidates of a shard the replica no longer holds (`FR-010-001`): another replica's now.
    pub fenced: u32,
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
    /// The replica does not hold the run's shard any more.
    Fenced,
}

impl Outcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Fenced => "fenced",
        }
    }
}

/// What a tick works on: the shards it may look at, the weights of the tenants, and the snapshot
/// whose leases fence every action. A scheduler with no placement is the only replica: it looks at
/// every shard and fences nothing.
struct View {
    shards: ShardSet,
    weights: Weights,
    snapshot: Option<std::sync::Arc<Snapshot>>,
    owner: String,
}

/// What an action on a run must hold.
enum Hold {
    /// Nothing: the scheduler is the only replica.
    Nothing,
    /// The lease on the run's shard, checked in the action's transaction.
    Lease(Lease),
    /// The replica does not hold the run's shard (any more): the action is another replica's.
    Lost,
}

impl View {
    fn hold_for(&self, run: RunId) -> Hold {
        match &self.snapshot {
            None => Hold::Nothing,
            Some(snapshot) => snapshot
                .lease_of(Shard::of(run.as_uuid()), Instant::now())
                .map_or(Hold::Lost, Hold::Lease),
        }
    }

    /// The fence: inside the transaction that changes the run, the shard is still held at the
    /// generation the replica holds. `true` when the action may go on.
    async fn holds<Tx: RunTx>(&self, tx: &mut Tx, hold: &Hold) -> Result<bool, StoreError> {
        match hold {
            Hold::Nothing => Ok(true),
            Hold::Lease(lease) => tx.holds_shard(&self.owner, *lease).await,
            Hold::Lost => Ok(false),
        }
    }
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
    /// Which shards this replica works on; without it the scheduler is the only replica.
    placement: Option<Placement>,
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
            placement: None,
        }
    }

    /// Makes the scheduler one of several replicas: it works only on the shards the placement says
    /// it holds, and every action is fenced by the lease it holds on the run's shard.
    #[must_use]
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = Some(placement);
        self
    }

    fn view(&self) -> View {
        match &self.placement {
            None => View {
                shards: ShardSet::all(),
                weights: Weights::default(),
                snapshot: None,
                owner: String::new(),
            },
            Some(placement) => {
                let snapshot = placement.snapshot();
                View {
                    shards: snapshot.shards(Instant::now()),
                    weights: snapshot.weights().clone(),
                    owner: placement.owner().to_owned(),
                    snapshot: Some(snapshot),
                }
            }
        }
    }

    /// Lets the scheduler start the child runs that `child` steps reserve.
    #[must_use]
    pub fn with_children(mut self, definitions: DefinitionCache) -> Self {
        self.definitions = Some(definitions);
        self
    }

    /// Runs every pass once, over the shards this replica holds (all of them when it is alone).
    pub async fn tick(&self) -> Report {
        let mut report = Report::default();
        let view = self.view();
        if view.shards.is_empty() {
            // A replica that holds nothing (it has not got its share yet, or it cannot reach the
            // database) has nothing to look at; the keeper is what changes that.
            return report;
        }
        self.timers(&view, &mut report).await;
        self.wait_deadlines(&view, &mut report).await;
        self.retries(&view, &mut report).await;
        self.leases(&view, &mut report).await;
        self.deadlines(&view, &mut report).await;
        self.hints(&view, &mut report).await;
        self.children_start(&view, &mut report).await;
        self.children_finish(&view, &mut report).await;
        self.children_cancel(&view, &mut report).await;
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
            tracing::info!(
                removed = scripts,
                "purged script records past their retention"
            );
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
        view: &View,
        RunRef {
            tenant_id: tenant,
            run_id: run,
        }: RunRef,
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
        let hold = view.hold_for(run);
        let outcome = async {
            if matches!(hold, Hold::Lost) {
                // The shard changed hands after the candidate query: do not even open a transaction.
                return Ok(Outcome::Fenced);
            }
            let Some(mut tx) = self
                .store
                .begin(tenant, run, Lock::Wait)
                .await
                .map_err(EngineError::Store)?
            else {
                return Ok(Outcome::Skipped);
            };
            // The work belongs to the trace of the run it acts on, under the run's span (D162):
            // what the scheduler did to a run is read next to what the run did.
            let trace = tx.info().trace_id.clone();
            let span = tracing::info_span!(
                "scheduler.action",
                kind,
                run_id = %run,
                tenant_id = %tenant,
                trace_id = trace.as_deref(),
                parent_span_id = %sagat_telemetry::run_span_id(run.as_uuid()),
            );
            async {
                // The fence comes first: whatever the run's state says, it is not this replica's to
                // change unless it still holds the shard at the generation it believes it does.
                if !view
                    .holds(&mut tx, &hold)
                    .await
                    .map_err(EngineError::Store)?
                {
                    return Ok(Outcome::Fenced);
                }
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
            .instrument(span)
            .await
        }
        .await;
        let outcome = outcome.unwrap_or_else(|error: EngineError| {
            tracing::error!(%error, %run, kind, "a scheduler action failed");
            Outcome::Failed
        });
        metrics::counter!("scheduler_actions_total", "kind" => kind, "outcome" => outcome.label())
            .increment(1);
        if outcome == Outcome::Fenced {
            metrics::counter!("scheduler_fenced_total", "kind" => kind).increment(1);
        }
        outcome
    }

    /// [`Self::apply_if`] with nothing to re-check: the domain command itself refuses what is stale.
    async fn apply<F>(
        &self,
        view: &View,
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
        self.apply_if(
            view,
            RunRef {
                tenant_id: tenant,
                run_id: run,
            },
            kind,
            Condition::Always,
            effects,
            command,
        )
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
            Outcome::Fenced => report.fenced += 1,
        }
    }

    /// Reads the candidates of a pass: the oldest, taken fairly between the tenants (`FR-010-002`),
    /// from the shards this replica holds. A query that fails is logged and the pass has nothing to
    /// do this tick.
    async fn pick<'a, T, K, Fut>(
        &'a self,
        view: &View,
        what: &'static str,
        report: &mut Report,
        tenant: impl Fn(&T) -> TenantId,
        key: impl Fn(&T) -> K,
        query: impl Fn(&'a S, Window) -> Fut,
    ) -> Vec<T>
    where
        K: Eq + Hash,
        Fut: Future<Output = Result<Vec<T>, StoreError>>,
    {
        let (shards, limit, store) = (view.shards, self.settings.batch, &self.store);
        if !self.settings.fair {
            let oldest = query(
                store,
                Window {
                    shards,
                    exclude: Vec::new(),
                    limit,
                },
            )
            .await;
            return oldest.unwrap_or_else(|error| {
                tracing::warn!(%error, what, "a scheduler query failed");
                report.failed += 1;
                Vec::new()
            });
        }
        let selected = fairness::select(limit, &view.weights, tenant, key, |exclude| {
            query(
                store,
                Window {
                    shards,
                    exclude,
                    limit,
                },
            )
        })
        .await;
        match selected {
            Ok((items, selection)) => {
                if selection.limited > 0 {
                    metrics::counter!("scheduler_fairness_limited_total", "pass" => what)
                        .increment(selection.limited as u64);
                }
                if selection.exhausted {
                    metrics::counter!("scheduler_fairness_exhausted_total", "pass" => what)
                        .increment(1);
                }
                items
            }
            Err(error) => {
                tracing::warn!(%error, what, "a scheduler query failed");
                report.failed += 1;
                Vec::new()
            }
        }
    }

    /// Acts on candidates, [`SchedulerSettings::concurrency`] at a time.
    async fn each<T, O, Fut>(&self, items: Vec<T>, work: impl FnMut(T) -> Fut) -> Vec<O>
    where
        Fut: Future<Output = O>,
    {
        stream::iter(items)
            .map(work)
            .buffer_unordered(self.settings.concurrency.max(1))
            .collect()
            .await
    }

    async fn timers(&self, view: &View, report: &mut Report) {
        let due = self
            .pick(
                view,
                "timers",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.node.clone()),
                |store, window| async move { store.due_timers(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                due,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                 }| {
                    self.apply(
                        view,
                        tenant_id,
                        run_id,
                        "timer",
                        self.effects(),
                        move |def, state, now| {
                            Ok(((), execution::fire_timer(def, state, &node, now)?))
                        },
                    )
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.timers_fired += 1);
        }
    }

    async fn wait_deadlines(&self, view: &View, report: &mut Report) {
        let due = self
            .pick(
                view,
                "wait_deadlines",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.node.clone()),
                |store, window| async move { store.due_wait_deadlines(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                due,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                 }| {
                    self.apply(
                        view,
                        tenant_id,
                        run_id,
                        "wait_timeout",
                        self.effects(),
                        move |def, state, now| {
                            Ok(((), execution::time_out_wait(def, state, &node, now)?))
                        },
                    )
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.waits_timed_out += 1);
        }
    }

    async fn retries(&self, view: &View, report: &mut Report) {
        let due = self
            .pick(
                view,
                "retries",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.node.clone()),
                |store, window| async move { store.due_retries(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                due,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                 }| {
                    self.apply(
                        view,
                        tenant_id,
                        run_id,
                        "retry",
                        self.effects(),
                        move |def, state, now| {
                            Ok(((), execution::retry_due(def, state, &node, now)?))
                        },
                    )
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.retries_released += 1);
        }
    }

    async fn leases(&self, view: &View, report: &mut Report) {
        let lost = self
            .pick(
                view,
                "leases",
                report,
                |l: &sagat_application::maintenance::LostLease| l.step.tenant_id,
                |l| (l.step.run_id, l.step.node.clone()),
                |store, window| async move { store.lost_leases(&window).await },
            )
            .await;
        let outcomes = self
            .each(lost, |lost| async move {
                let jitter = self.entropy.unit();
                let cause = match lost.cause {
                    LeaseLoss::Expired => Expiry::LeaseExpired,
                    LeaseLoss::TimedOut => Expiry::AttemptTimeout,
                };
                let (node, fence) = (lost.step.node.clone(), lost.fence);
                self.apply_if(
                    view,
                    RunRef {
                        tenant_id: lost.step.tenant_id,
                        run_id: lost.step.run_id,
                    },
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
                .await
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.leases_expired += 1);
        }
    }

    async fn deadlines(&self, view: &View, report: &mut Report) {
        let overdue = self
            .pick(
                view,
                "deadlines",
                report,
                |r: &RunRef| r.tenant_id,
                |r| r.run_id,
                |store, window| async move { store.overdue_runs(&window).await },
            )
            .await;
        let outcomes = self
            .each(overdue, |RunRef { tenant_id, run_id }| {
                self.apply(
                    view,
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
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.deadlines_enforced += 1);
        }
    }

    async fn hints(&self, view: &View, report: &mut Report) {
        let older_than = self.settings.rehint_after;
        let stale = self
            .pick(
                view,
                "hints",
                report,
                |s: &StepRef| s.tenant_id,
                |s| (s.run_id, s.node.clone()),
                |store, window| async move { store.stale_hints(older_than, &window).await },
            )
            .await;
        let outcomes = self
            .each(
                stale,
                |StepRef {
                     tenant_id,
                     run_id,
                     node,
                 }| {
                    let mut effects = self.effects();
                    effects.rehint = vec![node];
                    self.apply(view, tenant_id, run_id, "rehint", effects, |_, _, _| {
                        Ok((
                            (),
                            Progress {
                                events: Vec::new(),
                                pending: None,
                            },
                        ))
                    })
                },
            )
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.hints_renewed += 1);
        }
    }

    // ------------------------------------------------------------------ child runs (FR-002-009)

    /// Creates the child runs that `child` steps reserved. Each is created, started and its
    /// reservation claimed in one transaction under the parent's lock, so a child exists exactly
    /// when its parent step was still waiting for it.
    async fn children_start(&self, view: &View, report: &mut Report) {
        let Some(definitions) = &self.definitions else {
            return;
        };
        let pending = self
            .pick(
                view,
                "pending_children",
                report,
                |c: &PendingChild| c.step.tenant_id,
                |c| (c.step.run_id, c.step.node.clone()),
                |store, window| async move { store.pending_children(&window).await },
            )
            .await;
        let outcomes = self
            .each(pending, |child| async move {
                let outcome = match self.start_child(view, definitions, &child).await {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        tracing::error!(%error, run = %child.step.run_id, node = %child.step.node, "starting a child run failed");
                        Outcome::Failed
                    }
                };
                metrics::counter!("scheduler_actions_total", "kind" => "child_start", "outcome" => outcome.label())
                    .increment(1);
                if outcome == Outcome::Fenced {
                    metrics::counter!("scheduler_fenced_total", "kind" => "child_start")
                        .increment(1);
                }
                outcome
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.children_started += 1);
        }
    }

    async fn start_child(
        &self,
        view: &View,
        definitions: &DefinitionCache,
        pending: &PendingChild,
    ) -> Result<Outcome, EngineError> {
        let tenant = pending.step.tenant_id;
        let parent_id = pending.step.run_id;
        let node = &pending.step.node;
        // The shard is the parent's: it is the parent step that waits for this child, and the
        // candidate was found through it. (The child's own shard is whoever owns the child's id.)
        let hold = view.hold_for(parent_id);
        if matches!(hold, Hold::Lost) {
            return Ok(Outcome::Fenced);
        }
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
                    view,
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
            parent_span: None,
            rerun_of: None,
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
                    view,
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
                // The fence, before anything is committed: a replica that lost the parent's shard
                // while it was reading must not leave a child run behind (dropping rolls it back).
                if !view.holds(&mut tx, &hold).await? {
                    return Ok(Outcome::Fenced);
                }
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
        view: &View,
        tenant: TenantId,
        parent: RunId,
        node: sagat_domain::workflow::NodeId,
        child: RunId,
        outcome: ChildOutcome,
    ) -> Outcome {
        self.apply(
            view,
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
    async fn children_finish(&self, view: &View, report: &mut Report) {
        let ended = self
            .pick(
                view,
                "ended_children",
                report,
                |c: &EndedChild| c.tenant_id,
                |c| c.child,
                |store, window| async move { store.ended_children(&window).await },
            )
            .await;
        let outcomes = self
            .each(
                ended,
                |EndedChild {
                     tenant_id,
                     child,
                     parent,
                     node,
                     status,
                 }| async move {
                    // Every way a run can end, and nothing else: a status added later must be mapped
                    // here (a run that ended in a way nobody reports would keep its parent waiting
                    // for ever, and sit at the head of the list of ended children).
                    let outcome = match status {
                        RunStatus::Succeeded => ChildOutcome::Succeeded,
                        RunStatus::Failed => ChildOutcome::Failed(FailureCode::child_failed()),
                        RunStatus::Cancelled => {
                            ChildOutcome::Failed(FailureCode::child_cancelled())
                        }
                        RunStatus::TimedOut => {
                            ChildOutcome::Failed(FailureCode::child_timed_out())
                        }
                        RunStatus::Compensated => {
                            ChildOutcome::Failed(FailureCode::child_compensated())
                        }
                        RunStatus::Partial => ChildOutcome::Failed(FailureCode::child_partial()),
                        RunStatus::CompensationFailed => {
                            ChildOutcome::Failed(FailureCode::child_compensation_failed())
                        }
                        RunStatus::Running
                        | RunStatus::Waiting
                        | RunStatus::Paused
                        | RunStatus::Cancelling
                        | RunStatus::Compensating
                        | RunStatus::ManualReview => return None,
                    };
                    let result = self
                        .finish_with(view, tenant_id, parent, node.clone(), child, outcome)
                        .await;
                    if result == Outcome::Skipped {
                        tracing::warn!(%parent, %node, %child, "an ended child could not be reported to its parent step");
                    }
                    Some(result)
                },
            )
            .await;
        for outcome in outcomes.into_iter().flatten() {
            Self::tally(report, outcome, |r| r.children_finished += 1);
        }
    }

    /// Stops the running children of a parent that was stopped under a policy that cancels them.
    async fn children_cancel(&self, view: &View, report: &mut Report) {
        let doomed = self
            .pick(
                view,
                "children_to_cancel",
                report,
                |r: &RunRef| r.tenant_id,
                |r| r.run_id,
                |store, window| async move { store.children_to_cancel(&window).await },
            )
            .await;
        let outcomes = self
            .each(doomed, |RunRef { tenant_id, run_id }| {
                self.apply(
                    view,
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
            })
            .await;
        for outcome in outcomes {
            Self::tally(report, outcome, |r| r.children_cancelled += 1);
        }
    }
}
