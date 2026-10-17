//! The telemetry projector: turns the journal into the spans and measurements of runs and attempts
//! (FR-008-001, FR-008-002, ADR-0017).
//!
//! It reads the notes the write path leaves, derives with `sagat_application::telemetry::derive`,
//! hands the spans to the collector, and only when the collector has them moves the progress of the
//! run forward. A collector that is down delays the spans and loses none; a projector that dies
//! between exporting and moving the progress exports the same spans again, with the same ids.
//! Measurements are made after the progress is recorded, so a retry of the same batch does not
//! count twice.
//!
//! One projector works at a time (a lease in the database); a second replica waits for it to expire.

use std::time::Duration;

use sagat_application::telemetry::{
    Observation, Pending, TelemetryStore, ToProject, derive, derive_unsupported,
};
use sagat_telemetry::{ExportError, OtlpClient, SpanData, keeps};

/// Tunable limits of the projector.
#[derive(Debug, Clone, Copy)]
pub struct ProjectorSettings {
    /// Runs taken per round.
    pub batch_runs: u32,
    /// Spans in one request to the collector.
    pub batch_spans: usize,
    /// How far behind the projection may be before it is called behind.
    pub lag_budget: Duration,
    /// The share of traces whose spans are exported (`OTEL_TRACES_SAMPLER_ARG`): decided from the
    /// trace id, so it agrees with the live spans of every process. The measurements are made
    /// whatever it is.
    pub sample_ratio: f64,
}

impl Default for ProjectorSettings {
    fn default() -> Self {
        Self {
            batch_runs: 50,
            batch_spans: 256,
            lag_budget: Duration::from_secs(30),
            sample_ratio: 1.0,
        }
    }
}

/// What one round did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionRound {
    /// This process did not hold the lease: nothing was done.
    pub passive: bool,
    /// Runs whose new journal entries were taken.
    pub runs: u32,
    /// Spans the collector accepted.
    pub spans: u32,
    /// Runs that had nothing to give (no trace to belong to, or nothing new).
    pub skipped: u32,
    /// The collector refused a batch for good and it was dropped.
    pub rejected: u32,
    /// The round stopped because the collector or the store could not be reached.
    pub failed: bool,
}

impl ProjectionRound {
    /// Whether the round moved the projection (so the next may follow at once).
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.runs > 0 || self.skipped > 0
    }
}

/// How the projection is doing, in words: for the API and the dashboards.
#[must_use]
pub fn state(backlog_runs: u64, oldest_seconds: f64, budget: Duration) -> &'static str {
    if backlog_runs == 0 {
        "CURRENT"
    } else if oldest_seconds <= budget.as_secs_f64() {
        "CATCHING_UP"
    } else {
        "BEHIND"
    }
}

/// The projector.
pub struct TelemetryProjector<S> {
    store: S,
    exporter: Option<OtlpClient>,
    settings: ProjectorSettings,
}

impl<S> std::fmt::Debug for TelemetryProjector<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryProjector")
            .field("exporting", &self.exporter.is_some())
            .finish_non_exhaustive()
    }
}

/// A run taken from the notes with what was derived from it.
struct Taken {
    pending: Pending,
    upto: u64,
    spans: Vec<SpanData>,
    observations: Vec<Observation>,
}

impl<S: TelemetryStore> TelemetryProjector<S> {
    /// A projector; with no `exporter` the measurements are still made and the spans are not sent.
    pub const fn new(store: S, exporter: Option<OtlpClient>, settings: ProjectorSettings) -> Self {
        Self {
            store,
            exporter,
            settings,
        }
    }

    /// One round: takes a batch of runs that changed, projects them, and says what it did. Never
    /// fails: what cannot be done now is counted and tried again by the next round.
    pub async fn run_once(&self) -> ProjectionRound {
        let mut round = ProjectionRound::default();
        match self.store.acquire_projector().await {
            Ok(true) => metrics::gauge!("telemetry_projector_active").set(1.0),
            Ok(false) => {
                metrics::gauge!("telemetry_projector_active").set(0.0);
                round.passive = true;
                return round;
            }
            Err(error) => {
                tracing::warn!(%error, "could not take the telemetry projector's lease");
                metrics::counter!("telemetry_projection_errors_total", "reason" => "store")
                    .increment(1);
                round.failed = true;
                return round;
            }
        }
        let pending = match self.store.pending_runs(self.settings.batch_runs).await {
            Ok(pending) => pending,
            Err(error) => {
                tracing::warn!(%error, "could not read the runs to project");
                metrics::counter!("telemetry_projection_errors_total", "reason" => "store")
                    .increment(1);
                round.failed = true;
                return round;
            }
        };
        let mut group: Vec<Taken> = Vec::new();
        let mut group_spans = 0usize;
        for note in &pending {
            match self.take(note).await {
                Ok(Some(taken)) => {
                    group_spans += taken.spans.len();
                    group.push(taken);
                }
                Ok(None) => {
                    // Nothing to derive (a run with no trace): the note is moved on so it is not asked again.
                    if self
                        .store
                        .mark_projected(note, note.head_seq)
                        .await
                        .is_err()
                    {
                        round.failed = true;
                        break;
                    }
                    round.skipped += 1;
                }
                Err(error) => {
                    tracing::warn!(%error, run = %note.run_id, "could not load a run to project");
                    metrics::counter!("telemetry_projection_errors_total", "reason" => "store")
                        .increment(1);
                    round.failed = true;
                    break;
                }
            }
            if group_spans >= self.settings.batch_spans {
                if !self.flush(&mut group, &mut round).await {
                    break;
                }
                group_spans = 0;
            }
        }
        if !round.failed && !round.passive {
            self.flush(&mut group, &mut round).await;
        }
        self.gauges().await;
        round
    }

    async fn take(
        &self,
        note: &Pending,
    ) -> Result<Option<Taken>, sagat_application::error::StoreError> {
        let Some(ToProject {
            facts,
            entries,
            unsupported,
            head_seq,
            applied_seq,
        }) = self.store.load_for_projection(note).await?
        else {
            return Ok(None);
        };
        let mut derived = derive(&facts, &entries, applied_seq);
        // Entries of a kind this version does not know are shown, not dropped and not fatal.
        let skipped = derive_unsupported(&facts, &unsupported, applied_seq);
        derived.spans.extend(skipped.spans);
        derived.observations.extend(skipped.observations);
        // A trace the sampler does not keep is not exported, and its progress moves all the same.
        if !keeps(facts.trace_id, self.settings.sample_ratio) {
            derived.spans.clear();
        }
        // The note is written in the transaction that appends to the journal, so what it claims is
        // there when it is read. When nothing at all came after what was applied and the note says
        // there should be more, the journal is shorter than the note (a restore, a trimmed journal):
        // there is nothing more to wait for, and asking again would only keep the projector busy
        // and the run "behind" for good. The note is settled to what it claimed.
        let upto = if head_seq == applied_seq && note.head_seq > applied_seq {
            tracing::warn!(
                run = %note.run_id,
                claimed = note.head_seq,
                found = applied_seq,
                "the note of a run claims journal entries that are not there; settling it"
            );
            metrics::counter!("telemetry_projection_errors_total", "reason" => "inconsistent")
                .increment(1);
            note.head_seq
        } else {
            head_seq
        };
        Ok(Some(Taken {
            pending: note.clone(),
            upto,
            spans: derived.spans,
            observations: derived.observations,
        }))
    }

    /// Hands the group's spans to the collector and, when it has them, records the progress and
    /// makes the measurements. `false` when the round must stop.
    async fn flush(&self, group: &mut Vec<Taken>, round: &mut ProjectionRound) -> bool {
        if group.is_empty() {
            return true;
        }
        // A batch can take longer than the lease (a slow collector, several tries): the lease is
        // asked for again before each hand-over, and a round that has lost it stops, so two
        // projectors do not work on the same runs for long.
        match self.store.acquire_projector().await {
            Ok(true) => {}
            Ok(false) => {
                metrics::gauge!("telemetry_projector_active").set(0.0);
                round.passive = true;
                return false;
            }
            Err(error) => {
                tracing::warn!(%error, "could not renew the telemetry projector's lease");
                metrics::counter!("telemetry_projection_errors_total", "reason" => "store")
                    .increment(1);
                round.failed = true;
                return false;
            }
        }
        let taken = std::mem::take(group);
        let spans: Vec<SpanData> = taken.iter().flat_map(|t| t.spans.iter().cloned()).collect();
        let mut exported = 0u32;
        if let Some(exporter) = &self.exporter {
            for chunk in spans.chunks(self.settings.batch_spans.max(1)) {
                match exporter.export(chunk).await {
                    Ok(()) => {
                        exported += u32::try_from(chunk.len()).unwrap_or(u32::MAX);
                        metrics::counter!("telemetry_spans_exported_total", "outcome" => "ok")
                            .increment(chunk.len() as u64);
                    }
                    Err(ExportError::Rejected(status)) => {
                        // Sending it again changes nothing: it is dropped, loudly, and the run moves on.
                        tracing::warn!(
                            status,
                            spans = chunk.len(),
                            "the collector refused a batch of spans for good"
                        );
                        metrics::counter!("telemetry_spans_exported_total", "outcome" => "rejected")
                            .increment(chunk.len() as u64);
                        round.rejected += 1;
                    }
                    Err(ExportError::Unavailable(why)) => {
                        tracing::warn!(%why, "the collector is not available; the spans wait in the journal");
                        metrics::counter!("telemetry_projection_errors_total", "reason" => "collector")
                            .increment(1);
                        round.failed = true;
                        return false;
                    }
                }
            }
        }
        round.spans += exported;
        for run in &taken {
            match self.store.mark_projected(&run.pending, run.upto).await {
                Ok(true) => {
                    round.runs += 1;
                    record(&run.observations);
                }
                // Another projector took this run while this one was exporting (its lease ran
                // out): its measurements were made by whoever moved the note, not twice.
                Ok(false) => round.skipped += 1,
                Err(error) => {
                    tracing::warn!(%error, run = %run.pending.run_id, "could not record the progress of a projection");
                    metrics::counter!("telemetry_projection_errors_total", "reason" => "store")
                        .increment(1);
                    round.failed = true;
                    return false;
                }
            }
        }
        true
    }

    async fn gauges(&self) {
        if let Ok(backlog) = self.store.backlog().await {
            #[allow(clippy::cast_precision_loss)] // a gauge
            metrics::gauge!("telemetry_projection_pending_runs").set(backlog.runs as f64);
            metrics::gauge!("telemetry_projection_lag_seconds").set(backlog.oldest_seconds);
        }
    }

    /// How the projection is doing now.
    pub async fn status(&self) -> Option<(u64, f64, &'static str)> {
        let backlog = self.store.backlog().await.ok()?;
        Some((
            backlog.runs,
            backlog.oldest_seconds,
            state(
                backlog.runs,
                backlog.oldest_seconds,
                self.settings.lag_budget,
            ),
        ))
    }
}

/// The measurements the projection makes. Labels are closed sets (an outcome, a status).
fn record(observations: &[Observation]) {
    for observation in observations {
        match observation {
            Observation::AttemptEnded {
                outcome,
                seconds,
                schedule_latency,
            } => {
                metrics::counter!("step_attempts_total", "outcome" => *outcome).increment(1);
                metrics::histogram!("step_attempt_duration_seconds", "outcome" => *outcome)
                    .record(*seconds);
                if let Some(latency) = schedule_latency {
                    metrics::histogram!("step_schedule_latency_seconds").record(*latency);
                }
            }
            Observation::RunFinished { status, seconds } => {
                metrics::counter!("runs_terminal_total", "status" => *status).increment(1);
                metrics::histogram!("run_duration_seconds", "status" => *status).record(*seconds);
            }
            Observation::PrimitiveTransition { kind, outcome } => {
                metrics::counter!(
                    "workflow_primitive_transitions_total",
                    "kind" => kind.as_str(),
                    "outcome" => outcome.as_str(),
                )
                .increment(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sagat_application::telemetry::Observation;
    use sagat_telemetry::{Metrics, WorkflowPrimitiveKind, WorkflowPrimitiveOutcome};

    use super::record;

    #[test]
    fn primitive_transition_metric_has_only_closed_kind_and_outcome_labels() {
        let (metrics, recorder) = Metrics::detached(Duration::from_secs(60));
        metrics::with_local_recorder(&recorder, || {
            record(&[
                Observation::PrimitiveTransition {
                    kind: WorkflowPrimitiveKind::Switch,
                    outcome: WorkflowPrimitiveOutcome::Case,
                },
                Observation::PrimitiveTransition {
                    kind: WorkflowPrimitiveKind::Switch,
                    outcome: WorkflowPrimitiveOutcome::Default,
                },
            ]);
        });

        let page = metrics.render();
        assert!(
            page.contains(
                "workflow_primitive_transitions_total{kind=\"switch\",outcome=\"case\"} 1"
            )
        );
        assert!(page.contains(
            "workflow_primitive_transitions_total{kind=\"switch\",outcome=\"default\"} 1"
        ));
        assert!(!page.contains("node="));
        assert!(!page.contains("case_id="));
        assert!(!page.contains("tenant="));
    }
}
