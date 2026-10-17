//! Evaluates workflow expressions over a run's data with the bounded CEL subset.
//!
//! Variables available to expressions: `input` (the run input), `steps.<node>.output` (the output of
//! a finished step), `run.id`, and, in templates, `node.id`. Only the variables an expression
//! actually mentions are parsed, so a condition on `input` never touches the step outputs.

use std::collections::BTreeSet;

use sagat_application::dispatch::EventView;
use sagat_application::execution::{
    ExpressionEvaluator, ExpressionFailure, ReconcileAnswer, RunData,
};
use sagat_domain::execution::CorrelationKey;
use sagat_domain::workflow::{Expression, NodeId};
use sagat_infra_expr::{Compiled, Env, Template};
use serde_json::{Map, Value, json};

/// The production [`ExpressionEvaluator`].
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonEvaluator;

fn failure(error: impl ToString) -> ExpressionFailure {
    ExpressionFailure(error.to_string())
}

fn parse(bytes: &[u8]) -> Result<Value, ExpressionFailure> {
    serde_json::from_slice(bytes).map_err(|_| failure("a stored payload is not valid JSON"))
}

/// Builds the environment holding just `needed`.
fn environment(
    needed: &BTreeSet<String>,
    data: &RunData,
    node: Option<&NodeId>,
) -> Result<Env, ExpressionFailure> {
    let mut env = Env::new();
    if needed.contains("input") {
        env = env.with("input", parse(&data.input)?);
    }
    if needed.contains("steps") {
        let mut steps = Map::new();
        for (id, bytes) in &data.outputs {
            steps.insert(id.as_str().to_owned(), json!({ "output": parse(bytes)? }));
        }
        env = env.with("steps", Value::Object(steps));
    }
    if needed.contains("run") {
        env = env.with("run", json!({ "id": data.run_id.map(|r| r.to_string()) }));
    }
    if needed.contains("node") {
        env = env.with("node", json!({ "id": node.map(NodeId::as_str) }));
    }
    Ok(env)
}

impl ExpressionEvaluator for JsonEvaluator {
    fn condition(
        &self,
        expression: &Expression,
        data: &RunData,
    ) -> Result<bool, ExpressionFailure> {
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let env = environment(&compiled.variables(), data, None)?;
        compiled.eval_bool(&env).map_err(failure)
    }

    fn correlation_key(
        &self,
        expression: &Expression,
        data: &RunData,
    ) -> Result<CorrelationKey, ExpressionFailure> {
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let env = environment(&compiled.variables(), data, None)?;
        let key = compiled.eval_key(&env).map_err(failure)?;
        CorrelationKey::parse(&key).map_err(failure)
    }

    fn idempotency_key(
        &self,
        template: &Expression,
        data: &RunData,
        node: &NodeId,
    ) -> Result<String, ExpressionFailure> {
        let template = Template::compile(template.as_str()).map_err(failure)?;
        let env = environment(&template.variables(), data, Some(node))?;
        template.render(&env).map_err(failure)
    }

    fn child_input(
        &self,
        expression: Option<&Expression>,
        data: &RunData,
    ) -> Result<Vec<u8>, ExpressionFailure> {
        let Some(expression) = expression else {
            return Ok(data.input.clone());
        };
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let env = environment(&compiled.variables(), data, None)?;
        let value = compiled.eval(&env).map_err(failure)?;
        if !value.is_object() {
            return Err(failure("a child input must evaluate to a JSON object"));
        }
        serde_json::to_vec(&value).map_err(failure)
    }

    fn run_result(
        &self,
        expression: &Expression,
        data: &RunData,
    ) -> Result<Vec<u8>, ExpressionFailure> {
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let env = environment(&compiled.variables(), data, None)?;
        let value = compiled.eval(&env).map_err(failure)?;
        serde_json::to_vec(&value).map_err(failure)
    }

    fn task_input(&self, data: &RunData, node: &NodeId) -> Vec<u8> {
        let input = parse(&data.input).unwrap_or(Value::Null);
        let mut steps = Map::new();
        for (id, bytes) in &data.outputs {
            if let Ok(output) = parse(bytes) {
                steps.insert(id.as_str().to_owned(), json!({ "output": output }));
            }
        }
        let document = json!({
            "run": { "id": data.run_id.map(|r| r.to_string()) },
            "node": node.as_str(),
            "input": input,
            "steps": steps,
        });
        serde_json::to_vec(&document).unwrap_or_default()
    }

    fn event_document(&self, event: &EventView<'_>) -> Vec<u8> {
        let data = parse(event.data).unwrap_or(Value::Null);
        let document = json!({
            "event": {
                "id": event.id,
                "source": event.source,
                "type": event.event_type,
                "subject": event.subject,
                "correlationKey": event.correlation_key,
                "occurredAt": event.occurred_at,
                "receivedAt": event.received_at,
            },
            "data": data,
        });
        serde_json::to_vec(&document).unwrap_or_default()
    }

    fn event_filter(
        &self,
        filter: &Expression,
        document: &[u8],
    ) -> Result<bool, ExpressionFailure> {
        let compiled = Compiled::compile(filter.as_str()).map_err(failure)?;
        let needed = compiled.variables();
        let mut env = Env::new();
        if needed.contains("event") || needed.contains("data") {
            let Value::Object(mut parts) = parse(document)? else {
                return Err(failure("an event document is an object"));
            };
            for name in ["event", "data"] {
                if needed.contains(name) {
                    env = env.with(name, parts.remove(name).unwrap_or(Value::Null));
                }
            }
        }
        compiled.eval_bool(&env).map_err(failure)
    }

    fn reconcile_answer(&self, output: &[u8]) -> Result<ReconcileAnswer, ExpressionFailure> {
        let invalid = || {
            ExpressionFailure(
                "a reconciliation answers {\"effect\": \"applied\" | \"not_applied\" | \"unknown\"}"
                    .to_owned(),
            )
        };
        let value: Value = serde_json::from_slice(output).map_err(|_| invalid())?;
        let object = value.as_object().ok_or_else(invalid)?;
        let verdict = object
            .get("effect")
            .and_then(Value::as_str)
            .and_then(|effect| effect.parse().ok())
            .ok_or_else(invalid)?;
        let output = object
            .get("output")
            .map(|value| serde_json::to_vec(value).map_err(|_| invalid()))
            .transpose()?;
        Ok(ReconcileAnswer { verdict, output })
    }

    fn is_json(&self, bytes: &[u8]) -> bool {
        serde_json::from_slice::<Value>(bytes).is_ok()
    }

    fn is_json_object(&self, bytes: &[u8]) -> bool {
        matches!(serde_json::from_slice::<Value>(bytes), Ok(Value::Object(_)))
    }
}
