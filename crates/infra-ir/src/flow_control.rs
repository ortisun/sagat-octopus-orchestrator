//! The flow-control declarations of a document (feature 022, release 0.3.0 slice): top-level `concurrency` and
//! `rateLimit` (run starts) and the same two fields on a root `activity` or `script` node (attempt dispatches).
//!
//! The codec decodes the fields on every path, `parse_stored` included; whether a deployment may publish them is the
//! definition service's decision (the `concurrencyKeys` and `rateLimits` capabilities and their fleet activation,
//! FR-022-031, FR-022-053), never this module's. What a key reads is checked here: a top-level key sees only `input`
//! (a run does not exist when it is evaluated), a node-level one the run data a step sees. The key's value is never
//! known to the codec: it is evaluated and digested at admission or readiness (D1168).

use std::time::Duration;

use sagat_application::classification::{CarrierPointer, ClassificationSegment};
use sagat_application::definitions::IrError;
use sagat_domain::classification::Level;
use sagat_domain::workflow::flow_control::{self as domain, DEFAULT_QUEUE_TIMEOUT};
use sagat_domain::workflow::{
    ConcurrencyPolicy, Expression, FlowControl, FlowControlRule, FlowGroup, GateKind, GroupName,
    NodeId, RateLimitPolicy, RatePeriod, WorkflowDefinition,
};
use sagat_infra_expr::{Compiled, Read, ReadStep};
use serde_json::{Map, Value};

use crate::classified_locations::Schemas;

/// What a top-level key may read: the run's input, nothing that exists only once the run does.
const RUN_KEY_VARIABLES: [&str; 1] = ["input"];
/// What a node-level key may read: the run data a step sees when it would become ready.
const STEP_KEY_VARIABLES: [&str; 4] = ["input", "steps", "state", "run"];
/// The variable that holds the run's identity: its only field is the run's id (`run.id`), so a key that reads it makes a
/// key per run, which publication refuses (FR-022-056, decision D022-081). The business identifier and the idempotency key
/// are not visible to an expression.
const IDENTITY_VARIABLE: &str = "run";

/// The longest expression a key may be: the bounded evaluator's source limit.
const MAX_KEY_EXPRESSION_BYTES: usize = 1024;

/// Refuses, at publication only, a root node's key that reads the run's identity, alone or beside other data: the run's
/// id makes the key one of a kind per run, as many keys as runs, each of which a window row and its retention keep alive
/// (FR-022-056, decision D022-081). A stored version is restored as it was published (`parse_stored` does not call
/// this), so a version an earlier build published keeps running (review of 2026-11-25, correctness minor 1). A key that
/// does not compile is left to the decoding, which says why.
pub(crate) fn check_publication_keys(document: &Value) -> Result<(), IrError> {
    let nodes = document
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for node in nodes {
        let Some(id) = node
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| NodeId::parse(id).ok())
        else {
            continue;
        };
        for (field, kind) in [
            ("concurrency", GateKind::Concurrency),
            ("rateLimit", GateKind::RateLimit),
        ] {
            let Some(text) = node
                .get(field)
                .and_then(|policy| policy.get("key"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if Compiled::compile(text)
                .is_ok_and(|compiled| compiled.variables().contains(IDENTITY_VARIABLE))
            {
                return Err(refused(kind, Some(&id), FlowControlRule::UnboundedKey));
            }
        }
    }
    Ok(())
}

/// Gives every key of `definition` the level of what it reads, from the schemas of `document` (FR-022-057, decision
/// D022-085), for publication and for the restore alike: the level decides who sees the key's label, so a version
/// restored by a later build is read by the same rules. Derived, so it never reaches the canonical bytes. A definition
/// without a key is returned as it is, and its schemas are not read.
pub(crate) fn attach_key_levels(
    document: &Value,
    definition: WorkflowDefinition,
) -> WorkflowDefinition {
    let keyed = |gates: &FlowControl| {
        gates
            .concurrency()
            .and_then(ConcurrencyPolicy::key)
            .is_some()
            || gates.rate_limit().and_then(RateLimitPolicy::key).is_some()
    };
    if !keyed(definition.run_flow_control())
        && !definition
            .gated_nodes()
            .any(|node| keyed(node.flow_control()))
    {
        return definition;
    }
    let schemas = Schemas::new(document);
    definition.with_flow_control_key_levels(|_, _, key| key_level(&schemas, key.as_str()))
}

/// Refuses, at publication only, a key that reads a location classified `secret` (`FLOW_CONTROL_KEY_SECRET`,
/// FR-022-057, decision D022-085). A stored version is restored as it was published: its key keeps the level `secret`,
/// whose label nobody may see.
pub(crate) fn check_secret_keys(definition: &WorkflowDefinition) -> Result<(), IrError> {
    if let Some(kind) = definition.run_flow_control().keys_at(Level::Secret).next() {
        return Err(refused(kind, None, FlowControlRule::SecretKey));
    }
    for node in definition.gated_nodes() {
        if let Some(kind) = node.flow_control().keys_at(Level::Secret).next() {
            return Err(refused(kind, Some(node.id()), FlowControlRule::SecretKey));
        }
    }
    Ok(())
}

/// The strictest level of every place `text` reads; `public` for a key that reads nothing, and `pii` for one this build
/// cannot compile (a stored text an earlier grammar accepted), as for anything it cannot follow.
fn key_level(schemas: &Schemas<'_>, text: &str) -> Level {
    let Ok(compiled) = Compiled::compile(text) else {
        return Level::UNMARKED;
    };
    compiled
        .reads()
        .iter()
        .map(|read| read_level(schemas, read))
        .fold(Level::Public, Level::stricter)
}

/// The level of one place a key reads, everything under it included. `input` is the input carrier; `steps.<id>.output`
/// (and `steps.<id>`, whose only field it is) the output carrier of a root node; every node's output for `steps` as a
/// whole. `state` has no carrier (016 refuses an annotation there), so it is `pii`, the level of a location no schema
/// classifies, as is anything else this reading cannot follow.
fn read_level(schemas: &Schemas<'_>, read: &Read) -> Level {
    let (carrier, path) = match (read.variable.as_str(), read.path.as_slice()) {
        ("input", path) => (CarrierPointer::Input, path),
        ("steps", [ReadStep::Field(node), rest @ ..]) => {
            let Ok(node) = NodeId::parse(node) else {
                return Level::UNMARKED;
            };
            match rest {
                [] => (CarrierPointer::NodeOutput(node), rest),
                [ReadStep::Field(output), rest @ ..] if output == "output" => {
                    (CarrierPointer::NodeOutput(node), rest)
                }
                _ => return Level::UNMARKED,
            }
        }
        ("steps", []) => {
            return schemas
                .index
                .locations()
                .iter()
                .filter(|location| matches!(location.carrier, CarrierPointer::NodeOutput(_)))
                .map(|location| location.level)
                .fold(Level::UNMARKED, Level::stricter);
        }
        _ => return Level::UNMARKED,
    };
    let Some(location) = path
        .iter()
        .map(|step| match step {
            ReadStep::Field(name) => Some(ClassificationSegment::Property(name.clone())),
            ReadStep::Position(at) => usize::try_from(*at).ok().map(ClassificationSegment::Index),
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Level::UNMARKED;
    };
    schemas
        .level_at(&carrier, &location)
        .unwrap_or(Level::UNMARKED)
}

/// Refuses, before the schema is applied, a gate declared on a node of a body (`forEach`, `while`, `repeatUntil`): body
/// instances are scheduled by 014's exact-path transactions, which need their own evidence (G1160, G1175), so the
/// refusal names what is missing (`UNSUPPORTED_NODE`, dependency `SCOPED_CONCURRENCY` or `SCOPED_RATE_LIMIT`) instead
/// of the schema's "additional property". Only a node whose id is a node id is answered here; any other shape is left
/// to the schema, so the error never repeats other text of the document.
pub(crate) fn check_surfaces(document: &Value) -> Result<(), IrError> {
    let bodies = document
        .get("bodies")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|bodies| bodies.values())
        .filter_map(|body| body.get("nodes").and_then(Value::as_array));
    for node in bodies.flatten() {
        let Some(id) = node
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| NodeId::parse(id).is_ok())
        else {
            continue;
        };
        for (field, dependency) in [
            ("concurrency", "SCOPED_CONCURRENCY"),
            ("rateLimit", "SCOPED_RATE_LIMIT"),
        ] {
            if node.get(field).is_some() {
                return Err(IrError::Pending {
                    node: id.to_owned(),
                    kind: field.to_owned(),
                    dependency,
                    reason: "flow control on a node inside a collection or loop body is not supported yet; declare it on a body `child` node's workflow",
                });
            }
        }
    }
    Ok(())
}

/// The gates of a run start, from the document's top level.
///
/// # Errors
/// [`IrError::Invalid`] with the rule a declaration breaks.
pub(crate) fn run(document: &Value) -> Result<FlowControl, IrError> {
    level(document.as_object(), None)
}

/// The gates of a root node's steps. A node of another kind that declares one is refused by the domain
/// (`Node::with_flow_control`), with the rule [`FlowControlRule::NodeKind`].
///
/// # Errors
/// [`IrError::Invalid`] with the rule a declaration breaks.
pub(crate) fn node(raw: &Map<String, Value>, id: &NodeId) -> Result<FlowControl, IrError> {
    level(Some(raw), Some(id))
}

fn level(raw: Option<&Map<String, Value>>, node: Option<&NodeId>) -> Result<FlowControl, IrError> {
    let Some(raw) = raw else {
        return Ok(FlowControl::NONE);
    };
    let concurrency = raw
        .get("concurrency")
        .map(|value| concurrency(value, node))
        .transpose()?;
    let rate_limit = raw
        .get("rateLimit")
        .map(|value| rate_limit(value, node))
        .transpose()?;
    Ok(FlowControl::new(concurrency, rate_limit))
}

fn refused(kind: GateKind, node: Option<&NodeId>, rule: FlowControlRule) -> IrError {
    IrError::Invalid(domain::invalid(kind, node, rule))
}

/// The fields of one declaration, closed: the schema says the same, and a stored document is read the same way.
fn object<'a>(
    value: &'a Value,
    kind: GateKind,
    node: Option<&NodeId>,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, IrError> {
    value
        .as_object()
        .filter(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
        .ok_or_else(|| shape_issue(kind, node))
}

fn shape_issue(kind: GateKind, node: Option<&NodeId>) -> IrError {
    let at = node.map_or_else(
        || format!("/{}", kind.field()),
        |node| format!("node '{node}'/{}", kind.field()),
    );
    IrError::Schema(vec![sagat_application::definitions::IrIssue {
        path: at,
        message: "a flow-control declaration is an object with only the fields of its schema"
            .to_owned(),
    }])
}

fn limit(
    object: &Map<String, Value>,
    kind: GateKind,
    node: Option<&NodeId>,
    max: u32,
) -> Result<u32, IrError> {
    object
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|limit| u32::try_from(limit).ok())
        .ok_or_else(|| refused(kind, node, FlowControlRule::Limit { max }))
}

fn group(
    object: &Map<String, Value>,
    kind: GateKind,
    node: Option<&NodeId>,
) -> Result<FlowGroup, IrError> {
    match object.get("group") {
        None => Ok(FlowGroup::Derived),
        Some(Value::String(name)) => GroupName::parse(name)
            .map(FlowGroup::Explicit)
            .map_err(|rule| refused(kind, node, rule)),
        Some(_) => Err(refused(kind, node, FlowControlRule::Group)),
    }
}

fn queue_timeout(
    object: &Map<String, Value>,
    kind: GateKind,
    node: Option<&NodeId>,
) -> Result<Duration, IrError> {
    match object.get("queueTimeoutMs") {
        None => Ok(DEFAULT_QUEUE_TIMEOUT),
        Some(value) => value
            .as_u64()
            .filter(|millis| *millis >= 1)
            .map(Duration::from_millis)
            .ok_or_else(|| refused(kind, node, FlowControlRule::QueueTimeout)),
    }
}

/// The key expression: compiled now, so a run never meets a syntax error, and reading only what its level may.
fn key(
    object: &Map<String, Value>,
    kind: GateKind,
    node: Option<&NodeId>,
) -> Result<Option<Expression>, IrError> {
    let Some(raw) = object.get("key") else {
        return Ok(None);
    };
    let place = node.map_or("(top level)", NodeId::as_str);
    let field = match kind {
        GateKind::Concurrency => "concurrency.key",
        GateKind::RateLimit => "rateLimit.key",
    };
    let text = raw
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= MAX_KEY_EXPRESSION_BYTES)
        .ok_or_else(|| IrError::Expression {
            node: place.to_owned(),
            field,
            message: format!("expected an expression of 1 to {MAX_KEY_EXPRESSION_BYTES} bytes"),
        })?;
    let compiled = Compiled::compile(text).map_err(|error| IrError::Expression {
        node: place.to_owned(),
        field,
        message: error.to_string(),
    })?;
    let allowed: &[&str] = if node.is_none() {
        &RUN_KEY_VARIABLES
    } else {
        &STEP_KEY_VARIABLES
    };
    let variables = compiled.variables();
    if variables
        .iter()
        .any(|variable| !allowed.contains(&variable.as_str()))
    {
        return Err(if node.is_none() {
            refused(kind, node, FlowControlRule::TopLevelKey)
        } else {
            IrError::Expression {
                node: place.to_owned(),
                field,
                message: "a step's key reads `input`, `steps` or `state`".to_owned(),
            }
        });
    }
    Expression::parse(text)
        .map(Some)
        .map_err(|error| IrError::Expression {
            node: place.to_owned(),
            field,
            message: error.to_string(),
        })
}

fn concurrency(value: &Value, node: Option<&NodeId>) -> Result<ConcurrencyPolicy, IrError> {
    let kind = GateKind::Concurrency;
    let object = object(
        value,
        kind,
        node,
        &["limit", "key", "group", "queueTimeoutMs"],
    )?;
    ConcurrencyPolicy::new(
        group(object, kind, node)?,
        key(object, kind, node)?,
        limit(object, kind, node, domain::MAX_CONCURRENCY_LIMIT)?,
        queue_timeout(object, kind, node)?,
    )
    .map_err(|rule| refused(kind, node, rule))
}

fn rate_limit(value: &Value, node: Option<&NodeId>) -> Result<RateLimitPolicy, IrError> {
    let kind = GateKind::RateLimit;
    let object = object(
        value,
        kind,
        node,
        &["limit", "per", "key", "group", "queueTimeoutMs"],
    )?;
    let per = object
        .get("per")
        .and_then(Value::as_str)
        .and_then(RatePeriod::parse)
        .ok_or_else(|| refused(kind, node, FlowControlRule::Period))?;
    RateLimitPolicy::new(
        group(object, kind, node)?,
        key(object, kind, node)?,
        limit(object, kind, node, per.max_limit())?,
        per,
        queue_timeout(object, kind, node)?,
    )
    .map_err(|rule| refused(kind, node, rule))
}
