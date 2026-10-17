//! Turns a schema-valid document into the domain model.
//!
//! The schema already guarantees the shape of every known node kind, but it allows extra fields on
//! nodes and does not know which kinds this engine can run, so conversion still reads defensively
//! and reports anything unexpected as an issue instead of panicking.

use std::time::Duration;

use sagat_application::definitions::{IrError, IrIssue};
use sagat_domain::execution::ApprovalPolicy;
use sagat_domain::identity::Slug;
use sagat_domain::identity::{Role, Subject};
use sagat_domain::saga::{EffectPolicy, OnCompensationFailure, SagaCancelPolicy};
use sagat_domain::scripts::{Digest, Language, activity_of};
use sagat_domain::secrets::{MAX_SECRETS_PER_ACTIVITY, SecretName};
use sagat_domain::triggers::{CronSpec, Trigger, TriggerKind};
use sagat_domain::workflow::{
    ActivityName, BackoffStrategy, BodyDefinition, CancelPolicy, ContinuationPolicy, EventType,
    Expression, JoinPolicy, Node, NodeId, NodeKind, RetryPolicy, SwitchCase, WorkflowDefinition,
    WorkflowError, WorkflowLimits,
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

/// Historical placeholder shapes have no executable definition.
fn unsupported(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "foreach" => "legacy collection placeholder has no executable definition",
        "humanApproval" => "legacy approval placeholder has no executable definition",
        "forEach" | "while" | "repeatUntil" | "return" | "continueAsNew" => {
            "this workflow capability is not enabled by the runtime"
        }
        _ => return None,
    })
}

fn unsupported_structured(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "foreach" => "legacy collection placeholder has no executable definition",
        "humanApproval" => "legacy approval placeholder has no executable definition",
        _ => return None,
    })
}

fn integer_u32(node: &Map<String, Value>, field: &str, at: &str) -> Result<u32, IrError> {
    node.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| issue(format!("{at}/{field}"), "expected a positive integer"))
}

fn integer_u16(node: &Map<String, Value>, field: &str, at: &str) -> Result<u16, IrError> {
    node.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| issue(format!("{at}/{field}"), "expected a positive integer"))
}

fn retry(node: &Map<String, Value>, at: &str) -> Result<RetryPolicy, IrError> {
    retry_at(node, "retry", RetryPolicy::DEFAULT, at)
}

/// The retry policy under `key`, with `defaults` for whatever it leaves out.
fn retry_at(
    node: &Map<String, Value>,
    key: &str,
    defaults: RetryPolicy,
    at: &str,
) -> Result<RetryPolicy, IrError> {
    let Some(Value::Object(spec)) = node.get(key) else {
        return Ok(defaults);
    };
    let at = format!("{at}/{key}");
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

/// What the author says about the effect of an activity: how its compensation retries, whether it
/// can be undone, whether an unknown outcome may ever be retried blindly.
fn effect(node: &Map<String, Value>, at: &str) -> Result<EffectPolicy, IrError> {
    let flag = |field: &str| match node.get(field) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(issue(format!("{at}/{field}"), "expected a boolean")),
    };
    Ok(EffectPolicy {
        compensation_retry: if node.contains_key("compensationRetry") {
            Some(retry_at(
                node,
                "compensationRetry",
                RetryPolicy::COMPENSATION,
                at,
            )?)
        } else {
            None
        },
        compensation_timeout: if node.contains_key("compensationTimeoutMs") {
            Some(millis(node, "compensationTimeoutMs", at)?)
        } else {
            None
        },
        irreversible: flag("irreversible")?,
        high_risk: flag("highRisk")?,
    })
}

/// The secret names an activity declares: strings of the secret-name grammar, at most
/// [`MAX_SECRETS_PER_ACTIVITY`], each once. A value can never be written here, only a name.
fn secrets(node: &Map<String, Value>, at: &str) -> Result<Vec<SecretName>, IrError> {
    let Some(value) = node.get("secrets") else {
        return Ok(Vec::new());
    };
    let path = format!("{at}/secrets");
    let items = value
        .as_array()
        .ok_or_else(|| issue(path.clone(), "expected an array of secret names"))?;
    if items.len() > MAX_SECRETS_PER_ACTIVITY {
        return Err(issue(
            path,
            format!("an activity declares at most {MAX_SECRETS_PER_ACTIVITY} secrets"),
        ));
    }
    let mut names = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let path = format!("{at}/secrets/{i}");
        let name = item
            .as_str()
            .and_then(|text| SecretName::parse(text).ok())
            .ok_or_else(|| {
                issue(
                    path.clone(),
                    "expected a secret name: lower-case letters, digits, '.', '_' or '-', starting with a letter",
                )
            })?;
        if names.contains(&name) {
            return Err(issue(path, "a secret is declared twice"));
        }
        names.push(name);
    }
    Ok(names)
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
            secrets: secrets(node, at)?,
        },
        "script" => {
            // A script runs as an activity named after the digest of its artifact: the step names
            // the exact code it runs, and a worker bound to `script.*` is the sandbox's runner.
            let digest = Digest::parse(text(node, "artifactDigest", at)?)
                .map_err(|e| issue(format!("{at}/artifactDigest"), e.to_string()))?;
            if let Some(language) = optional_text(node, "language", at)? {
                language
                    .parse::<Language>()
                    .map_err(|e| issue(format!("{at}/language"), e.to_string()))?;
            }
            NodeKind::Activity {
                activity: ActivityName::parse(&activity_of(&digest)).map_err(IrError::Invalid)?,
                retry: retry(node, at)?,
                compensate_with: optional_text(node, "compensateWith", at)?
                    .map(ActivityName::parse)
                    .transpose()
                    .map_err(IrError::Invalid)?,
                reconcile_with: None,
                idempotency_template: None,
                secrets: secrets(node, at)?,
            }
        }
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
            lookback: match node.get("lookbackMs") {
                None => Duration::ZERO,
                Some(value) => value.as_u64().map(Duration::from_millis).ok_or_else(|| {
                    issue(
                        format!("{at}/lookbackMs"),
                        "expected a non-negative integer number of milliseconds",
                    )
                })?,
            },
        },
        "condition" => NodeKind::Condition {
            expression: expression(text(node, "expression", at)?, &format!("{at}/expression"))?,
            if_true: node_id(text(node, "ifTrue", at)?, &format!("{at}/ifTrue"))?,
            if_false: node_id(text(node, "ifFalse", at)?, &format!("{at}/ifFalse"))?,
        },
        "switch" => {
            let cases = node
                .get("cases")
                .and_then(Value::as_array)
                .ok_or_else(|| issue(format!("{at}/cases"), "expected an array"))?
                .iter()
                .enumerate()
                .map(|(index, raw)| {
                    let path = format!("{at}/cases/{index}");
                    let case = raw
                        .as_object()
                        .ok_or_else(|| issue(path.clone(), "expected an object"))?;
                    Ok(SwitchCase::new(
                        node_id(text(case, "id", &path)?, &format!("{path}/id"))?,
                        expression(text(case, "when", &path)?, &format!("{path}/when"))?,
                        node_id(text(case, "target", &path)?, &format!("{path}/target"))?,
                    ))
                })
                .collect::<Result<Vec<_>, IrError>>()?;
            NodeKind::Switch {
                cases,
                default: node_id(text(node, "default", at)?, &format!("{at}/default"))?,
            }
        }
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
        "saga" => NodeKind::Saga {
            begin: node_id(text(node, "begin", at)?, &format!("{at}/begin"))?,
            on_compensation_failure: match optional_text(node, "onCompensationFailure", at)? {
                None => OnCompensationFailure::default(),
                Some(value) => value.parse().map_err(|()| {
                    issue(
                        format!("{at}/onCompensationFailure"),
                        "expected 'manual_review' or 'fail'",
                    )
                })?,
            },
            cancel_policy: match optional_text(node, "cancelPolicy", at)? {
                None => SagaCancelPolicy::default(),
                Some(value) => value.parse().map_err(|()| {
                    issue(
                        format!("{at}/cancelPolicy"),
                        "expected 'compensate_completed', 'abort_no_compensation' or 'wait_in_flight_then_compensate'",
                    )
                })?,
            },
        },
        "forEach" => NodeKind::ForEach {
            items: expression(text(node, "items", at)?, &format!("{at}/items"))?,
            body: node_id(text(node, "body", at)?, &format!("{at}/body"))?,
            result: expression(text(node, "result", at)?, &format!("{at}/result"))?,
            max_items: integer_u32(node, "maxItems", at)?,
            max_concurrency: integer_u16(node, "maxConcurrency", at)?,
        },
        "while" => NodeKind::While {
            condition: expression(text(node, "condition", at)?, &format!("{at}/condition"))?,
            body: node_id(text(node, "body", at)?, &format!("{at}/body"))?,
            initial: expression(text(node, "initial", at)?, &format!("{at}/initial"))?,
            carry: expression(text(node, "carry", at)?, &format!("{at}/carry"))?,
            max_iterations: integer_u32(node, "maxIterations", at)?,
            delay: match node.get("delayMs") {
                None => None,
                Some(_) => Some(millis(node, "delayMs", at)?),
            },
        },
        "repeatUntil" => NodeKind::RepeatUntil {
            condition: expression(text(node, "condition", at)?, &format!("{at}/condition"))?,
            body: node_id(text(node, "body", at)?, &format!("{at}/body"))?,
            initial: expression(text(node, "initial", at)?, &format!("{at}/initial"))?,
            carry: expression(text(node, "carry", at)?, &format!("{at}/carry"))?,
            max_iterations: integer_u32(node, "maxIterations", at)?,
            delay: match node.get("delayMs") {
                None => None,
                Some(_) => Some(millis(node, "delayMs", at)?),
            },
        },
        "approval" => {
            let policy_path = format!("{at}/policy");
            let policy = node
                .get("policy")
                .and_then(Value::as_object)
                .ok_or_else(|| issue(policy_path.clone(), "expected an object"))?;
            let subjects = policy
                .get("subjects")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let path = format!("{policy_path}/subjects/{index}");
                    value
                        .as_str()
                        .ok_or_else(|| issue(path.clone(), "expected a string"))
                        .and_then(|value| Subject::parse(value).map_err(|error| issue(path, error.to_string())))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let roles = policy
                .get("roles")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let path = format!("{policy_path}/roles/{index}");
                    value
                        .as_str()
                        .ok_or_else(|| issue(path.clone(), "expected a string"))
                        .and_then(|value| value.parse::<Role>().map_err(|error| issue(path, error.to_string())))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let exclude_initiator = policy
                .get("excludeInitiator")
                .map(Value::as_bool)
                .unwrap_or(Some(true))
                .ok_or_else(|| issue(format!("{policy_path}/excludeInitiator"), "expected a boolean"))?;
            NodeKind::Approval {
                policy: ApprovalPolicy::with_initiator_exclusion(subjects, roles, exclude_initiator)
                    .map_err(|error| issue(policy_path, error.to_string()))?,
                on_approve: node_id(text(node, "onApprove", at)?, &format!("{at}/onApprove"))?,
                on_reject: node_id(text(node, "onReject", at)?, &format!("{at}/onReject"))?,
                on_expire: node_id(text(node, "onExpire", at)?, &format!("{at}/onExpire"))?,
            }
        }
        "return" => NodeKind::Return {
            result: expression(text(node, "result", at)?, &format!("{at}/result"))?,
        },
        "continueAsNew" => NodeKind::ContinueAsNew {
            input: expression(text(node, "input", at)?, &format!("{at}/input"))?,
            state: optional_text(node, "state", at)?
                .map(|value| expression(value, &format!("{at}/state")))
                .transpose()?,
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

fn nodes(raw_nodes: &[Value], at: &str, structured: bool) -> Result<Vec<Node>, IrError> {
    let mut nodes = Vec::with_capacity(raw_nodes.len());
    for (index, raw) in raw_nodes.iter().enumerate() {
        let at = format!("{at}/{index}");
        let node = raw
            .as_object()
            .ok_or_else(|| issue(at.clone(), "expected an object"))?;
        let id = text(node, "id", &at)?;
        let kind = text(node, "type", &at)?;
        let unsupported = if structured {
            unsupported_structured(kind)
        } else {
            unsupported(kind)
        };
        if let Some(reason) = unsupported {
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
        let effect = effect(node, &at)?;
        if effect != EffectPolicy::NONE {
            built = built.with_effect(effect);
        }
        nodes.push(built);
    }
    Ok(nodes)
}

fn identity(document: &Value) -> Result<(Slug, NodeId), IrError> {
    let name = document
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .ok_or_else(|| issue("/metadata/name".into(), "expected a string"))?;
    let name = Slug::parse(name).map_err(|e| issue("/metadata/name".into(), e.to_string()))?;
    let entrypoint = document
        .get("entrypoint")
        .and_then(Value::as_str)
        .ok_or_else(|| issue("/entrypoint".into(), "expected a string"))?;
    Ok((name, node_id(entrypoint, "/entrypoint")?))
}

fn root_nodes(document: &Value, structured: bool) -> Result<Vec<Node>, IrError> {
    let raw = document
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| issue("/nodes".into(), "expected an array"))?;
    nodes(raw, "/nodes", structured)
}

pub(crate) fn definition(document: &Value) -> Result<WorkflowDefinition, IrError> {
    for field in ["bodies", "state", "chainPolicy"] {
        if document.get(field).is_some() {
            return Err(IrError::UnsupportedDeclaration {
                field,
                reason: "this workflow capability is not enabled by the runtime",
            });
        }
    }
    let limits = document_limits(document)?;
    let (name, entrypoint) = identity(document)?;
    let nodes = root_nodes(document, false)?;
    let has_approval = nodes
        .iter()
        .any(|node| matches!(node.kind(), NodeKind::Approval { .. }));
    if has_approval
        && let Some(parallel) = nodes
            .iter()
            .find(|node| matches!(node.kind(), NodeKind::Parallel { .. }))
    {
        return Err(IrError::Unsupported {
            node: parallel.id().as_str().into(),
            kind: "parallel".into(),
            reason: "approval and parallel execution are not enabled together",
        });
    }
    let definition = if has_approval {
        WorkflowDefinition::new_structured(name, entrypoint, nodes, Vec::new(), limits)
    } else {
        WorkflowDefinition::new(name, entrypoint, nodes)
            .and_then(|definition| definition.with_limits(limits))
    };
    definition
        .and_then(|definition| definition.with_triggers(triggers(document)?))
        .map_err(IrError::Invalid)
}

pub(crate) fn published_declared_state_definition(
    document: &Value,
) -> Result<WorkflowDefinition, IrError> {
    for field in ["bodies", "chainPolicy"] {
        if document.get(field).is_some() {
            return Err(IrError::UnsupportedDeclaration {
                field,
                reason: "this workflow capability is not enabled by the runtime",
            });
        }
    }
    let limits = document_limits(document)?;
    if let Some(node) = document
        .get("nodes")
        .and_then(Value::as_array)
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node.get("type").and_then(Value::as_str) == Some("approval"))
        })
    {
        return Err(IrError::Unsupported {
            node: node
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("approval")
                .chars()
                .take(64)
                .collect(),
            kind: "approval".into(),
            reason: "declared state and approval are not enabled together",
        });
    }
    let (name, entrypoint) = identity(document)?;
    let nodes = root_nodes(document, false)?;
    WorkflowDefinition::new(name, entrypoint, nodes)
        .and_then(|definition| definition.with_limits(limits))
        .and_then(|definition| definition.with_triggers(triggers(document)?))
        .map_err(IrError::Invalid)
}

pub(crate) fn structured_definition(document: &Value) -> Result<WorkflowDefinition, IrError> {
    for field in ["state", "chainPolicy"] {
        if document.get(field).is_some() {
            return Err(IrError::UnsupportedDeclaration {
                field,
                reason: "this workflow capability has no validated domain mapping",
            });
        }
    }
    structured_definition_inner(document)
}

/// The longest chain of activities a published collection or loop body may have. A chain is what makes the
/// compensation order inside one iteration observable at runtime (the later activities are undone
/// first); the bound is the largest the runtime tests exercise, and can be raised with them.
const MAX_BODY_CHAIN: usize = 8;

/// Whether the body is one to `limit` activities in sequence and then its return: each activity
/// routes to the next, has no `onError` edge, and may declare `compensateWith`, `reconcileWith`,
/// `irreversible` and `highRisk` (their derived companions are not nodes of the chain). Nothing
/// else (branches, timers, children, sagas, nested scopes) is routed by the scoped planner.
fn is_activity_chain(body: &sagat_domain::workflow::BodyDefinition, limit: usize) -> bool {
    let declared = body.nodes().count();
    let mut current = body.entrypoint();
    let mut activities = 0_usize;
    loop {
        let Some(node) = body.node(current) else {
            return false;
        };
        match node.kind() {
            NodeKind::Activity { .. } if node.on_error().is_none() => {
                activities += 1;
                if activities > limit {
                    return false;
                }
                let Some(next) = node.next() else {
                    return false;
                };
                current = next;
            }
            NodeKind::Return { .. } => break,
            _ => return false,
        }
    }
    activities >= 1 && activities + 1 == declared
}

pub(crate) fn published_collection_definition(
    document: &Value,
) -> Result<WorkflowDefinition, IrError> {
    if document.get("chainPolicy").is_some() {
        return Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            reason: "collection continuation has not passed its transactional runtime gate",
        });
    }
    if document.get("triggers").is_some() {
        return Err(IrError::UnsupportedDeclaration {
            field: "triggers",
            reason: "triggered collections are not enabled by the runtime",
        });
    }
    let definition = structured_definition(document)?;
    if !definition
        .node(definition.entrypoint())
        .is_some_and(|node| matches!(node.kind(), NodeKind::ForEach { .. }))
    {
        return Err(IrError::UnsupportedDeclaration {
            field: "entrypoint",
            reason: "a published collection workflow starts at its collection owner",
        });
    }
    if let Some(node) = definition
        .nodes()
        .find(|node| !matches!(node.kind(), NodeKind::ForEach { .. } | NodeKind::End { .. }))
    {
        return Err(IrError::Unsupported {
            node: node.id().as_str().to_owned(),
            kind: node.kind().label().to_owned(),
            reason: "the public collection subset is a collection chain ending at end",
        });
    }
    let referenced_bodies = definition
        .nodes()
        .filter_map(|node| match node.kind() {
            NodeKind::ForEach { body, .. } => Some(body),
            NodeKind::While { .. }
            | NodeKind::RepeatUntil { .. }
            | NodeKind::Approval { .. }
            | NodeKind::ContinueAsNew { .. }
            | NodeKind::Return { .. } => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    if referenced_bodies.is_empty() {
        return Err(IrError::UnsupportedDeclaration {
            field: "bodies",
            reason: "published bodies require a collection owner",
        });
    }
    if let Some(node) = definition.nodes().find(|node| {
        matches!(
            node.kind(),
            NodeKind::While { .. }
                | NodeKind::RepeatUntil { .. }
                | NodeKind::Approval { .. }
                | NodeKind::ContinueAsNew { .. }
                | NodeKind::Return { .. }
        )
    }) {
        return Err(IrError::Unsupported {
            node: node.id().as_str().to_owned(),
            kind: node.kind().label().to_owned(),
            reason: "this combination with a collection is not enabled by the runtime",
        });
    }
    for body in definition.bodies() {
        if !referenced_bodies.contains(&body.name()) {
            return Err(IrError::Unsupported {
                node: body.name().as_str().to_owned(),
                kind: "body".to_owned(),
                reason: "published collection bodies must be referenced by a collection owner",
            });
        }
        if !is_activity_chain(body, MAX_BODY_CHAIN) {
            return Err(IrError::Unsupported {
                node: body.name().as_str().to_owned(),
                kind: "body".to_owned(),
                reason: "the runtime supports a collection body of one to eight activities in sequence followed by one return",
            });
        }
    }
    Ok(definition)
}

/// The root activities a loop's `onError` edge recovers with (G340): the target of the edge and the
/// activities it chains to, ending at `end`. Such an activity is an ordinary root activity that the
/// loop's failure finish makes ready, so it is kept as plain as the runtime proves: it has no
/// `onError` edge of its own and declares no compensation, reconciliation or effect policy. An
/// activity after a loop's success edge, or anywhere else in the root graph, stays unsupported.
fn loop_recovery_activities(
    definition: &WorkflowDefinition,
) -> Result<std::collections::BTreeSet<NodeId>, IrError> {
    let unsupported =
        |node: &sagat_domain::workflow::Node, reason: &'static str| IrError::Unsupported {
            node: node.id().as_str().to_owned(),
            kind: node.kind().label().to_owned(),
            reason,
        };
    let mut recovery = std::collections::BTreeSet::new();
    for owner in definition.nodes().filter(|node| {
        matches!(
            node.kind(),
            NodeKind::While { .. } | NodeKind::RepeatUntil { .. }
        )
    }) {
        if let Some(next) = owner
            .next()
            .and_then(|next| definition.node(next))
            .filter(|next| matches!(next.kind(), NodeKind::Activity { .. }))
        {
            return Err(unsupported(
                next,
                "an activity after a loop's success edge is not enabled by the runtime; only an onError recovery activity is",
            ));
        }
        let mut current = owner.on_error().and_then(|target| definition.node(target));
        while let Some(node) = current {
            let NodeKind::Activity {
                compensate_with,
                reconcile_with,
                ..
            } = node.kind()
            else {
                break;
            };
            if node.on_error().is_some()
                || compensate_with.is_some()
                || reconcile_with.is_some()
                || node.effect().irreversible
                || node.effect().high_risk
            {
                return Err(unsupported(
                    node,
                    "a loop recovery activity has no onError edge, compensation, reconciliation or effect policy",
                ));
            }
            if !recovery.insert(node.id().clone()) {
                break;
            }
            let next = node.next().and_then(|next| definition.node(next));
            if !next.is_some_and(|next| {
                matches!(
                    next.kind(),
                    NodeKind::Activity { .. } | NodeKind::End { .. }
                )
            }) {
                return Err(unsupported(
                    node,
                    "a loop recovery activity chain ends at end",
                ));
            }
            current = next;
        }
    }
    Ok(recovery)
}

pub(crate) fn published_loop_definition(document: &Value) -> Result<WorkflowDefinition, IrError> {
    if document.get("chainPolicy").is_some() {
        return Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            reason: "loop continuation has not passed its combined runtime gate",
        });
    }
    if document.get("triggers").is_some() {
        return Err(IrError::UnsupportedDeclaration {
            field: "triggers",
            reason: "triggered loops are not enabled by the runtime",
        });
    }
    let definition = structured_definition(document)?;
    if !definition
        .node(definition.entrypoint())
        .is_some_and(|node| {
            matches!(
                node.kind(),
                NodeKind::While { .. } | NodeKind::RepeatUntil { .. }
            )
        })
    {
        return Err(IrError::UnsupportedDeclaration {
            field: "entrypoint",
            reason: "a published loop workflow starts at its loop owner",
        });
    }
    let recovery = loop_recovery_activities(&definition)?;
    if let Some(node) = definition.nodes().find(|node| match node.kind() {
        NodeKind::While { .. } | NodeKind::RepeatUntil { .. } | NodeKind::End { .. } => false,
        NodeKind::Activity { .. } => !recovery.contains(node.id()),
        _ => true,
    }) {
        return Err(IrError::Unsupported {
            node: node.id().as_str().to_owned(),
            kind: node.kind().label().to_owned(),
            reason: "the public loop subset is a loop chain ending at end, plus the activities a loop's onError edge recovers with",
        });
    }
    let referenced_bodies = definition
        .nodes()
        .filter_map(|node| match node.kind() {
            NodeKind::While { body, .. } | NodeKind::RepeatUntil { body, .. } => Some(body),
            _ => None,
        })
        .collect::<Vec<_>>();
    if referenced_bodies.is_empty() {
        return Err(IrError::UnsupportedDeclaration {
            field: "bodies",
            reason: "published bodies require a loop owner",
        });
    }
    for body in definition.bodies() {
        if !referenced_bodies.contains(&body.name()) {
            return Err(IrError::Unsupported {
                node: body.name().as_str().to_owned(),
                kind: "body".to_owned(),
                reason: "published loop bodies must be referenced by a loop owner",
            });
        }
        if !is_activity_chain(body, MAX_BODY_CHAIN) {
            return Err(IrError::Unsupported {
                node: body.name().as_str().to_owned(),
                kind: "body".to_owned(),
                reason: "the runtime supports a loop body of one to eight activities in sequence followed by one return",
            });
        }
    }
    Ok(definition)
}

pub(crate) fn declared_state_definition(document: &Value) -> Result<WorkflowDefinition, IrError> {
    if document.get("chainPolicy").is_some() {
        return Err(IrError::UnsupportedDeclaration {
            field: "chainPolicy",
            reason: "this workflow capability has no validated domain mapping",
        });
    }
    structured_definition_inner(document)
}

fn structured_definition_inner(document: &Value) -> Result<WorkflowDefinition, IrError> {
    structured_definition_with_policy(document, None)
}

pub(crate) fn continuation_definition(
    document: &Value,
    policy: ContinuationPolicy,
) -> Result<WorkflowDefinition, IrError> {
    structured_definition_with_policy(document, Some(policy))
}

/// The author's `limits` block: each omitted field keeps its platform ceiling (100000 instances,
/// depth 8) and a value outside `1..=100000` / `1..=8` is refused, so a document can only lower
/// the ceilings.
fn document_limits(document: &Value) -> Result<WorkflowLimits, IrError> {
    let defaults = WorkflowLimits::default();
    match document.get("limits") {
        None => Ok(defaults),
        Some(Value::Object(raw)) => {
            let max_instances = match raw.get("maxInstances") {
                None => defaults.max_instances(),
                Some(value) => value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        issue("/limits/maxInstances".into(), "expected a positive integer")
                    })?,
            };
            let max_depth = match raw.get("maxDepth") {
                None => defaults.max_depth(),
                Some(value) => value
                    .as_u64()
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or_else(|| {
                        issue("/limits/maxDepth".into(), "expected a positive integer")
                    })?,
            };
            WorkflowLimits::new(max_instances, max_depth).map_err(IrError::Invalid)
        }
        Some(_) => Err(issue("/limits".into(), "expected an object")),
    }
}

fn structured_definition_with_policy(
    document: &Value,
    continuation_policy: Option<ContinuationPolicy>,
) -> Result<WorkflowDefinition, IrError> {
    let (name, entrypoint) = identity(document)?;
    let root = root_nodes(document, true)?;
    let mut bodies = Vec::new();
    if let Some(raw_bodies) = document.get("bodies").and_then(Value::as_object) {
        bodies.reserve(raw_bodies.len());
        for (body_name, raw_body) in raw_bodies {
            let at = format!("/bodies/{body_name}");
            let body = raw_body
                .as_object()
                .ok_or_else(|| issue(at.clone(), "expected an object"))?;
            let name = node_id(body_name, &at)?;
            let entrypoint = node_id(text(body, "entrypoint", &at)?, &format!("{at}/entrypoint"))?;
            let raw_nodes = body
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| issue(format!("{at}/nodes"), "expected an array"))?;
            let nodes = nodes(raw_nodes, &format!("{at}/nodes"), true)?;
            bodies.push(BodyDefinition::new(name, entrypoint, nodes).map_err(IrError::Invalid)?);
        }
    }
    let limits = document_limits(document)?;
    let definition = match continuation_policy {
        Some(policy) => WorkflowDefinition::new_structured_with_continuation(
            name, entrypoint, root, bodies, limits, policy,
        ),
        None => WorkflowDefinition::new_structured(name, entrypoint, root, bodies, limits),
    };
    definition
        .and_then(|definition| definition.with_triggers(triggers(document)?))
        .map_err(IrError::Invalid)
}

/// The `triggers` of a document. An absent list is no trigger: runs are started by the API.
fn triggers(document: &Value) -> Result<Vec<Trigger>, WorkflowError> {
    let Some(raw) = document.get("triggers") else {
        return Ok(Vec::new());
    };
    let items = raw
        .as_array()
        .ok_or(WorkflowError::InvalidTrigger("triggers is an array"))?;
    let slug = |item: &Map<String, Value>, field: &str| -> Result<Option<Slug>, WorkflowError> {
        item.get(field)
            .map(|v| {
                v.as_str().and_then(|text| Slug::parse(text).ok()).ok_or(
                    WorkflowError::InvalidTrigger(
                        "a source, endpoint or connector is a lower-case slug",
                    ),
                )
            })
            .transpose()
    };
    let event_type = |item: &Map<String, Value>| -> Result<Option<EventType>, WorkflowError> {
        item.get("eventType")
            .map(|v| {
                v.as_str()
                    .ok_or(WorkflowError::InvalidTrigger("eventType is a string"))
                    .and_then(EventType::parse)
            })
            .transpose()
    };
    items
        .iter()
        .map(|item| {
            let item = item
                .as_object()
                .ok_or(WorkflowError::InvalidTrigger("a trigger is an object"))?;
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .and_then(|k| k.parse::<TriggerKind>().ok())
                .ok_or(WorkflowError::InvalidTrigger("unknown trigger kind"))?;
            let trigger = match kind {
                TriggerKind::Manual => Trigger::manual(),
                TriggerKind::Event => Trigger::event(
                    event_type(item)?.ok_or(WorkflowError::InvalidTrigger(
                        "an event trigger needs an eventType",
                    ))?,
                    slug(item, "source")?,
                ),
                TriggerKind::Webhook => Trigger::webhook(
                    slug(item, "endpoint")?.ok_or(WorkflowError::InvalidTrigger(
                        "a webhook trigger needs the endpoint that receives it",
                    ))?,
                    event_type(item)?,
                ),
                TriggerKind::Message => Trigger::message(
                    slug(item, "connector")?.ok_or(WorkflowError::InvalidTrigger(
                        "a message trigger needs the connector that consumes it",
                    ))?,
                    event_type(item)?,
                ),
                TriggerKind::Cron => {
                    let expression = item.get("cron").and_then(Value::as_str).ok_or(
                        WorkflowError::InvalidTrigger("a cron trigger needs a cron expression"),
                    )?;
                    let timezone = item
                        .get("timezone")
                        .and_then(Value::as_str)
                        .unwrap_or("UTC");
                    Trigger::cron(CronSpec::parse(expression, timezone)?)
                }
            };
            match item.get("filter") {
                None => Ok(trigger),
                Some(filter) => filter
                    .as_str()
                    .ok_or(WorkflowError::InvalidTrigger(
                        "a filter is an expression string",
                    ))
                    .and_then(Expression::parse)
                    .map(|filter| trigger.with_filter(filter)),
            }
        })
        .collect()
}
