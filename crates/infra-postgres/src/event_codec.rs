//! Journal events as rows: a name, the node and attempt they concern, and a JSON object with the
//! event's own fields. [`decode`] is the exact inverse of [`encode`], so a stored journal can be
//! replayed into a `RunState` and compared with the projections (`verify_run`).
//!
//! Times are stored as microseconds since the epoch, the precision PostgreSQL keeps.

use sagat_domain::execution::{
    ChildOutcome, CorrelationKey, FailureCode, Fence, JoinOutcome, RunEvent, RunId, RunStatus,
    StopReason,
};
use sagat_domain::workflow::NodeId;
use serde_json::{Map, Value, json};
use time::OffsetDateTime;

/// The columns of one journal row that come from the event.
pub(crate) struct Encoded {
    pub event_type: &'static str,
    pub node: Option<String>,
    pub attempt: Option<i32>,
    pub detail: Value,
}

fn micros(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000).unwrap_or(i64::MAX)
}

fn from_micros(value: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000).ok()
}

fn names(nodes: &[NodeId]) -> Value {
    Value::Array(nodes.iter().map(|n| json!(n.as_str())).collect())
}

pub(crate) fn encode(event: &RunEvent) -> Encoded {
    let attempt = event.attempt().and_then(|a| i32::try_from(a).ok());
    let detail = match event {
        RunEvent::RunStarted { entrypoint } => json!({ "entrypoint": entrypoint.as_str() }),
        RunEvent::StepRouted { activated, .. } => json!({ "activated": names(activated) }),
        RunEvent::StepLeased {
            attempt,
            generation,
            ..
        }
        | RunEvent::StepStarted {
            attempt,
            generation,
            ..
        }
        | RunEvent::StepCompleted {
            attempt,
            generation,
            ..
        } => {
            json!({ "attempt": attempt, "generation": generation })
        }
        RunEvent::StepRetryScheduled {
            attempt,
            generation,
            code,
            not_before,
            ..
        } => json!({
            "attempt": attempt,
            "generation": generation,
            "code": code.as_str(),
            "not_before_us": micros(*not_before),
        }),
        RunEvent::StepFailed { code, fence, .. } => json!({
            "code": code.as_str(),
            "fence": fence.map(|f| json!({ "attempt": f.attempt, "generation": f.generation })),
        }),
        RunEvent::ConditionEvaluated { outcome, .. } => json!({ "outcome": outcome }),
        RunEvent::TimerStarted { due_at, .. } => json!({ "due_at_us": micros(*due_at) }),
        RunEvent::WaitStarted {
            correlation_key,
            deadline,
            ..
        } => json!({
            "correlation_key": correlation_key.as_str(),
            "deadline_us": micros(*deadline),
        }),
        RunEvent::WaitTimedOut { handled, .. } => json!({ "handled": handled }),
        RunEvent::ParallelForked { branches, .. } => json!({ "branches": names(branches) }),
        RunEvent::ParallelJoined { outcome, .. } => json!({
            "outcome": match outcome {
                JoinOutcome::Succeeded => "succeeded",
                JoinOutcome::Failed => "failed",
            }
        }),
        RunEvent::ChildStarted { child, .. } => json!({ "child": child.to_string() }),
        RunEvent::ChildFinished { child, outcome, .. } => json!({
            "child": child.to_string(),
            "outcome": match outcome {
                ChildOutcome::Succeeded => Value::Null,
                ChildOutcome::Failed(code) => json!(code.as_str()),
            },
        }),
        RunEvent::RunStopRequested { reason, code } => json!({
            "reason": match reason {
                StopReason::Cancelled => "CANCELLED",
                StopReason::TimedOut => "TIMED_OUT",
                StopReason::Failed => "FAILED",
            },
            "code": code.as_ref().map(FailureCode::as_str),
        }),
        RunEvent::RunFinished { status } => json!({ "status": status.as_str() }),
        RunEvent::StepReleased { .. }
        | RunEvent::StepRetryDue { .. }
        | RunEvent::TimerFired { .. }
        | RunEvent::WaitSatisfied { .. }
        | RunEvent::EndReached { .. }
        | RunEvent::StepCancelRequested { .. }
        | RunEvent::StepCancelled { .. }
        | RunEvent::RunPaused
        | RunEvent::RunResumed => json!({}),
    };
    Encoded {
        event_type: event.name(),
        node: event.node().map(|n| n.as_str().to_owned()),
        attempt,
        detail,
    }
}

fn node_of(node: Option<&str>) -> Option<NodeId> {
    NodeId::parse(node?).ok()
}

fn nodes_of(detail: &Map<String, Value>, key: &str) -> Option<Vec<NodeId>> {
    detail
        .get(key)?
        .as_array()?
        .iter()
        .map(|v| NodeId::parse(v.as_str()?).ok())
        .collect()
}

fn text<'a>(detail: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    detail.get(key)?.as_str()
}

fn number(detail: &Map<String, Value>, key: &str) -> Option<u64> {
    detail.get(key)?.as_u64()
}

fn time_of(detail: &Map<String, Value>, key: &str) -> Option<OffsetDateTime> {
    from_micros(detail.get(key)?.as_i64()?)
}

/// Rebuilds an event; `None` when the row is not one this version wrote.
pub(crate) fn decode(event_type: &str, node: Option<&str>, detail: &Value) -> Option<RunEvent> {
    let d = detail.as_object()?;
    let node_id = || node_of(node);
    let fence = |d: &Map<String, Value>| {
        Some((
            u32::try_from(number(d, "attempt")?).ok()?,
            number(d, "generation")?,
        ))
    };
    Some(match event_type {
        "run.started" => RunEvent::RunStarted {
            entrypoint: NodeId::parse(text(d, "entrypoint")?).ok()?,
        },
        "step.routed" => RunEvent::StepRouted {
            node: node_id()?,
            activated: nodes_of(d, "activated")?,
        },
        "step.released" => RunEvent::StepReleased { node: node_id()? },
        "step.leased" => {
            let (attempt, generation) = fence(d)?;
            RunEvent::StepLeased {
                node: node_id()?,
                attempt,
                generation,
            }
        }
        "step.started" => {
            let (attempt, generation) = fence(d)?;
            RunEvent::StepStarted {
                node: node_id()?,
                attempt,
                generation,
            }
        }
        "step.completed" => {
            let (attempt, generation) = fence(d)?;
            RunEvent::StepCompleted {
                node: node_id()?,
                attempt,
                generation,
            }
        }
        "step.retry_scheduled" => {
            let (attempt, generation) = fence(d)?;
            RunEvent::StepRetryScheduled {
                node: node_id()?,
                attempt,
                generation,
                code: FailureCode::parse(text(d, "code")?).ok()?,
                not_before: time_of(d, "not_before_us")?,
            }
        }
        "step.retry_due" => RunEvent::StepRetryDue { node: node_id()? },
        "step.failed" => RunEvent::StepFailed {
            node: node_id()?,
            code: FailureCode::parse(text(d, "code")?).ok()?,
            fence: match d.get("fence")? {
                Value::Null => None,
                Value::Object(f) => {
                    let (attempt, generation) = fence(f)?;
                    Some(Fence {
                        attempt,
                        generation,
                    })
                }
                _ => return None,
            },
        },
        "condition.evaluated" => RunEvent::ConditionEvaluated {
            node: node_id()?,
            outcome: d.get("outcome")?.as_bool()?,
        },
        "timer.started" => RunEvent::TimerStarted {
            node: node_id()?,
            due_at: time_of(d, "due_at_us")?,
        },
        "timer.fired" => RunEvent::TimerFired { node: node_id()? },
        "wait.started" => RunEvent::WaitStarted {
            node: node_id()?,
            correlation_key: CorrelationKey::parse(text(d, "correlation_key")?).ok()?,
            deadline: time_of(d, "deadline_us")?,
        },
        "wait.satisfied" => RunEvent::WaitSatisfied { node: node_id()? },
        "wait.timed_out" => RunEvent::WaitTimedOut {
            node: node_id()?,
            handled: d.get("handled")?.as_bool()?,
        },
        "parallel.forked" => RunEvent::ParallelForked {
            node: node_id()?,
            branches: nodes_of(d, "branches")?,
        },
        "parallel.joined" => RunEvent::ParallelJoined {
            node: node_id()?,
            outcome: match text(d, "outcome")? {
                "succeeded" => JoinOutcome::Succeeded,
                "failed" => JoinOutcome::Failed,
                _ => return None,
            },
        },
        "child.started" => RunEvent::ChildStarted {
            node: node_id()?,
            child: text(d, "child")?.parse::<RunId>().ok()?,
        },
        "child.finished" => RunEvent::ChildFinished {
            node: node_id()?,
            child: text(d, "child")?.parse::<RunId>().ok()?,
            outcome: match d.get("outcome")? {
                Value::Null => ChildOutcome::Succeeded,
                Value::String(code) => ChildOutcome::Failed(FailureCode::parse(code).ok()?),
                _ => return None,
            },
        },
        "end.reached" => RunEvent::EndReached { node: node_id()? },
        "step.cancel_requested" => RunEvent::StepCancelRequested { node: node_id()? },
        "step.cancelled" => RunEvent::StepCancelled { node: node_id()? },
        "run.paused" => RunEvent::RunPaused,
        "run.resumed" => RunEvent::RunResumed,
        "run.stop_requested" => RunEvent::RunStopRequested {
            reason: match text(d, "reason")? {
                "CANCELLED" => StopReason::Cancelled,
                "TIMED_OUT" => StopReason::TimedOut,
                "FAILED" => StopReason::Failed,
                _ => return None,
            },
            code: match d.get("code")? {
                Value::Null => None,
                Value::String(c) => Some(FailureCode::parse(c).ok()?),
                _ => return None,
            },
        },
        "run.finished" => RunEvent::RunFinished {
            status: text(d, "status")?.parse::<RunStatus>().ok()?,
        },
        _ => return None,
    })
}
