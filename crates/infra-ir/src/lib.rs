//! The workflow IR codec: JSON document in, validated domain model, canonical bytes and digest out.
//!
//! Pipeline (`docs/architecture/workflow-ir-and-sdk-modes.md`): parse, validate against the
//! published JSON Schema, convert to the domain (which proves the graph is sound), compile every
//! expression with the bounded evaluator, canonicalise and hash. Nothing here executes business
//! logic; compiling a document has no side effects.

mod canonical;
mod convert;
pub mod cron;
mod evaluator;
mod schema;

pub use evaluator::JsonEvaluator;

use sagat_application::definitions::{IrCodec, IrError, MAX_IR_BYTES, ParsedDefinition};
use sagat_domain::workflow::{NodeKind, WorkflowDefinition};
use sagat_infra_expr::{Compiled, Template};
use serde_json::Value;

/// The production [`IrCodec`].
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonIrCodec;

impl IrCodec for JsonIrCodec {
    fn check_draft(&self, document: &[u8]) -> Result<(), IrError> {
        if document.len() > MAX_IR_BYTES {
            return Err(IrError::TooLarge);
        }
        match serde_json::from_slice::<Value>(document) {
            Ok(Value::Object(_)) => Ok(()),
            _ => Err(IrError::Malformed),
        }
    }

    fn parse(&self, document: &[u8]) -> Result<ParsedDefinition, IrError> {
        if document.len() > MAX_IR_BYTES {
            return Err(IrError::TooLarge);
        }
        // `serde_json` bounds its own recursion (128 levels), so a nesting bomb is a syntax error.
        let value: Value = serde_json::from_slice(document).map_err(|_| IrError::Malformed)?;
        let issues = schema::issues(&value);
        if !issues.is_empty() {
            return Err(IrError::Schema(issues));
        }
        let definition = convert::definition(&value)?;
        check_expressions(&definition)?;
        let canonical = canonical::canonical_json(&value);
        let digest = canonical::digest_of(&canonical);
        let warnings = definition.warnings();
        Ok(ParsedDefinition {
            definition,
            canonical,
            digest,
            warnings,
        })
    }
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
    for node in definition.nodes() {
        let id = node.id().as_str();
        match node.kind() {
            NodeKind::Condition { expression, .. } => {
                Compiled::compile(expression.as_str())
                    .map_err(|e| expression_error(id, "expression", e))?;
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
            _ => {}
        }
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
