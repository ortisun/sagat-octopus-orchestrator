//! The event dispatcher: turns admitted events into started runs and woken steps (`FR-004-001`,
//! `FR-004-003`, `FR-004-008`).
//!
//! It is a loop over the inbox, safe to run in several replicas: an event is claimed with a lease
//! and every effect of dispatching it is idempotent, because it is recorded in the same transaction
//! as the thing it did (a run created under an idempotency key, a step woken together with a
//! delivery row). A dispatcher that dies half way leaves an event whose lease runs out, and the next
//! one finishes the job without doing anything twice. A failure that is not a crash is retried with
//! backoff until the event is set aside as dead, where an operator can see it and why.

use std::sync::Arc;

use sagat_application::clock::{Clock, SystemClock};
use sagat_application::definitions::{Applied, DefinitionStore, Idempotency};
use sagat_application::dispatch::{
    CLAIM_LEASE, ClaimedEvent, CronSchedule, DeliveryRef, EventDispatchStore, EventView,
    StoredTrigger, WaitingStep, start_key,
};
use sagat_application::entropy::Entropy;
use sagat_application::error::StoreError;
use sagat_application::execution::{
    Actor, DefinitionCache, Effects, Engine, EngineError, ExpressionEvaluator,
    MAX_RUN_OUTPUT_BYTES, RunInfo, RunTx,
};
use sagat_application::quotas::RunLimits;
use sagat_application::runs::{Lock, NewRun, RunStore};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::events::{
    GIVE_UP_AFTER, MAX_DISPATCH_ATTEMPTS, capacity_backoff, dispatch_backoff,
};
use sagat_domain::execution::{self, ExecutionError, RunId, StepStatus, WaitingOn};
use sagat_domain::workflow::{Node, NodeKind, WorkflowDefinition};
use time::OffsetDateTime;

/// Tunable limits of the dispatcher.
#[derive(Debug, Clone, Copy)]
pub struct EventDispatchSettings {
    /// Events claimed per round.
    pub batch: u32,
    /// Waiting steps one event may wake (a fan-out larger than this is delivered over the
    /// following rounds: the event is retried until the waits that are left are woken).
    pub fan_out: u32,
    /// Event waits whose inbox is searched per round.
    pub scan_batch: u32,
    /// Expired events removed per housekeeping round.
    pub purge_batch: u32,
}

impl Default for EventDispatchSettings {
    fn default() -> Self {
        Self {
            batch: 100,
            fan_out: 500,
            scan_batch: 200,
            purge_batch: 5_000,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventRound {
    /// Events whose dispatch finished.
    pub dispatched: u32,
    pub runs_started: u32,
    pub steps_woken: u32,
    /// Events whose dispatch failed and will be tried again.
    pub retried: u32,
    /// Events given up on after too many failures.
    pub dead: u32,
}

impl EventRound {
    /// Whether anything at all happened (so the loop should go on at once).
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.dispatched + self.runs_started + self.steps_woken + self.retried + self.dead > 0
    }
}

/// How a delivery to a waiting step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Woken {
    /// The step was woken and the delivery recorded.
    Yes,
    /// Not (any more) for this event: the run is gone, the step no longer waits for it, the wait
    /// timed out first, or the event had already been delivered to it.
    No,
    /// The wake-up was refused for a reason that will not change (the run keeps too much output).
    Refused,
}

/// One firing of a trigger: what makes it the same firing when it is repeated, and why it happened.
struct Firing<'a> {
    key: String,
    fingerprint: Vec<u8>,
    /// For the audit record of the run.
    reason: String,
    /// The event that fired it, when it was one.
    event: Option<&'a ClaimedEvent>,
}

/// How starting a run for a trigger ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Started {
    Yes,
    /// The event had already started the run (a repeat), or the trigger cannot start one.
    No,
}

/// The dispatcher over a store, an evaluator and a source of jitter.
pub struct EventDispatcher<S, E, R> {
    store: S,
    engine: Engine<E>,
    entropy: R,
    definitions: DefinitionCache,
    settings: EventDispatchSettings,
    /// The calendar cron triggers are evaluated against; without one they are left alone.
    cron: Option<Arc<dyn CronSchedule>>,
    /// The process's clock, for what is measured against it (how late an event is). What is due is
    /// decided by the database's clock.
    clock: Arc<dyn Clock>,
}

impl<S, E, R> std::fmt::Debug for EventDispatcher<S, E, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventDispatcher").finish_non_exhaustive()
    }
}

impl<S, E, R> EventDispatcher<S, E, R>
where
    S: RunStore + DefinitionStore + EventDispatchStore,
    E: ExpressionEvaluator,
    R: Entropy,
{
    pub fn new(
        store: S,
        engine: Engine<E>,
        entropy: R,
        definitions: DefinitionCache,
        settings: EventDispatchSettings,
    ) -> Self {
        Self {
            store,
            engine,
            entropy,
            definitions,
            settings,
            cron: None,
            clock: Arc::new(SystemClock),
        }
    }

    /// Measures lag and age against another clock (a test's).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Lets the dispatcher fire the cron triggers of published workflows.
    #[must_use]
    pub fn with_cron(mut self, calendar: Arc<dyn CronSchedule>) -> Self {
        self.cron = Some(calendar);
        self
    }

    /// One round: searches the inbox for the events waits missed, then dispatches the events that
    /// are due.
    pub async fn tick(&self) -> EventRound {
        let mut round = EventRound::default();
        self.cron_pass(&mut round).await;
        self.late_waits(&mut round).await;
        match self
            .store
            .claim_events(self.settings.batch, CLAIM_LEASE)
            .await
        {
            Ok(events) => {
                for event in events {
                    self.dispatch_one(&event, &mut round).await;
                }
            }
            Err(error) => tracing::warn!(%error, "claiming events failed"),
        }
        if let Ok(pending) = self.store.pending_events().await {
            #[allow(clippy::cast_precision_loss)] // a gauge
            metrics::gauge!("inbox_pending_events").set(pending as f64);
        }
        round
    }

    /// Removes the events whose deduplication window ended. Run it rarely.
    pub async fn housekeeping(&self) {
        let mut removed = 0u64;
        for _ in 0..20 {
            match self.store.purge_events(self.settings.purge_batch).await {
                Ok(n) => {
                    removed += n;
                    if n < u64::from(self.settings.purge_batch) {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "could not purge expired events");
                    break;
                }
            }
        }
        if removed > 0 {
            tracing::info!(removed, "purged expired events");
        }
    }

    async fn dispatch_one(&self, event: &ClaimedEvent, round: &mut EventRound) {
        let started = std::time::Instant::now();
        let outcome = self.dispatch(event, round).await;
        metrics::histogram!("event_dispatch_seconds").record(started.elapsed().as_secs_f64());
        match outcome {
            Ok(()) => {
                if let Err(error) = self.store.finish_event(event).await {
                    // The deliveries are recorded; the event is claimed again after its lease and
                    // finds nothing left to do.
                    tracing::warn!(%error, "could not mark an event dispatched");
                    return;
                }
                round.dispatched += 1;
                metrics::counter!("events_dispatched_total", "outcome" => "dispatched")
                    .increment(1);
                let lag = (self.clock.now() - event.received_at).as_seconds_f64();
                metrics::histogram!("event_dispatch_lag_seconds").record(lag.max(0.0));
            }
            Err(error) => {
                // Waiting for room (a quota, a store that is down) is not the event's failure: it
                // does not use up its attempts and is tried again soon, for a day. Anything else is
                // counted, backs off, and ends in `DEAD`.
                let waiting = is_waiting_for_room(&error);
                let age = self.clock.now() - event.received_at;
                let dead = if waiting {
                    age > time::Duration::try_from(GIVE_UP_AFTER).unwrap_or(time::Duration::DAY)
                } else {
                    event.attempts + 1 >= MAX_DISPATCH_ATTEMPTS
                };
                let delay = if waiting {
                    capacity_backoff(self.entropy.unit())
                } else {
                    dispatch_backoff(event.attempts, self.entropy.unit())
                };
                let why = describe(&error);
                tracing::warn!(%error, attempts = event.attempts, dead, waiting, "dispatching an event failed");
                if let Err(store) = self
                    .store
                    .retry_event(event, why, delay, dead, !waiting)
                    .await
                {
                    tracing::warn!(%store, "could not schedule another attempt for an event");
                    return;
                }
                if dead {
                    round.dead += 1;
                    metrics::counter!("events_dispatched_total", "outcome" => "dead").increment(1);
                } else {
                    round.retried += 1;
                    metrics::counter!("events_dispatched_total", "outcome" => "retry").increment(1);
                }
            }
        }
    }

    /// Delivers one event to every trigger and every waiting step it is about.
    async fn dispatch(
        &self,
        event: &ClaimedEvent,
        round: &mut EventRound,
    ) -> Result<(), EngineError> {
        let document = self.document(event);
        // One consumer that cannot be served now (no room for its run) must not keep the others
        // from theirs: every one is tried, the first failure is what the event is retried for, and
        // the retry repeats only what was not recorded (each effect is recorded with its delivery).
        let mut failure: Option<EngineError> = None;
        for trigger in self.store.triggers_matching(event).await? {
            if !trigger.registration.kind.is_event() {
                continue;
            }
            // (The query narrowed by source and type with NULL meaning "any"; the kind decides
            // nothing more.)
            if let Some(filter) = &trigger.registration.filter {
                match self.engine.evaluator().event_filter(filter, &document) {
                    Ok(true) => {}
                    Ok(false) => {
                        metrics::counter!("trigger_events_total", "outcome" => "filtered")
                            .increment(1);
                        continue;
                    }
                    Err(error) => {
                        // A filter that cannot be evaluated over this event does not match it.
                        tracing::warn!(%error, trigger = %trigger.registration.id, "a trigger filter failed");
                        metrics::counter!("trigger_events_total", "outcome" => "filter_error")
                            .increment(1);
                        continue;
                    }
                }
            }
            match self.start_run(&trigger, event, &document).await {
                Ok(Started::Yes) => round.runs_started += 1,
                Ok(Started::No) => {}
                Err(error) => {
                    tracing::warn!(%error, trigger = %trigger.registration.id, "starting the run of a trigger failed");
                    failure.get_or_insert(error);
                }
            }
        }

        // The waits that want this event, a page at a time. A wait that is woken is no longer a
        // wait, but one that is not (it lost a race, or already had the event) stays, so the walk
        // goes on from where the last page ended and not from the start.
        let mut after: Option<(RunId, sagat_domain::workflow::NodeId)> = None;
        loop {
            let waits = self
                .store
                .waits_for_event(
                    event,
                    after.as_ref().map(|(run, node)| (*run, node)),
                    self.settings.fan_out,
                )
                .await?;
            let last = waits.last().map(|w| (w.run_id, w.node.clone()));
            let page = waits.len();
            for step in waits {
                match self.wake(&step, event, &document, "event").await {
                    Ok(Woken::Yes) => round.steps_woken += 1,
                    Ok(Woken::No | Woken::Refused) => {}
                    Err(error) => {
                        tracing::warn!(%error, run = %step.run_id, node = %step.node, "waking a step failed");
                        failure.get_or_insert(error);
                    }
                }
            }
            if page < self.settings.fan_out as usize {
                return failure.map_or(Ok(()), Err);
            }
            after = last;
        }
    }

    /// The document a run or a step is given for `event`.
    fn document(&self, event: &ClaimedEvent) -> Vec<u8> {
        let received = ClaimedEvent::rfc3339(event.received_at);
        let occurred = event.occurred_at.map(ClaimedEvent::rfc3339);
        self.engine.evaluator().event_document(&EventView {
            id: event.envelope.key.event_id.as_str(),
            source: event.envelope.key.source.as_str(),
            event_type: event.envelope.event_type.as_str(),
            subject: event.envelope.subject.as_deref(),
            correlation_key: event.envelope.correlation_key.as_ref().map(|k| k.as_str()),
            occurred_at: occurred.as_deref(),
            received_at: &received,
            data: &event.payload,
        })
    }

    /// Starts the run that `trigger` starts for `event`, once.
    async fn start_run(
        &self,
        trigger: &StoredTrigger,
        event: &ClaimedEvent,
        document: &[u8],
    ) -> Result<Started, EngineError> {
        let (key, fingerprint) = start_key(event);
        let firing = Firing {
            key,
            fingerprint,
            reason: format!(
                "event '{}/{}' via trigger {}",
                event.envelope.key.source, event.envelope.key.event_id, trigger.registration.id
            ),
            event: Some(event),
        };
        self.start(trigger, &firing, document).await
    }

    /// Starts the run of `trigger` for one firing of it (an event, or a time on a schedule), once:
    /// the idempotency key of the firing is what makes a repeat start nothing.
    async fn start(
        &self,
        trigger: &StoredTrigger,
        firing: &Firing<'_>,
        document: &[u8],
    ) -> Result<Started, EngineError> {
        let tenant = trigger.tenant_id;
        let Some(version) = self
            .store
            .get_version_by_id(tenant, trigger.version_id)
            .await?
        else {
            tracing::error!(trigger = %trigger.registration.id, "a trigger names a version that does not exist");
            return Ok(Started::No);
        };
        let definition = self
            .definitions
            .get_or_parse(version.version.id(), &version.document)
            .map_err(|_| StoreError::Internal {
                detail: "a stored version no longer parses".into(),
            })?;
        let run_id = RunId::generate();
        let now = OffsetDateTime::now_utc();
        // The run continues the trace of the event that started it when its publisher sent one
        // (G79); a run started by the clock, or by an event with no trace, begins a trace of its own,
        // so that every run can be followed from its first step to its last.
        let (trace_id, parent_span) = match firing.event.and_then(|event| event.trace.as_ref()) {
            Some(link) => (link.trace_id.clone(), link.parent_span.clone()),
            None => (sagat_telemetry::TraceId::generate().to_string(), None),
        };
        let info = RunInfo {
            run_id,
            tenant_id: tenant,
            namespace_id: trigger.namespace_id,
            workflow_id: trigger.workflow_id,
            version_id: trigger.version_id,
            started_by: sagat_domain::identity::Subject::engine()
                .as_str()
                .to_owned(),
            correlation_id: None,
            trace_id: Some(trace_id.clone()),
            parent_span,
            rerun_of: None,
            deadline_at: None,
            created_at: now,
            parent: None,
            depth: 0,
            root: run_id,
        };
        let audit = AuditRecord::engine(
            tenant,
            AuditAction::RunStarted,
            ResourceType::Run,
            run_id.to_string(),
            firing.reason.clone(),
            Some(trace_id),
            now,
        );
        let limits = RunLimits::of(&self.store, tenant, trigger.namespace_id).await?;
        let new = NewRun {
            limits,
            info,
            definition: Arc::clone(&definition),
            input: document.to_vec(),
            input_blob: None,
            audit,
            idempotency: Idempotency {
                scope: format!("trigger:{}", trigger.registration.id),
                key: firing.key.clone(),
                fingerprint: firing.fingerprint.clone(),
            },
        };
        match self.store.create(new).await {
            // An earlier dispatch of this event already started it.
            Ok(Applied::Replayed(_)) => Ok(Started::No),
            Ok(Applied::Done(mut tx)) => {
                // An event leaves the evidence of its delivery with the run it starts.
                if let Some(event) = firing.event {
                    let delivery = DeliveryRef::to_trigger(event, trigger.registration.id, run_id);
                    if !tx.record_delivery(&delivery).await? {
                        // Delivered before, yet no run was created under this key: nothing to repeat.
                        return Ok(Started::No);
                    }
                }
                let effects = Effects::by(Actor::System("event-dispatcher"), None);
                self.engine
                    .run(&mut tx, effects, |def: &WorkflowDefinition, state, now| {
                        let (started, progress) = execution::start_run(def, run_id, now)?;
                        *state = started;
                        Ok(((), progress))
                    })
                    .await?;
                tx.commit().await?;
                metrics::counter!("trigger_events_total", "outcome" => "run_started").increment(1);
                metrics::counter!("runs_started_total", "source" => "trigger").increment(1);
                Ok(Started::Yes)
            }
            // The event left the inbox while it was being dispatched (it expired): nothing to
            // start for it.
            Err(StoreError::MissingReference) => Ok(Started::No),
            Err(other) => Err(other.into()),
        }
    }

    /// Wakes `step` with `event`, once.
    async fn wake(
        &self,
        step: &WaitingStep,
        event: &ClaimedEvent,
        document: &[u8],
        via: &'static str,
    ) -> Result<Woken, EngineError> {
        let Some(mut tx) = self
            .store
            .begin(step.tenant_id, step.run_id, Lock::Wait)
            .await?
        else {
            return Ok(Woken::No);
        };
        // Under the run's lock, is the step still waiting for exactly this?
        let still_waiting = {
            let (def, state) = tx.parts();
            let waiting = state.step(&step.node).is_some_and(|s| {
                s.status() == StepStatus::Waiting
                    && matches!(
                        s.waiting_on(),
                        Some(WaitingOn::Event { correlation_key, .. })
                            if *correlation_key == step.correlation_key
                    )
            });
            waiting
                && matches!(
                    def.node(&step.node).map(Node::kind),
                    Some(NodeKind::WaitEvent { event_type, .. }) if *event_type == step.event_type
                )
        };
        if !still_waiting {
            return Ok(Woken::No);
        }
        if !tx
            .record_delivery(&DeliveryRef::to_wait(event, step))
            .await?
        {
            return Ok(Woken::No);
        }
        // What a run retains is bounded (every lease carries it).
        let retained: usize = tx.data().await?.outputs.values().map(Vec::len).sum();
        if retained.saturating_add(document.len()) > MAX_RUN_OUTPUT_BYTES {
            tracing::warn!(run = %step.run_id, node = %step.node, "an event was not delivered: the run keeps too much output");
            metrics::counter!("event_wakeups_total", "outcome" => "refused").increment(1);
            return Ok(Woken::Refused);
        }
        let mut effects = Effects::by(Actor::System("event-dispatcher"), None);
        effects.audit = Some(AuditRecord::engine(
            step.tenant_id,
            AuditAction::RunSignalled,
            ResourceType::Run,
            step.run_id.to_string(),
            format!(
                "event '{}/{}' woke '{}'",
                event.envelope.key.source, event.envelope.key.event_id, step.node
            ),
            None,
            OffsetDateTime::now_utc(),
        ));
        effects.outputs = vec![(step.node.clone(), document.to_vec())];
        let node = step.node.clone();
        match self
            .engine
            .run(&mut tx, effects, move |def, state, now| {
                let progress = execution::satisfy_wait(def, state, &node, now)?;
                Ok(((), progress))
            })
            .await
        {
            Ok(()) => {
                tx.commit().await?;
                metrics::counter!("event_wakeups_total", "outcome" => "woken", "via" => via)
                    .increment(1);
                Ok(Woken::Yes)
            }
            // The wait ended first (it timed out, the run was cancelled): the event is not
            // delivered, and the rolled-back delivery row does not say it was.
            Err(EngineError::Execution(error))
                if !matches!(error, ExecutionError::Invariant(_)) =>
            {
                metrics::counter!("event_wakeups_total", "outcome" => "lost_race").increment(1);
                Ok(Woken::No)
            }
            Err(other) => Err(other),
        }
    }

    /// Fires the cron triggers whose time has come.
    ///
    /// A firing is made first, under an idempotency key that names the scheduled instant, and the
    /// schedule is moved on afterwards with a compare-and-swap: a dispatcher that dies in between
    /// leaves a trigger that is still due, and the next one fires it again, which starts nothing
    /// (the key is known) and then moves the schedule. A schedule that was missed while the platform
    /// was down fires once, late, and continues from the first time after now: it is not made up
    /// for, tick by tick.
    async fn cron_pass(&self, round: &mut EventRound) {
        let Some(calendar) = &self.cron else {
            return;
        };
        let due = match self.store.due_cron_triggers(self.settings.batch).await {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "searching for due cron triggers failed");
                return;
            }
        };
        for trigger in due {
            let Some(schedule) = &trigger.registration.schedule else {
                continue;
            };
            let id = trigger.registration.id;
            // A trigger is due by the database's clock, and so is everything that follows from it.
            let now = trigger.db_now;
            let Some(scheduled) = trigger.next_fire_at else {
                // Never scheduled (just published): the first time is the next one, nothing fires.
                match calendar.next_after(schedule, now) {
                    Ok(next) => {
                        if let Err(error) = self.store.advance_cron_trigger(id, None, next).await {
                            tracing::warn!(%error, "could not schedule a cron trigger");
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, trigger = %id, "a cron trigger has no next time");
                    }
                }
                continue;
            };
            let next = match calendar.next_after(schedule, now.max(scheduled)) {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(%error, trigger = %id, "a cron trigger has no next time");
                    continue;
                }
            };
            let lag = (now - scheduled).as_seconds_f64().max(0.0);
            metrics::histogram!("cron_fire_lag_seconds").record(lag);
            let epoch = scheduled.unix_timestamp();
            let at = ClaimedEvent::rfc3339(scheduled);
            let fired = ClaimedEvent::rfc3339(now);
            let data = format!(r#"{{"scheduledAt":"{at}","firedAt":"{fired}"}}"#);
            let event_id = format!("cron:{id}:{epoch}");
            let document = self.engine.evaluator().event_document(&EventView {
                id: &event_id,
                source: "cron",
                event_type: "cron.fired",
                subject: None,
                correlation_key: None,
                occurred_at: Some(&at),
                received_at: &fired,
                data: data.as_bytes(),
            });
            let firing = Firing {
                key: format!("cron-{epoch}"),
                fingerprint: epoch.to_be_bytes().to_vec(),
                reason: format!("cron trigger {id} scheduled for {at}"),
                event: None,
            };
            match self.start(&trigger, &firing, &document).await {
                Ok(started) => {
                    if started == Started::Yes {
                        round.runs_started += 1;
                        metrics::counter!("cron_fires_total", "outcome" => "fired").increment(1);
                    }
                    // Only one dispatcher moves the schedule; the others find it already moved.
                    if let Err(error) = self
                        .store
                        .advance_cron_trigger(id, Some(scheduled), next)
                        .await
                    {
                        tracing::warn!(%error, "could not move a cron trigger on");
                    }
                }
                Err(error) => {
                    // Starting failed (no room for another run, the store is down): try again
                    // soon instead of every round. The tick stays what it was, so the next try is
                    // made under the same key: if this one did commit, the next finds it so.
                    tracing::warn!(%error, trigger = %id, "firing a cron trigger failed");
                    metrics::counter!("cron_fires_total", "outcome" => "failed").increment(1);
                    let retry = now + time::Duration::seconds(30);
                    let _ = self.store.delay_cron_trigger(id, scheduled, retry).await;
                }
            }
        }
    }

    /// Finds, for each wait that has not been looked at, the event it should have received: one
    /// that arrived before the wait began (within its look-back) or while the matcher was between
    /// finding waits and finishing the event.
    async fn late_waits(&self, round: &mut EventRound) {
        let waits = match self.store.unscanned_waits(self.settings.scan_batch).await {
            Ok(waits) => waits,
            Err(error) => {
                tracing::warn!(%error, "searching for event waits failed");
                return;
            }
        };
        for step in waits {
            let outcome = async {
                match self.store.undelivered_event_for(&step).await? {
                    Some(event) => {
                        let document = self.document(&event);
                        let woken = self.wake(&step, &event, &document, "lookback").await?;
                        Ok::<_, EngineError>(woken)
                    }
                    None => Ok(Woken::No),
                }
            }
            .await;
            match outcome {
                Ok(Woken::Yes) => round.steps_woken += 1,
                Ok(Woken::No | Woken::Refused) => {}
                Err(error) => {
                    // Leave it unscanned: the next round looks again.
                    tracing::warn!(%error, run = %step.run_id, node = %step.node, "delivering a past event failed");
                    continue;
                }
            }
            if let Err(error) = self.store.mark_wait_scanned(&step).await {
                tracing::warn!(%error, "could not mark an event wait scanned");
            }
        }
    }
}

/// Whether a dispatch failed only for want of room: a quota that is full, a store that cannot be
/// reached. Neither is a fault of the event.
const fn is_waiting_for_room(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::Store(StoreError::Unavailable { .. } | StoreError::QuotaExceeded { .. })
    )
}

/// What the event records about a failed dispatch: a class, never a detail (details can carry
/// identifiers and are only in the logs).
fn describe(error: &EngineError) -> &'static str {
    match error {
        EngineError::Store(StoreError::Unavailable { .. }) => "storage unavailable; will retry",
        EngineError::Store(StoreError::QuotaExceeded { .. }) => {
            "a quota of the namespace or tenant was reached; will retry"
        }
        EngineError::Store(StoreError::MissingReference) => "a referenced resource is gone",
        EngineError::Store(_) => "storage error; see the logs",
        EngineError::Execution(_) => "the run refused the command; see the logs",
    }
}
