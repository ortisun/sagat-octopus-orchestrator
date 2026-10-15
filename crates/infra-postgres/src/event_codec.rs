//! Journal events as rows: a name, the node and attempt they concern, and a JSON object with the
//! event's own fields. [`decode`] is the exact inverse of [`encode`], so a stored journal can be
//! replayed into a `RunState` and compared with the projections (`verify_run`).
//!
//! Times are stored as microseconds since the epoch, the precision PostgreSQL keeps.

use sagat_domain::execution::{
    ChildOutcome, CollectionDrainReason, CollectionOutcome, CorrelationKey, ExecutionInstanceId,
    FailureCode, Fence, InstancePath, JoinOutcome, PlannedEntry, ReconcileResult, ReviewOutcome,
    RunEvent, RunId, RunStatus, StopReason, SwitchSelection,
};
use sagat_domain::saga::{ReconcileVerdict, SagaOutcome};
use sagat_domain::workflow::{Digest, NodeId};
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

fn instance_value(instance: &ExecutionInstanceId) -> Value {
    json!({
        "node": instance.node().as_str(),
        "path": instance.path().segments().iter().map(|segment| json!({
            "scope": segment.scope_node().as_str(),
            "index": segment.iteration_index(),
        })).collect::<Vec<_>>(),
    })
}

fn instance_of(value: &Value) -> Option<ExecutionInstanceId> {
    let object = value.as_object()?;
    let node = NodeId::parse_stored(object.get("node")?.as_str()?).ok()?;
    let mut path = InstancePath::root();
    for segment in object.get("path")?.as_array()? {
        let segment = segment.as_object()?;
        let scope = NodeId::parse_stored(segment.get("scope")?.as_str()?).ok()?;
        let index = u32::try_from(segment.get("index")?.as_u64()?).ok()?;
        path = path.child(scope, index).ok()?;
    }
    Some(ExecutionInstanceId::in_path(path, node))
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
        RunEvent::SwitchEvaluated { selection, .. } => json!({
            "case_id": match selection {
                SwitchSelection::Case(id) => Some(id.as_str()),
                SwitchSelection::Default => None,
            }
        }),
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
        RunEvent::CollectionEntered {
            item_count,
            max_items,
            max_concurrency,
            snapshot_digest,
            ..
        } => json!({
            "item_count": item_count,
            "max_items": max_items,
            "max_concurrency": max_concurrency,
            "snapshot_digest": snapshot_digest.to_hex(),
        }),
        RunEvent::CollectionIterationAdmitted {
            index,
            entry,
            item_digest,
            ..
        } => json!({
            "index": index,
            "entry": instance_value(entry),
            "item_digest": item_digest.to_hex(),
        }),
        RunEvent::CollectionIterationReturned {
            index,
            return_instance,
            ..
        } => json!({
            "index": index,
            "return_instance": instance_value(return_instance),
        }),
        RunEvent::CollectionIterationCheckpointed {
            index,
            return_instance,
            result_digest,
            ..
        } => json!({
            "index": index,
            "return_instance": instance_value(return_instance),
            "result_digest": result_digest.to_hex(),
        }),
        RunEvent::CollectionDrainStarted { reason, .. } => match reason {
            CollectionDrainReason::Failed { iteration, code } => json!({
                "reason": "failed", "iteration": iteration, "code": code.as_str(),
            }),
            CollectionDrainReason::Cancelled => json!({"reason": "cancelled"}),
        },
        RunEvent::CollectionIterationDrained { index, .. } => json!({"index": index}),
        RunEvent::CollectionFinished { outcome, .. } => match outcome {
            CollectionOutcome::Succeeded => json!({"outcome": "succeeded"}),
            CollectionOutcome::Failed(code) => {
                json!({"outcome": "failed", "code": code.as_str()})
            }
            CollectionOutcome::Cancelled => json!({"outcome": "cancelled"}),
        },
        RunEvent::ChildStarted { child, .. } => json!({ "child": child.to_string() }),
        RunEvent::ChildFinished { child, outcome, .. } => json!({
            "child": child.to_string(),
            "outcome": match outcome {
                ChildOutcome::Succeeded => Value::Null,
                ChildOutcome::Failed(code) => json!(code.as_str()),
            },
        }),
        RunEvent::RunStopRequested { reason, code } => json!({
            "reason": reason.as_str(),
            "code": code.as_ref().map(FailureCode::as_str),
        }),
        RunEvent::SagaEntered { begin, .. } => json!({ "begin": begin.as_str() }),
        RunEvent::SagaAborting { cause, .. } => json!({ "cause": cause.as_str() }),
        RunEvent::CompensationPlanned {
            entries,
            irreversible,
            ..
        } => json!({
            "entries": entries
                .iter()
                .map(|e| json!({ "node": e.node.as_str(), "after": names(&e.after) }))
                .collect::<Vec<_>>(),
            "irreversible": names(irreversible),
        }),
        RunEvent::SagaCompensated { outcome, .. } => json!({ "outcome": outcome.as_str() }),
        RunEvent::ReconciliationRequested { fence, cause, .. } => json!({
            "attempt": fence.attempt,
            "generation": fence.generation,
            "cause": cause.as_str(),
        }),
        RunEvent::ReconciliationConcluded {
            verdict, result, ..
        } => json!({
            "verdict": verdict.as_str(),
            "result": match result {
                ReconcileResult::Succeeded => json!({ "kind": "succeeded" }),
                ReconcileResult::RetryAt(at) => json!({ "kind": "retry_at", "at_us": micros(*at) }),
                ReconcileResult::Failed(code) => json!({ "kind": "failed", "code": code.as_str() }),
                ReconcileResult::Review(code) => json!({ "kind": "review", "code": code.as_str() }),
            },
        }),
        RunEvent::StepNeedsReview { code, fence, .. } => json!({
            "code": code.as_str(),
            "fence": fence.map(|f| json!({ "attempt": f.attempt, "generation": f.generation })),
        }),
        RunEvent::StepResolved { outcome, .. } => match outcome {
            ReviewOutcome::Succeeded => json!({ "outcome": "succeeded" }),
            ReviewOutcome::Failed(code) => json!({ "outcome": "failed", "code": code.as_str() }),
        },
        RunEvent::RunFinished { status } => json!({ "status": status.as_str() }),
        RunEvent::StepReleased { .. }
        | RunEvent::StepRetryDue { .. }
        | RunEvent::TimerFired { .. }
        | RunEvent::WaitSatisfied { .. }
        | RunEvent::EndReached { .. }
        | RunEvent::StepCancelRequested { .. }
        | RunEvent::StepCancelled { .. }
        | RunEvent::SagaCompleted { .. }
        | RunEvent::StepRetryRequested { .. }
        | RunEvent::StepSkipped { .. }
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
    NodeId::parse_stored(node?).ok()
}

fn nodes_of(detail: &Map<String, Value>, key: &str) -> Option<Vec<NodeId>> {
    detail
        .get(key)?
        .as_array()?
        .iter()
        .map(|v| NodeId::parse_stored(v.as_str()?).ok())
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
        "switch.evaluated" => RunEvent::SwitchEvaluated {
            node: node_id()?,
            selection: match d.get("case_id")? {
                Value::Null => SwitchSelection::Default,
                Value::String(id) if !id.is_empty() => {
                    SwitchSelection::Case(NodeId::parse(id).ok()?)
                }
                _ => return None,
            },
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
        "collection.entered" => RunEvent::CollectionEntered {
            node: node_id()?,
            item_count: u32::try_from(number(d, "item_count")?).ok()?,
            max_items: u32::try_from(number(d, "max_items")?).ok()?,
            max_concurrency: u16::try_from(number(d, "max_concurrency")?).ok()?,
            snapshot_digest: Digest::from_hex(text(d, "snapshot_digest")?)?,
        },
        "collection.iteration_admitted" => RunEvent::CollectionIterationAdmitted {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
            entry: instance_of(d.get("entry")?)?,
            item_digest: Digest::from_hex(text(d, "item_digest")?)?,
        },
        "collection.iteration_returned" => RunEvent::CollectionIterationReturned {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
            return_instance: instance_of(d.get("return_instance")?)?,
        },
        "collection.iteration_checkpointed" => RunEvent::CollectionIterationCheckpointed {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
            return_instance: instance_of(d.get("return_instance")?)?,
            result_digest: Digest::from_hex(text(d, "result_digest")?)?,
        },
        "collection.drain_started" => RunEvent::CollectionDrainStarted {
            node: node_id()?,
            reason: match text(d, "reason")? {
                "failed" => CollectionDrainReason::Failed {
                    iteration: u32::try_from(number(d, "iteration")?).ok()?,
                    code: FailureCode::parse(text(d, "code")?).ok()?,
                },
                "cancelled" => CollectionDrainReason::Cancelled,
                _ => return None,
            },
        },
        "collection.iteration_drained" => RunEvent::CollectionIterationDrained {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
        },
        "collection.finished" => RunEvent::CollectionFinished {
            node: node_id()?,
            outcome: match text(d, "outcome")? {
                "succeeded" => CollectionOutcome::Succeeded,
                "failed" => CollectionOutcome::Failed(FailureCode::parse(text(d, "code")?).ok()?),
                "cancelled" => CollectionOutcome::Cancelled,
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
        "saga.entered" => RunEvent::SagaEntered {
            node: node_id()?,
            begin: NodeId::parse_stored(text(d, "begin")?).ok()?,
        },
        "saga.completed" => RunEvent::SagaCompleted { node: node_id()? },
        "saga.aborting" => RunEvent::SagaAborting {
            node: node_id()?,
            cause: FailureCode::parse(text(d, "cause")?).ok()?,
        },
        "compensation.planned" => RunEvent::CompensationPlanned {
            node: node_id()?,
            entries: d
                .get("entries")?
                .as_array()?
                .iter()
                .map(|entry| {
                    let entry = entry.as_object()?;
                    Some(PlannedEntry {
                        node: NodeId::parse_stored(text(entry, "node")?).ok()?,
                        after: nodes_of(entry, "after")?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            irreversible: nodes_of(d, "irreversible")?,
        },
        "saga.compensated" => RunEvent::SagaCompensated {
            node: node_id()?,
            outcome: text(d, "outcome")?.parse::<SagaOutcome>().ok()?,
        },
        "reconciliation.requested" => {
            let (attempt, generation) = fence(d)?;
            RunEvent::ReconciliationRequested {
                node: node_id()?,
                fence: Fence {
                    attempt,
                    generation,
                },
                cause: FailureCode::parse(text(d, "cause")?).ok()?,
            }
        }
        "reconciliation.concluded" => {
            let result = d.get("result")?.as_object()?;
            RunEvent::ReconciliationConcluded {
                node: node_id()?,
                verdict: text(d, "verdict")?.parse::<ReconcileVerdict>().ok()?,
                result: match text(result, "kind")? {
                    "succeeded" => ReconcileResult::Succeeded,
                    "retry_at" => ReconcileResult::RetryAt(time_of(result, "at_us")?),
                    "failed" => {
                        ReconcileResult::Failed(FailureCode::parse(text(result, "code")?).ok()?)
                    }
                    "review" => {
                        ReconcileResult::Review(FailureCode::parse(text(result, "code")?).ok()?)
                    }
                    _ => return None,
                },
            }
        }
        "step.needs_review" => RunEvent::StepNeedsReview {
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
        "step.retry_requested" => RunEvent::StepRetryRequested { node: node_id()? },
        "step.skipped" => RunEvent::StepSkipped { node: node_id()? },
        "step.resolved" => RunEvent::StepResolved {
            node: node_id()?,
            outcome: match text(d, "outcome")? {
                "succeeded" => ReviewOutcome::Succeeded,
                "failed" => ReviewOutcome::Failed(FailureCode::parse(text(d, "code")?).ok()?),
                _ => return None,
            },
        },
        "run.stop_requested" => RunEvent::RunStopRequested {
            reason: text(d, "reason")?.parse::<StopReason>().ok()?,
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

#[cfg(test)]
mod tests {
    use sagat_domain::execution::{
        CollectionDrainReason, CollectionOutcome, ExecutionInstanceId, FailureCode, InstancePath,
        RunEvent, SwitchSelection,
    };
    use sagat_domain::workflow::{Digest, NodeId};
    use serde_json::json;

    use super::{decode, encode};

    fn node(value: &str) -> NodeId {
        NodeId::parse(value).expect("test node")
    }

    #[test]
    fn switch_case_and_default_are_exact_codec_inverses() {
        for (selection, detail) in [
            (
                SwitchSelection::Case(node("priority")),
                json!({"case_id": "priority"}),
            ),
            (SwitchSelection::Default, json!({"case_id": null})),
        ] {
            let event = RunEvent::SwitchEvaluated {
                node: node("choose"),
                selection,
            };
            let encoded = encode(&event);
            assert_eq!(encoded.event_type, "switch.evaluated");
            assert_eq!(encoded.node.as_deref(), Some("choose"));
            assert_eq!(encoded.detail, detail);
            assert_eq!(
                decode(encoded.event_type, encoded.node.as_deref(), &encoded.detail),
                Some(event)
            );
        }
    }

    #[test]
    fn malformed_switch_selections_are_not_replayed() {
        for detail in [
            json!({}),
            json!({"case_id": 1}),
            json!({"case_id": ""}),
            json!({"case_id": "invalid/case"}),
        ] {
            assert_eq!(decode("switch.evaluated", Some("choose"), &detail), None);
        }
    }

    #[test]
    fn collection_events_keep_digest_and_scoped_instance_fields() {
        let owner = node("items");
        let path = InstancePath::root()
            .child(owner.clone(), 1)
            .expect("bounded path");
        let entry = ExecutionInstanceId::in_path(path.clone(), node("work"));
        let returned = ExecutionInstanceId::in_path(path, node("return"));
        let digest = Digest::new([7; 32]);
        let failure = FailureCode::parse("ITEM_REJECTED").expect("failure code");
        let events = [
            RunEvent::CollectionEntered {
                node: owner.clone(),
                item_count: 2,
                max_items: 17,
                max_concurrency: 1,
                snapshot_digest: digest,
            },
            RunEvent::CollectionIterationAdmitted {
                node: owner.clone(),
                index: 1,
                entry,
                item_digest: digest,
            },
            RunEvent::CollectionIterationReturned {
                node: owner.clone(),
                index: 1,
                return_instance: returned.clone(),
            },
            RunEvent::CollectionIterationCheckpointed {
                node: owner.clone(),
                index: 1,
                return_instance: returned,
                result_digest: digest,
            },
            RunEvent::CollectionDrainStarted {
                node: owner.clone(),
                reason: CollectionDrainReason::Failed {
                    iteration: 1,
                    code: failure.clone(),
                },
            },
            RunEvent::CollectionIterationDrained {
                node: owner.clone(),
                index: 1,
            },
            RunEvent::CollectionFinished {
                node: owner,
                outcome: CollectionOutcome::Failed(failure),
            },
        ];
        for event in events {
            let encoded = encode(&event);
            assert_eq!(
                decode(encoded.event_type, encoded.node.as_deref(), &encoded.detail),
                Some(event)
            );
        }
        assert!(decode("collection.entered", Some("items"), &json!({})).is_none());
        for entry in [
            json!({"node": "work", "path": [{"scope": "items", "index": 10000}]}),
            json!({"node": "work", "path": [{"scope": "bad/name", "index": 0}]}),
            json!({"node": "work", "path": "items/0"}),
        ] {
            assert!(
                decode(
                    "collection.iteration_admitted",
                    Some("items"),
                    &json!({"index": 0, "entry": entry, "item_digest": digest.to_hex()})
                )
                .is_none()
            );
        }
    }
}
