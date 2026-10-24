//! Journal events as rows: a name, the node and attempt they concern, and a JSON object with the
//! event's own fields. [`decode`] is the exact inverse of [`encode`], so a stored journal can be
//! replayed into a `RunState` and compared with the projections (`verify_run`).
//!
//! Times are stored as microseconds since the epoch, the precision PostgreSQL keeps.

use sagat_domain::execution::{
    ApprovalDecision, ApprovalReasonCode, ApprovalReceiptId, ApprovalRequestId, ChildOutcome,
    CollectionDrainReason, CollectionOutcome, CorrelationKey, ExecutionInstanceId, FailureCode,
    Fence, InstancePath, JoinOutcome, LoopContinuation, LoopDrainReason, LoopKind, LoopOutcome,
    PlannedEntry, ReconcileResult, ReviewOutcome, RunEvent, RunId, RunStatus, ScopedPlannedEntry,
    StopReason, SwitchSelection,
};
use sagat_domain::identity::Subject;
use sagat_domain::saga::{ReconcileVerdict, SagaOutcome};
use sagat_domain::workflow::{ActivityName, Digest, NodeId};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

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
        RunEvent::ExecutionChainStarted {
            max_segments,
            original_deadline,
        } => json!({
            "max_segments": max_segments,
            "original_deadline_us": micros(*original_deadline),
        }),
        RunEvent::ExecutionChainSuccessorLinked {
            chain_id,
            predecessor,
            segment_index,
            max_segments,
            original_deadline,
            input_digest,
            state_digest,
        } => json!({
            "chain_id": chain_id.to_string(),
            "predecessor": predecessor.to_string(),
            "segment_index": segment_index,
            "max_segments": max_segments,
            "original_deadline_us": micros(*original_deadline),
            "input_digest": input_digest.to_hex(),
            "state_digest": state_digest.as_ref().map(Digest::to_hex),
        }),
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
        RunEvent::StateUpdated {
            operation,
            before_revision,
            after_revision,
            state_digest,
        } => json!({
            "operation": operation.as_str(),
            "before_revision": before_revision,
            "after_revision": after_revision,
            "state_digest": state_digest.to_hex(),
        }),
        RunEvent::RunContinued {
            successor,
            input_digest,
            state_digest,
        } => json!({
            "successor": successor.to_string(),
            "input_digest": input_digest.to_hex(),
            "state_digest": state_digest.as_ref().map(Digest::to_hex),
        }),
        RunEvent::ContinuationSelected {
            successor,
            input_digest,
            state_digest,
            ..
        } => json!({
            "successor": successor.to_string(),
            "input_digest": input_digest.to_hex(),
            "state_digest": state_digest.as_ref().map(Digest::to_hex),
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
        RunEvent::ApprovalOpened {
            request_id,
            deadline,
            ..
        } => json!({
            "request_id": request_id.to_string(),
            "deadline_us": micros(*deadline),
        }),
        RunEvent::ApprovalDecided {
            request_id,
            receipt_id,
            decision,
            actor,
            reason_code,
            ..
        } => json!({
            "request_id": request_id.to_string(),
            "receipt_id": receipt_id.to_string(),
            "decision": decision.as_str(),
            "actor": actor.as_str(),
            "reason_code": reason_code.as_ref().map(ApprovalReasonCode::as_str),
        }),
        RunEvent::ApprovalExpired { request_id, .. } => json!({
            "request_id": request_id.to_string(),
        }),
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
            CollectionDrainReason::RunDataLimit => json!({"reason": "run_data_limit"}),
            CollectionDrainReason::StorageQuotaExceeded => {
                json!({"reason": "storage_quota_exceeded"})
            }
            CollectionDrainReason::Cancelled => json!({"reason": "cancelled"}),
        },
        RunEvent::CollectionIterationDrained { index, .. } => json!({"index": index}),
        RunEvent::CollectionFinished {
            outcome,
            result_digest,
            ..
        } => match outcome {
            CollectionOutcome::Succeeded => json!({
                "outcome": "succeeded",
                "result_digest": result_digest.map(|digest| digest.to_hex()),
            }),
            CollectionOutcome::Failed(code) => {
                json!({
                    "outcome": "failed",
                    "code": code.as_str(),
                    "result_digest": result_digest.map(|digest| digest.to_hex()),
                })
            }
            CollectionOutcome::Cancelled => json!({
                "outcome": "cancelled",
                "result_digest": result_digest.map(|digest| digest.to_hex()),
            }),
        },
        RunEvent::LoopEntered {
            kind,
            max_iterations,
            entered_at,
            deadline,
            initial_carry_digest,
            initial_condition,
            ..
        } => json!({
            "kind": match kind { LoopKind::While => "while", LoopKind::RepeatUntil => "repeatUntil" },
            "max_iterations": max_iterations,
            "entered_at_us": micros(*entered_at),
            "deadline_us": micros(*deadline),
            "initial_carry_digest": initial_carry_digest.to_hex(),
            "initial_condition": initial_condition,
        }),
        RunEvent::LoopIterationAdmitted { index, entry, .. } => json!({
            "index": index, "entry": instance_value(entry),
        }),
        RunEvent::LoopIterationReturned {
            index,
            return_instance,
            ..
        } => json!({
            "index": index, "return_instance": instance_value(return_instance),
        }),
        RunEvent::LoopIterationCheckpointed {
            index,
            return_instance,
            result_digest,
            carry_digest,
            condition,
            checked_at,
            continuation,
            ..
        } => json!({
            "index": index,
            "return_instance": instance_value(return_instance),
            "result_digest": result_digest.to_hex(),
            "carry_digest": carry_digest.to_hex(),
            "condition": condition,
            "checked_at_us": micros(*checked_at),
            "continuation": match continuation {
                LoopContinuation::Complete => json!({"kind": "complete"}),
                LoopContinuation::Continue => json!({"kind": "continue"}),
                LoopContinuation::DelayUntil(at) => json!({"kind": "delay", "at_us": micros(*at)}),
                LoopContinuation::IterationLimit => json!({"kind": "iteration_limit"}),
                LoopContinuation::InstanceLimit => json!({"kind": "instance_limit"}),
            },
        }),
        RunEvent::LoopDelayElapsed { next_iteration, .. } => {
            json!({"next_iteration": next_iteration})
        }
        RunEvent::LoopDrainStarted { reason, .. } => match reason {
            LoopDrainReason::Failed { iteration, code } => json!({
                "reason": "failed", "iteration": iteration, "code": code.as_str(),
            }),
            LoopDrainReason::TimedOut => json!({"reason": "timed_out"}),
            LoopDrainReason::Cancelled => json!({"reason": "cancelled"}),
        },
        RunEvent::LoopIterationDrained { index, .. } => json!({"index": index}),
        RunEvent::LoopFinished { outcome, .. } => match outcome {
            LoopOutcome::Failed(code) => json!({"outcome": "failed", "code": code.as_str()}),
            LoopOutcome::Cancelled => json!({"outcome": "cancelled"}),
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
        RunEvent::ScopeCompensationPlanned {
            cause,
            entries,
            irreversible,
            ..
        } => json!({
            "cause": cause.as_str(),
            "entries": entries
                .iter()
                .map(|e| json!({
                    "instance": instance_value(&e.instance),
                    "after": e.after.iter().map(instance_value).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
            "irreversible": irreversible.iter().map(instance_value).collect::<Vec<_>>(),
        }),
        RunEvent::ScopeCompensated { outcome, .. } => json!({ "outcome": outcome.as_str() }),
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
        "execution_chain.started" => RunEvent::ExecutionChainStarted {
            max_segments: u16::try_from(number(d, "max_segments")?).ok()?,
            original_deadline: time_of(d, "original_deadline_us")?,
        },
        "execution_chain.successor_linked" => RunEvent::ExecutionChainSuccessorLinked {
            chain_id: text(d, "chain_id")?.parse::<RunId>().ok()?,
            predecessor: text(d, "predecessor")?.parse::<RunId>().ok()?,
            segment_index: u16::try_from(number(d, "segment_index")?).ok()?,
            max_segments: u16::try_from(number(d, "max_segments")?).ok()?,
            original_deadline: time_of(d, "original_deadline_us")?,
            input_digest: Digest::from_hex(text(d, "input_digest")?)?,
            state_digest: match d.get("state_digest")? {
                Value::Null => None,
                Value::String(value) => Some(Digest::from_hex(value)?),
                _ => return None,
            },
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
        "state.updated" => {
            let before_revision = number(d, "before_revision")?;
            let after_revision = number(d, "after_revision")?;
            if before_revision.checked_add(1) != Some(after_revision) {
                return None;
            }
            RunEvent::StateUpdated {
                operation: ActivityName::parse(text(d, "operation")?).ok()?,
                before_revision,
                after_revision,
                state_digest: Digest::from_hex(text(d, "state_digest")?)?,
            }
        }
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
        "approval.opened" => RunEvent::ApprovalOpened {
            node: node_id()?,
            request_id: ApprovalRequestId::new(Uuid::parse_str(text(d, "request_id")?).ok()?),
            deadline: time_of(d, "deadline_us")?,
        },
        "approval.decided" => RunEvent::ApprovalDecided {
            node: node_id()?,
            request_id: ApprovalRequestId::new(Uuid::parse_str(text(d, "request_id")?).ok()?),
            receipt_id: ApprovalReceiptId::new(Uuid::parse_str(text(d, "receipt_id")?).ok()?),
            decision: match text(d, "decision")? {
                "APPROVE" => ApprovalDecision::Approve,
                "REJECT" => ApprovalDecision::Reject,
                _ => return None,
            },
            actor: Subject::parse(text(d, "actor")?).ok()?,
            reason_code: match d.get("reason_code")? {
                Value::Null => None,
                Value::String(code) => Some(ApprovalReasonCode::parse(code).ok()?),
                _ => return None,
            },
        },
        "approval.expired" => RunEvent::ApprovalExpired {
            node: node_id()?,
            request_id: ApprovalRequestId::new(Uuid::parse_str(text(d, "request_id")?).ok()?),
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
                "run_data_limit" => CollectionDrainReason::RunDataLimit,
                "storage_quota_exceeded" => CollectionDrainReason::StorageQuotaExceeded,
                "cancelled" => CollectionDrainReason::Cancelled,
                _ => return None,
            },
        },
        "collection.iteration_drained" => RunEvent::CollectionIterationDrained {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
        },
        "collection.finished" => {
            let outcome = match text(d, "outcome")? {
                "succeeded" => CollectionOutcome::Succeeded,
                "failed" => CollectionOutcome::Failed(FailureCode::parse(text(d, "code")?).ok()?),
                "cancelled" => CollectionOutcome::Cancelled,
                _ => return None,
            };
            let result_digest = match outcome {
                CollectionOutcome::Succeeded => Some(Digest::from_hex(text(d, "result_digest")?)?),
                CollectionOutcome::Failed(_) | CollectionOutcome::Cancelled
                    if d.get("result_digest").is_none_or(Value::is_null) =>
                {
                    None
                }
                CollectionOutcome::Failed(_) | CollectionOutcome::Cancelled => return None,
            };
            RunEvent::CollectionFinished {
                node: node_id()?,
                outcome,
                result_digest,
            }
        }
        "loop.entered" => RunEvent::LoopEntered {
            node: node_id()?,
            kind: match text(d, "kind")? {
                "while" => LoopKind::While,
                "repeatUntil" => LoopKind::RepeatUntil,
                _ => return None,
            },
            max_iterations: u32::try_from(number(d, "max_iterations")?).ok()?,
            entered_at: time_of(d, "entered_at_us")?,
            deadline: time_of(d, "deadline_us")?,
            initial_carry_digest: Digest::from_hex(text(d, "initial_carry_digest")?)?,
            initial_condition: match d.get("initial_condition")? {
                Value::Null => None,
                Value::Bool(value) => Some(*value),
                _ => return None,
            },
        },
        "loop.iteration_admitted" => RunEvent::LoopIterationAdmitted {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
            entry: instance_of(d.get("entry")?)?,
        },
        "loop.iteration_returned" => RunEvent::LoopIterationReturned {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
            return_instance: instance_of(d.get("return_instance")?)?,
        },
        "loop.iteration_checkpointed" => {
            let continuation = d.get("continuation")?.as_object()?;
            RunEvent::LoopIterationCheckpointed {
                node: node_id()?,
                index: u32::try_from(number(d, "index")?).ok()?,
                return_instance: instance_of(d.get("return_instance")?)?,
                result_digest: Digest::from_hex(text(d, "result_digest")?)?,
                carry_digest: Digest::from_hex(text(d, "carry_digest")?)?,
                condition: d.get("condition")?.as_bool()?,
                checked_at: time_of(d, "checked_at_us")?,
                continuation: match text(continuation, "kind")? {
                    "complete" => LoopContinuation::Complete,
                    "continue" => LoopContinuation::Continue,
                    "delay" => LoopContinuation::DelayUntil(time_of(continuation, "at_us")?),
                    "iteration_limit" => LoopContinuation::IterationLimit,
                    "instance_limit" => LoopContinuation::InstanceLimit,
                    _ => return None,
                },
            }
        }
        "loop.delay_elapsed" => RunEvent::LoopDelayElapsed {
            node: node_id()?,
            next_iteration: u32::try_from(number(d, "next_iteration")?).ok()?,
        },
        "loop.drain_started" => RunEvent::LoopDrainStarted {
            node: node_id()?,
            reason: match text(d, "reason")? {
                "failed" => LoopDrainReason::Failed {
                    iteration: u32::try_from(number(d, "iteration")?).ok()?,
                    code: FailureCode::parse(text(d, "code")?).ok()?,
                },
                "timed_out" => LoopDrainReason::TimedOut,
                "cancelled" => LoopDrainReason::Cancelled,
                _ => return None,
            },
        },
        "loop.iteration_drained" => RunEvent::LoopIterationDrained {
            node: node_id()?,
            index: u32::try_from(number(d, "index")?).ok()?,
        },
        "loop.finished" => RunEvent::LoopFinished {
            node: node_id()?,
            outcome: match text(d, "outcome")? {
                "failed" => LoopOutcome::Failed(FailureCode::parse(text(d, "code")?).ok()?),
                "cancelled" => LoopOutcome::Cancelled,
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
        "scope.compensation_planned" => RunEvent::ScopeCompensationPlanned {
            node: node_id()?,
            cause: FailureCode::parse(text(d, "cause")?).ok()?,
            entries: d
                .get("entries")?
                .as_array()?
                .iter()
                .map(|entry| {
                    let entry = entry.as_object()?;
                    Some(ScopedPlannedEntry {
                        instance: instance_of(entry.get("instance")?)?,
                        after: entry
                            .get("after")?
                            .as_array()?
                            .iter()
                            .map(instance_of)
                            .collect::<Option<Vec<_>>>()?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            irreversible: d
                .get("irreversible")?
                .as_array()?
                .iter()
                .map(instance_of)
                .collect::<Option<Vec<_>>>()?,
        },
        "scope.compensated" => RunEvent::ScopeCompensated {
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
        "run.continued" => RunEvent::RunContinued {
            successor: text(d, "successor")?.parse::<RunId>().ok()?,
            input_digest: Digest::from_hex(text(d, "input_digest")?)?,
            state_digest: match d.get("state_digest")? {
                Value::Null => None,
                Value::String(value) => Some(Digest::from_hex(value)?),
                _ => return None,
            },
        },
        "continuation.selected" => RunEvent::ContinuationSelected {
            node: node_id()?,
            successor: text(d, "successor")?.parse::<RunId>().ok()?,
            input_digest: Digest::from_hex(text(d, "input_digest")?)?,
            state_digest: match d.get("state_digest")? {
                Value::Null => None,
                Value::String(value) => Some(Digest::from_hex(value)?),
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
        LoopContinuation, LoopDrainReason, LoopKind, LoopOutcome, RunEvent, ScopedPlannedEntry,
        SwitchSelection,
    };
    use sagat_domain::saga::SagaOutcome;
    use sagat_domain::workflow::{ActivityName, Digest, NodeId};
    use serde_json::json;

    use super::{decode, encode};

    fn node(value: &str) -> NodeId {
        NodeId::parse(value).expect("test node")
    }

    #[test]
    fn raw_collection_data_limit_drain_decodes_without_a_fictitious_member() {
        let detail = json!({"reason":"run_data_limit"});
        let event = decode("collection.drain_started", Some("items"), &detail)
            .expect("an entered empty collection can exceed the run-data limit without a member");
        let encoded = encode(&event);
        assert_eq!(encoded.event_type, "collection.drain_started");
        assert_eq!(encoded.node.as_deref(), Some("items"));
        assert_eq!(encoded.detail, detail);
        let cause = event.cause().expect("typed boundary failure");
        assert_eq!(
            (cause.kind, cause.code.as_str()),
            ("failure", "RUN_DATA_LIMIT")
        );
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
                return_instance: returned.clone(),
                result_digest: digest,
            },
            RunEvent::CollectionDrainStarted {
                node: owner.clone(),
                reason: CollectionDrainReason::Failed {
                    iteration: 1,
                    code: failure.clone(),
                },
            },
            RunEvent::CollectionDrainStarted {
                node: owner.clone(),
                reason: CollectionDrainReason::StorageQuotaExceeded,
            },
            RunEvent::CollectionIterationDrained {
                node: owner.clone(),
                index: 1,
            },
            RunEvent::CollectionFinished {
                node: owner.clone(),
                outcome: CollectionOutcome::Failed(failure),
                result_digest: None,
            },
            RunEvent::CollectionFinished {
                node: owner,
                outcome: CollectionOutcome::Succeeded,
                result_digest: Some(digest),
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
        for detail in [
            json!({"outcome": "succeeded"}),
            json!({"outcome": "succeeded", "result_digest": null}),
            json!({"outcome": "failed", "code": "ITEM_REJECTED", "result_digest": digest.to_hex()}),
            json!({"outcome": "cancelled", "result_digest": digest.to_hex()}),
        ] {
            assert!(decode("collection.finished", Some("items"), &detail).is_none());
        }
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

    #[test]
    fn collection_run_data_limit_drain_has_exact_codec_and_cause() {
        let event = RunEvent::CollectionDrainStarted {
            node: node("items"),
            reason: CollectionDrainReason::RunDataLimit,
        };

        let encoded = encode(&event);
        assert_eq!(encoded.event_type, "collection.drain_started");
        assert_eq!(encoded.node.as_deref(), Some("items"));
        assert_eq!(encoded.detail, json!({"reason": "run_data_limit"}));
        assert_eq!(
            decode(encoded.event_type, encoded.node.as_deref(), &encoded.detail),
            Some(event.clone())
        );
        let cause = event.cause().expect("data-limit drain cause");
        assert_eq!(
            (cause.kind, cause.code.as_str()),
            ("failure", "RUN_DATA_LIMIT")
        );
    }

    #[test]
    fn scope_compensation_events_are_exact_codec_inverses() {
        let owner = node("items");
        let at = |index: u32, name: &str| {
            ExecutionInstanceId::in_path(
                InstancePath::root()
                    .child(owner.clone(), index)
                    .expect("bounded path"),
                node(name),
            )
        };
        let planned = RunEvent::ScopeCompensationPlanned {
            node: owner.clone(),
            cause: FailureCode::parse("PAYMENT_DECLINED").expect("failure code"),
            entries: vec![
                ScopedPlannedEntry {
                    instance: at(0, "ship"),
                    after: Vec::new(),
                },
                ScopedPlannedEntry {
                    instance: at(0, "reserve"),
                    after: vec![at(0, "ship"), at(1, "reserve")],
                },
            ],
            irreversible: vec![at(2, "notify")],
        };
        let compensated = RunEvent::ScopeCompensated {
            node: owner,
            outcome: SagaOutcome::Partial,
        };
        for event in [planned, compensated] {
            let encoded = encode(&event);
            assert_eq!(
                decode(encoded.event_type, encoded.node.as_deref(), &encoded.detail),
                Some(event)
            );
        }
        for detail in [
            json!({}),
            json!({"cause": "X", "entries": [{"instance": {"node": "a", "path": "bad"}, "after": []}], "irreversible": []}),
            json!({"cause": "bad cause", "entries": [], "irreversible": []}),
        ] {
            assert!(decode("scope.compensation_planned", Some("items"), &detail).is_none());
        }
        assert!(
            decode(
                "scope.compensated",
                Some("items"),
                &json!({"outcome": "NOPE"})
            )
            .is_none()
        );
    }

    #[test]
    fn loop_events_preserve_checkpoints_and_next_actions() {
        let owner = node("poll");
        let path = InstancePath::root().child(owner.clone(), 0).expect("path");
        let entry = ExecutionInstanceId::in_path(path.clone(), node("work"));
        let returned = ExecutionInstanceId::in_path(path, node("return"));
        let digest = Digest::new([9; 32]);
        let failure = FailureCode::parse("PROVIDER_FAILED").expect("failure");
        let at = time::OffsetDateTime::UNIX_EPOCH;
        let events = [
            RunEvent::LoopEntered {
                node: owner.clone(),
                kind: LoopKind::While,
                max_iterations: 3,
                entered_at: at,
                deadline: at + time::Duration::seconds(60),
                initial_carry_digest: digest,
                initial_condition: Some(true),
            },
            RunEvent::LoopEntered {
                node: owner.clone(),
                kind: LoopKind::RepeatUntil,
                max_iterations: 1,
                entered_at: at,
                deadline: at + time::Duration::seconds(60),
                initial_carry_digest: digest,
                initial_condition: None,
            },
            RunEvent::LoopIterationAdmitted {
                node: owner.clone(),
                index: 0,
                entry,
            },
            RunEvent::LoopIterationReturned {
                node: owner.clone(),
                index: 0,
                return_instance: returned.clone(),
            },
            RunEvent::LoopIterationCheckpointed {
                node: owner.clone(),
                index: 0,
                return_instance: returned.clone(),
                result_digest: digest,
                carry_digest: digest,
                condition: true,
                checked_at: at,
                continuation: LoopContinuation::DelayUntil(at + time::Duration::seconds(1)),
            },
            RunEvent::LoopIterationCheckpointed {
                node: owner.clone(),
                index: 0,
                return_instance: returned.clone(),
                result_digest: digest,
                carry_digest: digest,
                condition: false,
                checked_at: at,
                continuation: LoopContinuation::Complete,
            },
            RunEvent::LoopIterationCheckpointed {
                node: owner.clone(),
                index: 0,
                return_instance: returned.clone(),
                result_digest: digest,
                carry_digest: digest,
                condition: true,
                checked_at: at,
                continuation: LoopContinuation::Continue,
            },
            RunEvent::LoopIterationCheckpointed {
                node: owner.clone(),
                index: 0,
                return_instance: returned,
                result_digest: digest,
                carry_digest: digest,
                condition: true,
                checked_at: at,
                continuation: LoopContinuation::IterationLimit,
            },
            RunEvent::LoopDelayElapsed {
                node: owner.clone(),
                next_iteration: 1,
            },
            RunEvent::LoopDrainStarted {
                node: owner.clone(),
                reason: LoopDrainReason::Failed {
                    iteration: 0,
                    code: failure.clone(),
                },
            },
            RunEvent::LoopDrainStarted {
                node: owner.clone(),
                reason: LoopDrainReason::Cancelled,
            },
            RunEvent::LoopDrainStarted {
                node: owner.clone(),
                reason: LoopDrainReason::TimedOut,
            },
            RunEvent::LoopIterationDrained {
                node: owner.clone(),
                index: 0,
            },
            RunEvent::LoopFinished {
                node: owner.clone(),
                outcome: LoopOutcome::Failed(failure),
            },
            RunEvent::LoopFinished {
                node: owner,
                outcome: LoopOutcome::Cancelled,
            },
        ];
        for event in events {
            let encoded = encode(&event);
            assert_eq!(
                decode(encoded.event_type, encoded.node.as_deref(), &encoded.detail),
                Some(event)
            );
        }
        assert!(decode("loop.entered", Some("poll"), &json!({})).is_none());
    }

    #[test]
    fn state_update_keeps_only_bounded_metadata_and_digest() {
        let event = RunEvent::StateUpdated {
            operation: ActivityName::parse("balance.credit").expect("operation"),
            before_revision: 8,
            after_revision: 9,
            state_digest: Digest::new([23; 32]),
        };
        let encoded = encode(&event);
        assert_eq!(encoded.event_type, "state.updated");
        assert_eq!(encoded.node, None);
        assert_eq!(
            decode(encoded.event_type, None, &encoded.detail),
            Some(event)
        );
        assert_eq!(encoded.detail.as_object().expect("detail").len(), 4);
        assert!(
            decode(
                "state.updated",
                None,
                &json!({
                    "operation": "balance.credit",
                    "before_revision": 8,
                    "after_revision": 10,
                    "state_digest": Digest::new([23; 32]).to_hex(),
                }),
            )
            .is_none()
        );
    }

    #[test]
    fn execution_chain_events_are_exact_codec_inverses() {
        let chain = sagat_domain::execution::RunId::generate();
        let predecessor = sagat_domain::execution::RunId::generate();
        let successor = sagat_domain::execution::RunId::generate();
        let deadline = time::OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1);
        let input = Digest::new([31; 32]);
        let state = Digest::new([32; 32]);
        let events = [
            RunEvent::ExecutionChainStarted {
                max_segments: 8,
                original_deadline: deadline,
            },
            RunEvent::ExecutionChainSuccessorLinked {
                chain_id: chain,
                predecessor,
                segment_index: 4,
                max_segments: 8,
                original_deadline: deadline,
                input_digest: input,
                state_digest: Some(state),
            },
            RunEvent::RunContinued {
                successor,
                input_digest: input,
                state_digest: Some(state),
            },
            RunEvent::RunContinued {
                successor,
                input_digest: input,
                state_digest: None,
            },
            RunEvent::ContinuationSelected {
                node: NodeId::parse("rollover").expect("node"),
                successor,
                input_digest: input,
                state_digest: Some(state),
            },
        ];
        for event in events {
            let encoded = encode(&event);
            assert_eq!(
                decode(encoded.event_type, encoded.node.as_deref(), &encoded.detail),
                Some(event)
            );
        }
        assert!(decode("execution_chain.started", None, &json!({})).is_none());
        assert!(
            decode(
                "run.continued",
                None,
                &json!({
                    "successor": successor.to_string(),
                    "input_digest": "not-a-digest",
                    "state_digest": null,
                }),
            )
            .is_none()
        );
    }
}
