//! The workflow IR codec: JSON document in, validated domain model, canonical bytes and digest out.
//!
//! Pipeline (`docs/architecture/workflow-ir-and-sdk-modes.md`): parse, validate against the
//! published JSON Schema, convert to the domain (which proves the graph is sound), compile every
//! expression with the bounded evaluator, canonicalise and hash. Nothing here executes business
//! logic; compiling a document has no side effects.

mod payload_projection;
pub use payload_projection::JsonPayloadProjector;
mod canonical;
mod carrier_schema_check;
mod classification_check;
mod continuation;
mod convert;
pub mod cron;
mod database;
mod database_result;
mod declared_state;
pub mod schedule_calendar;
pub use database_result::JsonDatabaseResultValidator;
mod diff;
mod evaluator;
mod operation_evaluator;
mod schema;

pub use continuation::{ChainPolicyDefinition, ContinueAsNewDefinition, ParsedContinuation};
pub use declared_state::{
    DeclaredQueryDefinition, DeclaredSchema, DeclaredStateDefinition, DeclaredUpdateDefinition,
    MAX_DECLARED_SCHEMA_BYTES, MAX_DECLARED_SCHEMA_DEPTH, ParsedDeclaredState,
};
pub use evaluator::JsonEvaluator;
pub use operation_evaluator::{
    EvaluatedOperationValue, JsonDeclaredOperationEvaluator, QueryEvaluation, UpdateEvaluationPlan,
};

use sagat_application::definitions::{IrCodec, IrError, MAX_IR_BYTES, ParsedDefinition};
use sagat_application::operation_evaluation::OperationEvaluationError;
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use sagat_infra_expr::{Compiled, Template};
use serde_json::Value;

/// The production [`IrCodec`].
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonIrCodec;

impl JsonIrCodec {
    /// Validates the additive structured subset without making it publishable or executable.
    ///
    /// This entry point currently maps named bodies, collections, sequential loops, approval, `return` and
    /// workflow limits. The production [`IrCodec::parse`] path remains the publication capability
    /// gate.
    ///
    /// # Errors
    /// Returns the same bounded syntax, schema, graph and expression errors as publication parsing,
    /// plus an unsupported-capability error for structured kinds without a domain mapping.
    pub fn parse_structured(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        parse_document(document, convert::structured_definition)
    }

    /// Validates and preserves an additive-v1 state declaration for internal operation evaluation.
    ///
    /// The returned artifact retains the canonical workflow bytes and typed state/operation
    /// metadata. It accepts the full typed subset even when publication keeps another primitive
    /// closed.
    ///
    /// # Errors
    /// Returns bounded syntax, schema, graph or expression errors. A chain policy is accepted only
    /// for the published direct continuation shape that transfers the exact declared state.
    pub fn parse_declared_state(&self, document: &[u8]) -> Result<ParsedDeclaredState, IrError> {
        declared_state_definition(validated_document(document)?)
    }

    /// Restores declared operation metadata from a trusted immutable version, without applying
    /// later classification authoring rules to its already-published business operations.
    ///
    /// # Errors
    /// Syntax, graph, expression, capability and declared-state validation remain enforced.
    pub fn parse_stored_declared_state(
        &self,
        document: &[u8],
    ) -> Result<ParsedDeclaredState, IrError> {
        declared_state_definition(validated_stored_document(document)?)
    }

    /// Validates and preserves additive-v1 continuation declarations.
    ///
    /// # Errors
    /// Returns bounded syntax, schema, graph or expression errors. Publication accepts only the
    /// narrower runtime shape enforced by [`IrCodec::parse`].
    pub fn parse_continuation(&self, document: &[u8]) -> Result<ParsedContinuation, IrError> {
        let value = validated_document(document)?;
        continuation::parse(&value)
    }
}

impl IrCodec for JsonIrCodec {
    fn classification_analysis(
        &self,
        document: &[u8],
    ) -> Option<sagat_application::classification::ClassificationAnalysis> {
        if document.len() > MAX_IR_BYTES {
            return None;
        }
        let value: Value = serde_json::from_slice(document).ok()?;
        value
            .is_object()
            .then(|| classification_check::check(&value))
    }

    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        if document.len() > MAX_IR_BYTES {
            return Err(IrError::TooLarge);
        }
        match serde_json::from_slice::<Value>(document) {
            Ok(Value::Object(_)) => Ok(()),
            _ => Err(IrError::Malformed),
        }
    }

    fn diff(
        &self,
        from: &[u8],
        to: &[u8],
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        diff::diff(from, to, sagat_application::diff::DiffRules::CURRENT)
    }

    fn diff_with_rules(
        &self,
        from: &[u8],
        to: &[u8],
        rules: sagat_application::diff::DiffRules,
    ) -> Result<sagat_application::diff::DocumentDiff, IrError> {
        diff::diff(from, to, rules)
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        published_definition(validated_document(document)?)
    }

    fn parse_stored(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        published_definition(validated_stored_document(document)?)
    }

    fn declared_state_publication(
        &self,
        document: &[u8],
    ) -> Result<Option<sagat_application::declared_operations::PublishedDeclaredState>, IrError>
    {
        let value = validated_document(document)?;
        value
            .get("state")
            .map(|_| declared_state::parse(&value)?.publication())
            .transpose()
    }

    fn initial_declared_state(
        &self,
        document: &[u8],
        input: &[u8],
    ) -> Result<
        Option<sagat_application::declared_operations::EvaluatedOperationWrite>,
        OperationEvaluationError,
    > {
        let value: Value = serde_json::from_slice(document)
            .map_err(|_| OperationEvaluationError::DeclarationMismatch)?;
        if value.get("state").is_none() {
            return Ok(None);
        }
        let state = declared_state::parse(&value)
            .map_err(|_| OperationEvaluationError::DeclarationMismatch)?;
        JsonDeclaredOperationEvaluator
            .initial_state(&state, input)
            .map(Some)
    }
}

// Both entry points use the same executable conversion and capability gates. Classification
// metadata is checked for new publications, never retroactively interpreted during restoration.
fn declared_state_definition(value: Value) -> Result<ParsedDeclaredState, IrError> {
    let state = declared_state::parse(&value)?;
    let parsed = if value.get("chainPolicy").is_some() {
        let continuation = continuation::parse_published(&value)?;
        ParsedDefinition {
            definition: continuation.definition().clone(),
            canonical: continuation.canonical().to_vec(),
            digest: continuation.digest(),
            warnings: continuation.warnings().to_vec(),
        }
    } else {
        converted_document(&value, convert::declared_state_definition)?
    };
    Ok(ParsedDeclaredState::new(parsed, state))
}

fn published_definition(value: Value) -> Result<ParsedDefinition, IrError> {
    if value.get("state").is_some() {
        declared_state::parse(&value)?;
        if value.get("chainPolicy").is_some() {
            let parsed = continuation::parse_published(&value)?;
            Ok(ParsedDefinition {
                definition: parsed.definition().clone(),
                canonical: parsed.canonical().to_vec(),
                digest: parsed.digest(),
                warnings: parsed.warnings().to_vec(),
            })
        } else {
            converted_document(&value, convert::published_declared_state_definition)
        }
    } else if value.get("chainPolicy").is_some()
        && value
            .get("nodes")
            .and_then(Value::as_array)
            .is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|node| node.get("type").and_then(Value::as_str) == Some("continueAsNew"))
            })
    {
        let parsed = continuation::parse_published(&value)?;
        Ok(ParsedDefinition {
            definition: parsed.definition().clone(),
            canonical: parsed.canonical().to_vec(),
            digest: parsed.digest(),
            warnings: parsed.warnings().to_vec(),
        })
    } else if value
        .get("nodes")
        .and_then(Value::as_array)
        .is_some_and(|nodes| {
            nodes
                .iter()
                .any(|node| node.get("type").and_then(Value::as_str) == Some("forEach"))
        })
    {
        converted_document(&value, convert::published_collection_definition)
    } else if value
        .get("nodes")
        .and_then(Value::as_array)
        .is_some_and(|nodes| {
            nodes.iter().any(|node| {
                matches!(
                    node.get("type").and_then(Value::as_str),
                    Some("while" | "repeatUntil")
                )
            })
        })
    {
        converted_document(&value, convert::published_loop_definition)
    } else {
        converted_document(&value, convert::definition)
    }
}

fn parse_document(
    document: &[u8],
    convert: fn(&Value) -> Result<WorkflowDefinition, IrError>,
) -> Result<ParsedDefinition, IrError> {
    let value = validated_document(document)?;
    converted_document(&value, convert)
}

fn read_document(document: &[u8]) -> Result<Value, IrError> {
    if document.len() > MAX_IR_BYTES {
        return Err(IrError::TooLarge);
    }
    // serde_json's recursion bound applies to authoring and stored definitions alike.
    serde_json::from_slice(document).map_err(|_| IrError::Malformed)
}

fn check_ir_structure(value: &Value) -> Result<(), IrError> {
    let issues = schema::issues(value);
    if !issues.is_empty() {
        return Err(IrError::Schema(issues));
    }
    Ok(())
}

fn validated_document(document: &[u8]) -> Result<Value, IrError> {
    let value = read_document(document)?;
    let classification = classification_check::check(&value);
    if !classification.is_publishable() {
        return Err(IrError::Classification(classification.findings));
    }
    check_ir_structure(&value)?;
    let issues = carrier_schema_check::issues(&value);
    if !issues.is_empty() {
        return Err(IrError::Schema(issues));
    }
    Ok(value)
}

fn validated_stored_document(document: &[u8]) -> Result<Value, IrError> {
    let value = read_document(document)?;
    check_ir_structure(&value)?;
    Ok(value)
}

fn converted_document(
    value: &Value,
    convert: fn(&Value) -> Result<WorkflowDefinition, IrError>,
) -> Result<ParsedDefinition, IrError> {
    let definition = convert(value)?;
    check_expressions(&definition)?;
    let canonical = canonical::canonical_json(value);
    let digest = canonical::digest_of(&canonical);
    let warnings = definition.warnings();
    Ok(ParsedDefinition {
        definition,
        canonical,
        digest,
        warnings,
    })
}

fn expression_error(node: &str, field: &'static str, error: impl ToString) -> IrError {
    IrError::Expression {
        node: node.to_owned(),
        field,
        message: error.to_string(),
    }
}

/// Every expression must compile in the bounded subset *now*, so a run never meets a syntax error.
fn check_expressions(definition: &WorkflowDefinition) -> Result<(), IrError> {
    check_node_expressions(definition.nodes())?;
    for body in definition.bodies() {
        check_node_expressions(body.nodes())?;
    }
    // A trigger's filter sees the event and its data, and nothing of any run: a run does not exist
    // yet when the filter is evaluated.
    for (i, trigger) in definition.triggers().iter().enumerate() {
        if let Some(schedule) = trigger.schedule() {
            sagat_application::dispatch::CronSchedule::validate(&cron::CronCalendar, schedule)
                .map_err(|e| expression_error(&format!("triggers[{i}]"), "cron", e))?;
        }
        if let Some(filter) = trigger.filter() {
            let compiled = Compiled::compile(filter.as_str())
                .map_err(|e| expression_error(&format!("triggers[{i}]"), "filter", e))?;
            if let Some(other) = compiled
                .variables()
                .into_iter()
                .find(|v| v != "event" && v != "data")
            {
                return Err(expression_error(
                    &format!("triggers[{i}]"),
                    "filter",
                    format!("a trigger filter reads `event` and `data`, not `{other}`"),
                ));
            }
        }
    }
    Ok(())
}

fn check_node_expressions<'a>(
    nodes: impl Iterator<Item = &'a sagat_domain::workflow::Node>,
) -> Result<(), IrError> {
    for node in nodes {
        let id = node.id().as_str();
        if let Some(operation) = node.database_operation() {
            for parameter in operation.parameters() {
                if let sagat_domain::database::Parameter::Expression { expression, .. } = parameter
                {
                    Compiled::compile(expression.as_str()).map_err(|_| {
                        expression_error(id, "parameters", "invalid database parameter expression")
                    })?;
                }
            }
        }
        match node.kind() {
            NodeKind::Condition { expression, .. } => {
                Compiled::compile(expression.as_str())
                    .map_err(|e| expression_error(id, "expression", e))?;
            }
            NodeKind::Switch { cases, .. } => {
                for case in cases {
                    Compiled::compile(case.when().as_str())
                        .map_err(|e| expression_error(id, "when", e))?;
                }
            }
            NodeKind::WaitEvent { correlation, .. } => {
                Compiled::compile(correlation.as_str())
                    .map_err(|e| expression_error(id, "correlationExpr", e))?;
            }
            NodeKind::End {
                output: Some(output),
            } => {
                Compiled::compile(output.as_str())
                    .map_err(|e| expression_error(id, "outputExpr", e))?;
            }
            NodeKind::Child {
                input: Some(input), ..
            } => {
                Compiled::compile(input.as_str()).map_err(|e| expression_error(id, "input", e))?;
            }
            NodeKind::Activity {
                idempotency_template: Some(template),
                ..
            } => {
                Template::compile(template.as_str())
                    .map_err(|e| expression_error(id, "idempotencyTemplate", e))?;
            }
            NodeKind::ForEach { items, result, .. } => {
                Compiled::compile(items.as_str()).map_err(|e| expression_error(id, "items", e))?;
                Compiled::compile(result.as_str())
                    .map_err(|e| expression_error(id, "result", e))?;
            }
            NodeKind::While {
                condition,
                initial,
                carry,
                ..
            }
            | NodeKind::RepeatUntil {
                condition,
                initial,
                carry,
                ..
            } => {
                Compiled::compile(condition.as_str())
                    .map_err(|e| expression_error(id, "condition", e))?;
                Compiled::compile(initial.as_str())
                    .map_err(|e| expression_error(id, "initial", e))?;
                Compiled::compile(carry.as_str()).map_err(|e| expression_error(id, "carry", e))?;
            }
            NodeKind::Return { result } => {
                Compiled::compile(result.as_str())
                    .map_err(|e| expression_error(id, "result", e))?;
            }
            NodeKind::ContinueAsNew { input, state } => {
                Compiled::compile(input.as_str()).map_err(|e| expression_error(id, "input", e))?;
                if let Some(state) = state {
                    Compiled::compile(state.as_str())
                        .map_err(|e| expression_error(id, "state", e))?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
