//! Turns a schema-valid document into the domain model.
//!
//! The schema already guarantees the shape of every known node kind, but it allows extra fields on
//! nodes and does not know which kinds this engine can run, so conversion still reads defensively
//! and reports anything unexpected as an issue instead of panicking.

use std::time::Duration;

use sagat_application::definitions::{IrError, IrIssue};
use sagat_domain::identity::Slug;
use sagat_domain::workflow::{
    ActivityName, BackoffStrategy, CancelPolicy, EventType, Expression, JoinPolicy, Node, NodeId,
    NodeKind, RetryPolicy, WorkflowDefinition,
};
use serde_json::{Map, Value};

fn issue(path: String, message: impl Into<String>) -> IrError {
    IrError::Schema(vec![IrIssue {
        path,
        message: message.into(),
    }])
}

fn text<'a>(node: &'a Map<String, Value>, field: &str, at: &str) -> Result<&'a str, IrError> {
    node.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| issue(format!("{at}/{field}"), "expected a string"))
}

fn optional_text<'a>(
    node: &'a Map<String, Value>,
    field: &str,
    at: &str,
) -> Result<Option<&'a str>, IrError> {
    match node.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(issue(format!("{at}/{field}"), "expected a string")),
    }
}

fn millis(node: &Map<String, Value>, field: &str, at: &str) -> Result<Duration, IrError> {
    node.get(field)
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .ok_or_else(|| {
            issue(
                format!("{at}/{field}"),
                "expected a positive integer number of milliseconds",
            )
        })
}

fn node_id(value: &str, at: &str) -> Result<NodeId, IrError> {
    NodeId::parse(value).map_err(|e| issue(at.to_owned(), e.to_string()))
}

fn expression(value: &str, at: &str) -> Result<Expression, IrError> {
    Expression::parse(value).map_err(|e| issue(at.to_owned(), e.to_string()))
}

fn reference(node: &Map<String, Value>, field: &str, at: &str) -> Result<Option<NodeId>, IrError> {
    optional_text(node, field, at)?
        .map(|v| node_id(v, &format!("{at}/{field}")))
        .transpose()
}

/// Why a node kind that exists in the schema is not executed yet.
fn unsupported(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "saga" => "the saga engine arrives with feature 003",
        "script" => "inline scripts arrive with feature 005",
        "foreach" => "bounded loops are not part of any planned feature yet",
        "humanApproval" => "human approval is not part of any planned feature yet",
        _ => return None,
    })
}

fn retry(node: &Map<String, Value>, at: &str) -> Result<RetryPolicy, IrError> {
    let Some(Value::Object(spec)) = node.get("retry") else {
        return Ok(RetryPolicy::DEFAULT);
    };
    let at = format!("{at}/retry");
    let defaults = RetryPolicy::DEFAULT;
    let max_attempts = match spec.get("maxAttempts") {
        None => defaults.max_attempts(),
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| issue(format!("{at}/maxAttempts"), "expected a positive integer"))?,
    };
    let initial = if spec.contains_key("initialDelayMs") {
        millis(spec, "initialDelayMs", &at)?
    } else {
        defaults.initial_delay()
    };
    let max = if spec.contains_key("maxDelayMs") {
        millis(spec, "maxDelayMs", &at)?
    } else {
        // Without an explicit ceiling, never let the default (1 minute) sit below the initial delay.
        defaults.max_delay().max(initial)
    };
    let strategy = match optional_text(spec, "strategy", &at)? {
        None | Some("exponential") => BackoffStrategy::Exponential,
        Some("fixed") => BackoffStrategy::Fixed,
        Some(_) => {
            return Err(issue(
                format!("{at}/strategy"),
                "expected 'fixed' or 'exponential'",
            ));
        }
    };
    let jitter = match spec.get("jitter") {
        None => defaults.jitter(),
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(issue(format!("{at}/jitter"), "expected a boolean")),
    };
    RetryPolicy::new(max_attempts, initial, max, strategy, jitter).map_err(IrError::Invalid)
}

fn kind_of(node: &Map<String, Value>, kind: &str, at: &str) -> Result<NodeKind, IrError> {
    Ok(match kind {
        "activity" => NodeKind::Activity {
            activity: ActivityName::parse(text(node, "activity", at)?).map_err(IrError::Invalid)?,
            retry: retry(node, at)?,
            compensate_with: optional_text(node, "compensateWith", at)?
                .map(ActivityName::parse)
                .transpose()
                .map_err(IrError::Invalid)?,
            reconcile_with: optional_text(node, "reconcileWith", at)?
                .map(ActivityName::parse)
                .transpose()
                .map_err(IrError::Invalid)?,
            idempotency_template: optional_text(node, "idempotencyTemplate", at)?
                .map(|t| expression(t, &format!("{at}/idempotencyTemplate")))
                .transpose()?,
        },
        "timer" => NodeKind::Timer {
            delay: millis(node, "durationMs", at)?,
        },
        "waitEvent" => NodeKind::WaitEvent {
            event_type: EventType::parse(text(node, "eventType", at)?).map_err(IrError::Invalid)?,
            correlation: expression(
                text(node, "correlationExpr", at)?,
                &format!("{at}/correlationExpr"),
            )?,
            timeout: millis(node, "timeoutMs", at)?,
            on_timeout: reference(node, "onTimeout", at)?,
        },
        "condition" => NodeKind::Condition {
            expression: expression(text(node, "expression", at)?, &format!("{at}/expression"))?,
            if_true: node_id(text(node, "ifTrue", at)?, &format!("{at}/ifTrue"))?,
            if_false: node_id(text(node, "ifFalse", at)?, &format!("{at}/ifFalse"))?,
        },
        "parallel" => {
            let branches = node
                .get("branches")
                .and_then(Value::as_array)
                .ok_or_else(|| issue(format!("{at}/branches"), "expected an array"))?
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let path = format!("{at}/branches/{i}");
                    b.as_str()
                        .ok_or_else(|| issue(path.clone(), "expected a string"))
                        .and_then(|b| node_id(b, &path))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let join = match text(node, "join", at)? {
                "all" => JoinPolicy::All,
                "any" => JoinPolicy::Any,
                _ => return Err(issue(format!("{at}/join"), "expected 'all' or 'any'")),
            };
            NodeKind::Parallel { branches, join }
        }
        "child" => NodeKind::Child {
            workflow: Slug::parse(text(node, "workflowId", at)?)
                .map_err(|e| issue(format!("{at}/workflowId"), e.to_string()))?,
            version: match node.get("version") {
                None => None,
                Some(v) => Some(
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .filter(|n| *n >= 1)
                        .ok_or_else(|| {
                            issue(format!("{at}/version"), "expected a positive integer")
                        })?,
                ),
            },
            input: optional_text(node, "input", at)?
                .map(|e| expression(e, &format!("{at}/input")))
                .transpose()?,
            cancel_policy: match optional_text(node, "cancelPolicy", at)? {
                None | Some("propagate") => CancelPolicy::Propagate,
                Some("request_cancel") => CancelPolicy::RequestCancel,
                Some("detach") => CancelPolicy::Detach,
                Some(_) => {
                    return Err(issue(
                        format!("{at}/cancelPolicy"),
                        "expected 'propagate', 'request_cancel' or 'detach'",
                    ));
                }
            },
        },
        "end" => NodeKind::End {
            output: optional_text(node, "outputExpr", at)?
                .map(|e| expression(e, &format!("{at}/outputExpr")))
                .transpose()?,
        },
        other => {
            return Err(issue(
                format!("{at}/type"),
                format!(
                    "unknown node type '{}'",
                    other.chars().take(32).collect::<String>()
                ),
            ));
        }
    })
}

pub(crate) fn definition(document: &Value) -> Result<WorkflowDefinition, IrError> {
    let name = document
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .ok_or_else(|| issue("/metadata/name".into(), "expected a string"))?;
    let name = Slug::parse(name).map_err(|e| issue("/metadata/name".into(), e.to_string()))?;
    let entrypoint = document
        .get("entrypoint")
        .and_then(Value::as_str)
        .ok_or_else(|| issue("/entrypoint".into(), "expected a string"))?;
    let entrypoint = node_id(entrypoint, "/entrypoint")?;
    let raw_nodes = document
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| issue("/nodes".into(), "expected an array"))?;

    let mut nodes = Vec::with_capacity(raw_nodes.len());
    for (index, raw) in raw_nodes.iter().enumerate() {
        let at = format!("/nodes/{index}");
        let node = raw
            .as_object()
            .ok_or_else(|| issue(at.clone(), "expected an object"))?;
        let id = text(node, "id", &at)?;
        let kind = text(node, "type", &at)?;
        if let Some(reason) = unsupported(kind) {
            return Err(IrError::Unsupported {
                node: id.chars().take(64).collect(),
                kind: kind.to_owned(),
                reason,
            });
        }
        let mut built = Node::new(node_id(id, &format!("{at}/id"))?, kind_of(node, kind, &at)?);
        if let Some(next) = reference(node, "next", &at)? {
            built = built.with_next(next);
        }
        if let Some(on_error) = reference(node, "onError", &at)? {
            built = built.with_on_error(on_error);
        }
        if node.contains_key("timeoutMs") {
            built = built.with_timeout(millis(node, "timeoutMs", &at)?);
        }
        nodes.push(built);
    }
    WorkflowDefinition::new(name, entrypoint, nodes).map_err(IrError::Invalid)
}
