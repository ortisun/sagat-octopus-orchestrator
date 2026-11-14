//! The `operators` of a `forEach` node (design 042, slice S0): decoding, the expression and variable rules, and the
//! chain's canonical encoding and digest.
//!
//! The codec always decodes the field, on every path, `parse_stored` included; whether a deployment may publish it is the
//! definition service's decision (`collection-operators`, FR-042-003), never this module's.

use sagat_application::definitions::{IrError, IrIssue, OperatorRefusal};
use sagat_domain::operators::{Operator, OperatorChain, OperatorError, OperatorName, Surface};
use sagat_domain::workflow::{Digest, Expression, NodeId};
use sagat_infra_expr::Compiled;
use serde_json::{Map, Value, json};

/// The variables a stage expression may read (FR-042-005): the element and its position, and what `items` sees.
const STAGE_VARIABLES: [&str; 6] = ["element", "position", "input", "steps", "run", "state"];

/// The names a stage may never read, kept free for the body and for an enclosing iteration (design 039).
const RESERVED_VARIABLES: [&str; 5] = ["item", "index", "iteration", "carry", "bodyResult"];

/// The structured expression ceiling, in UTF-8 bytes (the schema's `x-maxUtf8Bytes`).
const MAX_EXPRESSION_BYTES: usize = 1024;

/// Longest operator name an error repeats from the document.
const MAX_NAME_ECHO: usize = 64;

fn clipped(text: &str) -> String {
    text.chars().take(MAX_NAME_ECHO).collect()
}

fn refused(
    node: &str,
    stage: usize,
    operator: &str,
    refusal: OperatorRefusal,
    message: impl Into<String>,
) -> IrError {
    IrError::Operator {
        node: clipped(node),
        stage,
        operator: clipped(operator),
        refusal,
        message: message.into(),
    }
}

/// Refuses, before the schema is applied, a `forEach` stage that names an operator a collection does not have: one only
/// a trigger has, or a name that is no operator. The closed schema would refuse both, but without saying which operator
/// of which stage; this answer is typed (FR-042-001). Any other shape is left to the schema.
pub(crate) fn check_surfaces(document: &Value) -> Result<(), IrError> {
    let roots = document.get("nodes").and_then(Value::as_array);
    let bodies = document
        .get("bodies")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|bodies| bodies.values())
        .filter_map(|body| body.get("nodes").and_then(Value::as_array));
    for node in roots.into_iter().chain(bodies).flatten() {
        if node.get("type").and_then(Value::as_str) != Some("forEach") {
            continue;
        }
        let Some(stages) = node.get("operators").and_then(Value::as_array) else {
            continue;
        };
        let id = node.get("id").and_then(Value::as_str).unwrap_or_default();
        for (index, stage) in stages.iter().enumerate() {
            let Some((name, _)) = single_property(stage) else {
                continue;
            };
            match OperatorName::parse(name) {
                None => {
                    return Err(refused(
                        id,
                        index,
                        name,
                        OperatorRefusal::Unknown,
                        "not an operator",
                    ));
                }
                Some(operator) => {
                    operator
                        .require_on(Surface::Collection, index)
                        .map_err(|error| chain_error(id, "", &error))?;
                }
            }
        }
    }
    Ok(())
}

/// The answer to a domain refusal of a chain. `operators` is the chain's JSON pointer, where a length refusal points.
fn chain_error(node: &str, operators: &str, error: &OperatorError) -> IrError {
    match error {
        OperatorError::Surface {
            stage, operator, ..
        } => refused(
            node,
            *stage,
            operator.as_str(),
            OperatorRefusal::Surface,
            "only a trigger has this operator; a collection has filter, map, flatMap, skip, take, distinct and window",
        ),
        OperatorError::Count {
            stage,
            operator,
            min,
            max,
            ..
        } => refused(
            node,
            *stage,
            operator.as_str(),
            OperatorRefusal::Range,
            format!("count must be between {min} and {max}"),
        ),
        OperatorError::Length(_) => IrError::Schema(vec![IrIssue {
            path: operators.to_owned(),
            message: error.to_string(),
        }]),
    }
}

fn single_property(stage: &Value) -> Option<(&str, &Value)> {
    let object = stage.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.iter().next().map(|(k, v)| (k.as_str(), v))
}

/// The chain of a `forEach`, when it declares one. `at` is the node's JSON pointer.
pub(crate) fn decode(
    node: &Map<String, Value>,
    id: &NodeId,
    at: &str,
) -> Result<Option<OperatorChain>, IrError> {
    let Some(raw) = node.get("operators") else {
        return Ok(None);
    };
    let path = format!("{at}/operators");
    let issue = |path: String, message: &str| {
        IrError::Schema(vec![IrIssue {
            path,
            message: message.to_owned(),
        }])
    };
    let raw = raw
        .as_array()
        .ok_or_else(|| issue(path.clone(), "expected an array"))?;
    let node_name = id.as_str();
    let mut stages = Vec::with_capacity(raw.len());
    for (index, stage) in raw.iter().enumerate() {
        let at = format!("{path}/{index}");
        let (name, params) = single_property(stage)
            .ok_or_else(|| issue(at.clone(), "a stage has exactly one property"))?;
        let operator = OperatorName::parse(name).ok_or_else(|| {
            refused(
                node_name,
                index,
                name,
                OperatorRefusal::Unknown,
                "not an operator",
            )
        })?;
        operator
            .require_on(Surface::Collection, index)
            .map_err(|error| chain_error(node_name, &path, &error))?;
        let params = params
            .as_object()
            .ok_or_else(|| issue(format!("{at}/{name}"), "expected an object"))?;
        let expression = |field: &str| -> Result<Option<Expression>, IrError> {
            let Some(value) = params.get(field) else {
                return Ok(None);
            };
            let text = value
                .as_str()
                .ok_or_else(|| issue(format!("{at}/{name}/{field}"), "expected a string"))?;
            if text.len() > MAX_EXPRESSION_BYTES {
                return Err(refused(
                    node_name,
                    index,
                    name,
                    OperatorRefusal::Expression,
                    format!("`{field}` is longer than {MAX_EXPRESSION_BYTES} bytes"),
                ));
            }
            Expression::parse(text).map(Some).map_err(|error| {
                refused(
                    node_name,
                    index,
                    name,
                    OperatorRefusal::Expression,
                    format!("`{field}`: {error}"),
                )
            })
        };
        let required = |field: &str| {
            expression(field)?.ok_or_else(|| issue(format!("{at}/{name}/{field}"), "required"))
        };
        let count = || {
            params
                .get("count")
                .and_then(Value::as_u64)
                .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
                .ok_or_else(|| {
                    issue(
                        format!("{at}/{name}/count"),
                        "expected a non-negative integer",
                    )
                })
        };
        stages.push(match operator {
            OperatorName::Filter => Operator::Filter {
                predicate: required("where")?,
            },
            OperatorName::Map => Operator::Map {
                select: required("select")?,
            },
            OperatorName::FlatMap => Operator::FlatMap {
                select: required("select")?,
            },
            OperatorName::Skip => Operator::Skip { count: count()? },
            OperatorName::Take => Operator::Take { count: count()? },
            OperatorName::Distinct => Operator::Distinct {
                by: expression("by")?,
            },
            OperatorName::Window => Operator::Window { count: count()? },
            // `require_on` above refused them.
            OperatorName::Debounce | OperatorName::IgnoreElements => {
                return Err(chain_error(
                    node_name,
                    &path,
                    &OperatorError::Surface {
                        stage: index,
                        operator,
                        surface: Surface::Collection,
                    },
                ));
            }
        });
    }
    OperatorChain::collection(stages)
        .map(Some)
        .map_err(|error| chain_error(node_name, &path, &error))
}

/// Every stage expression compiles in the bounded language and reads only what a stage sees (FR-042-002, FR-042-005).
pub(crate) fn check_expressions(node: &NodeId, chain: &OperatorChain) -> Result<(), IrError> {
    for (stage, field, expression) in chain.expressions() {
        let operator = chain.stages()[stage].name().as_str();
        let compiled = Compiled::compile(expression.as_str()).map_err(|error| {
            refused(
                node.as_str(),
                stage,
                operator,
                OperatorRefusal::Expression,
                format!("`{field}`: {error}"),
            )
        })?;
        for variable in compiled.variables() {
            if STAGE_VARIABLES.contains(&variable.as_str()) {
                continue;
            }
            let message = if RESERVED_VARIABLES.contains(&variable.as_str()) {
                format!(
                    "`{field}` reads `{variable}`, which a stage never sees; a stage reads `element` and `position`"
                )
            } else {
                format!(
                    "`{field}` reads `{}`; a stage reads `element`, `position`, `input`, `steps`, `run` and `state`",
                    clipped(&variable)
                )
            };
            return Err(refused(
                node.as_str(),
                stage,
                operator,
                OperatorRefusal::Variable,
                message,
            ));
        }
    }
    Ok(())
}

/// The declaration of one stage, as the document writes it.
fn stage_value(operator: &Operator) -> Value {
    let params = match operator {
        Operator::Filter { predicate } => json!({ "where": predicate.as_str() }),
        Operator::Map { select } | Operator::FlatMap { select } => {
            json!({ "select": select.as_str() })
        }
        Operator::Skip { count } | Operator::Take { count } | Operator::Window { count } => {
            json!({ "count": count })
        }
        Operator::Distinct { by: Some(by) } => json!({ "by": by.as_str() }),
        Operator::Distinct { by: None } => json!({}),
    };
    let mut stage = Map::new();
    stage.insert(operator.name().as_str().to_owned(), params);
    Value::Object(stage)
}

/// The canonical encoding of a chain: the canonical JSON of its `operators` array, so the bytes are those of the
/// declaration whatever its whitespace or key order (D042-100).
#[must_use]
pub fn canonical_chain(chain: &OperatorChain) -> Vec<u8> {
    let stages = chain.stages().iter().map(stage_value).collect();
    crate::canonical::canonical_json(&Value::Array(stages))
}

/// SHA-256 of [`canonical_chain`]: what a chained blob manifest records as `chainSha256` (FR-042-010).
#[must_use]
pub fn chain_digest(chain: &OperatorChain) -> Digest {
    crate::canonical::digest_of(&canonical_chain(chain))
}
