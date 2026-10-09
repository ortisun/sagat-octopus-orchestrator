//! Metrics of the saga machinery, counted once the transaction that caused them has committed: a
//! rolled-back transition leaves no trace in a counter, and a crash between commit and count loses
//! at most the count, never the state (the journal is the record).
//!
//! Labels are closed sets (never a tenant, run or node id), so cardinality is bounded by this file.

use sagat_domain::execution::{ReviewOutcome, RunEvent};
use sagat_domain::saga::{ReconcileVerdict, SagaOutcome};

/// What a committed transaction adds to the counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    /// `saga_transitions_total{event}`.
    SagaTransition(&'static str),
    /// `saga_outcomes_total{outcome}`.
    SagaOutcome(&'static str),
    /// `compensation_attempts_total{result}`.
    CompensationAttempt(&'static str),
    /// `reconciliations_total{stage}`.
    Reconciliation(&'static str),
    /// `operator_decisions_total{action}`.
    OperatorDecision(&'static str),
    /// `compensation_breaker_opened_total`.
    BreakerOpened,
}

impl Signal {
    pub(crate) fn emit(self) {
        match self {
            Self::SagaTransition(event) => {
                metrics::counter!("saga_transitions_total", "event" => event).increment(1);
            }
            Self::SagaOutcome(outcome) => {
                metrics::counter!("saga_outcomes_total", "outcome" => outcome).increment(1);
            }
            Self::CompensationAttempt(result) => {
                metrics::counter!("compensation_attempts_total", "result" => result).increment(1);
            }
            Self::Reconciliation(stage) => {
                metrics::counter!("reconciliations_total", "stage" => stage).increment(1);
            }
            Self::OperatorDecision(action) => {
                metrics::counter!("operator_decisions_total", "action" => action).increment(1);
            }
            Self::BreakerOpened => {
                metrics::counter!("compensation_breaker_opened_total").increment(1);
            }
        }
    }
}

/// The counters one journal event moves, apart from the breaker's (which depend on its state).
pub(crate) fn of_event(event: &RunEvent) -> Vec<Signal> {
    let mut signals = Vec::new();
    match event {
        RunEvent::SagaEntered { .. } => signals.push(Signal::SagaTransition("entered")),
        RunEvent::SagaCompleted { .. } => signals.push(Signal::SagaTransition("completed")),
        RunEvent::SagaAborting { .. } => signals.push(Signal::SagaTransition("aborting")),
        RunEvent::CompensationPlanned { .. } => signals.push(Signal::SagaTransition("planned")),
        RunEvent::SagaCompensated { outcome, .. } => {
            signals.push(Signal::SagaOutcome(match outcome {
                SagaOutcome::Compensated => "compensated",
                SagaOutcome::Partial => "partial",
                SagaOutcome::CompensationFailed => "compensation_failed",
            }))
        }
        RunEvent::ReconciliationRequested { .. } => {
            signals.push(Signal::Reconciliation("requested"))
        }
        RunEvent::ReconciliationConcluded { verdict, .. } => {
            signals.push(Signal::Reconciliation(match verdict {
                ReconcileVerdict::Applied => "applied",
                ReconcileVerdict::NotApplied => "not_applied",
                ReconcileVerdict::Unknown => "unknown",
            }));
        }
        RunEvent::StepNeedsReview { node, fence, .. } => {
            signals.push(Signal::OperatorDecision("requested"));
            if node.is_compensation() && fence.is_some() {
                signals.push(Signal::CompensationAttempt("needs_review"));
            }
        }
        RunEvent::StepRetryRequested { .. } => signals.push(Signal::OperatorDecision("retry")),
        RunEvent::StepSkipped { .. } => signals.push(Signal::OperatorDecision("skip")),
        RunEvent::StepResolved { outcome, .. } => {
            signals.push(Signal::OperatorDecision(match outcome {
                ReviewOutcome::Succeeded => "resolve_succeeded",
                ReviewOutcome::Failed(_) => "resolve_failed",
            }))
        }
        RunEvent::StepCompleted { node, .. } if node.is_compensation() => {
            signals.push(Signal::CompensationAttempt("succeeded"));
        }
        RunEvent::StepRetryScheduled { node, .. } if node.is_compensation() => {
            signals.push(Signal::CompensationAttempt("retry_scheduled"));
        }
        RunEvent::StepFailed {
            node,
            fence: Some(_),
            ..
        } if node.is_compensation() => {
            signals.push(Signal::CompensationAttempt("failed"));
        }
        _ => {}
    }
    signals
}
