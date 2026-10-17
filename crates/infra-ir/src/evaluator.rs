//! Evaluates workflow expressions over a run's data with the bounded CEL subset.
//!
//! Variables available to expressions: `input` (the run input), `steps.<node>.output` (the output of
//! a finished step), `run.id`, and, in templates, `node.id`. Only the variables an expression
//! actually mentions are parsed, so a condition on `input` never touches the step outputs.

use std::collections::BTreeSet;

use sagat_application::collection_source::{CollectionSource, descriptor_from_json};
use sagat_application::dispatch::EventView;
use sagat_application::execution::{
    ExpressionEvaluator, ExpressionFailure, ReconcileAnswer, RunData, ScopeInput,
};
use sagat_application::loop_progress::{LoopCheckpointEvaluation, LoopEntryEvaluation};
use sagat_domain::execution::{CorrelationKey, ExecutionInstanceId, InstancePath};
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
    instance: Option<&ExecutionInstanceId>,
) -> Result<Env, ExpressionFailure> {
    let mut env = Env::new();
    if needed.contains("input") {
        env = env.with("input", parse(&data.input)?);
    }
    if needed.contains("steps") {
        let mut steps = Map::new();
        let path = instance.map_or_else(InstancePath::root, |value| value.path().clone());
        for (id, bytes) in data.outputs_in(&path) {
            steps.insert(id.as_str().to_owned(), json!({ "output": parse(bytes)? }));
        }
        env = env.with("steps", Value::Object(steps));
    }
    if needed.contains("run") {
        env = env.with("run", json!({ "id": data.run_id.map(|r| r.to_string()) }));
    }
    if needed.contains("node") {
        env = env.with(
            "node",
            json!({ "id": instance.map(ExecutionInstanceId::node).map(NodeId::as_str) }),
        );
    }
    if needed.contains("state") {
        env = env.with(
            "state",
            data.state
                .as_deref()
                .map(parse)
                .transpose()?
                .unwrap_or(Value::Null),
        );
    }
    if needed
        .iter()
        .any(|name| matches!(name.as_str(), "item" | "index" | "iteration" | "carry"))
    {
        let instance = instance.ok_or_else(|| failure("an iteration context is required"))?;
        let scope = data
            .scope_input(instance.path())
            .ok_or_else(|| failure("the admitted iteration input is unavailable"))?;
        match scope {
            ScopeInput::Collection { index, item } => {
                if needed.contains("item") {
                    env = env.with("item", parse(item)?);
                }
                if needed.contains("index") {
                    env = env.with("index", json!(index));
                }
            }
            ScopeInput::Loop { iteration, carry } => {
                if needed.contains("iteration") {
                    env = env.with("iteration", json!(iteration));
                }
                if needed.contains("carry") {
                    env = env.with("carry", parse(carry)?);
                }
            }
        }
    }
    Ok(env)
}

fn scoped_task_input(
    data: &RunData,
    instance: &ExecutionInstanceId,
) -> Result<Vec<u8>, ExpressionFailure> {
    let input = parse(&data.input)?;
    let state = data
        .state
        .as_deref()
        .map(parse)
        .transpose()?
        .unwrap_or(Value::Null);
    let mut steps = Map::new();
    for (id, bytes) in data.outputs_in(instance.path()) {
        steps.insert(id.as_str().to_owned(), json!({ "output": parse(bytes)? }));
    }
    let mut document = Map::new();
    document.insert("input".to_owned(), input);
    document.insert("node".to_owned(), json!(instance.node().as_str()));
    document.insert(
        "run".to_owned(),
        json!({ "id": data.run_id.map(|run| run.to_string()) }),
    );
    document.insert("state".to_owned(), state);
    document.insert("steps".to_owned(), Value::Object(steps));
    match data
        .scope_input(instance.path())
        .ok_or_else(|| failure("the admitted iteration input is unavailable"))?
    {
        ScopeInput::Collection { index, item } => {
            document.insert("index".to_owned(), json!(index));
            document.insert("item".to_owned(), parse(item)?);
        }
        ScopeInput::Loop { iteration, carry } => {
            document.insert("carry".to_owned(), parse(carry)?);
            document.insert("iteration".to_owned(), json!(iteration));
        }
    }
    serde_json::to_vec(&Value::Object(document)).map_err(failure)
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
        let instance = ExecutionInstanceId::root(node.clone());
        let env = environment(&template.variables(), data, Some(&instance))?;
        template.render(&env).map_err(failure)
    }

    fn idempotency_key_instance(
        &self,
        template: &Expression,
        data: &RunData,
        instance: &ExecutionInstanceId,
    ) -> Result<String, ExpressionFailure> {
        let template = Template::compile(template.as_str()).map_err(failure)?;
        let env = environment(&template.variables(), data, Some(instance))?;
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

    fn continuation_value(
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
        for (id, bytes) in data.root_outputs() {
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

    fn task_input_instance(
        &self,
        data: &RunData,
        instance: &ExecutionInstanceId,
    ) -> Result<Vec<u8>, ExpressionFailure> {
        if instance.path().is_root() {
            return Ok(self.task_input(data, instance.node()));
        }
        scoped_task_input(data, instance)
    }

    fn collection_items(
        &self,
        expression: &Expression,
        data: &RunData,
    ) -> Result<Vec<Vec<u8>>, ExpressionFailure> {
        match self.collection_source(expression, data)? {
            CollectionSource::Items(items) => Ok(items),
            CollectionSource::Blob(_) => Err(failure(
                "a collection items expression must yield an array, not a blob reference",
            )),
        }
    }

    fn collection_source(
        &self,
        expression: &Expression,
        data: &RunData,
    ) -> Result<CollectionSource, ExpressionFailure> {
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let value = compiled
            .eval(&environment(&compiled.variables(), data, None)?)
            .map_err(failure)?;
        match value {
            Value::Array(items) => items
                .into_iter()
                .map(|item| serde_json::to_vec(&item).map_err(failure))
                .collect::<Result<_, _>>()
                .map(CollectionSource::Items),
            other => descriptor_from_json(&other)
                .map(CollectionSource::Blob)
                .ok_or_else(|| {
                    failure("a collection items expression must yield an array or a blob reference")
                }),
        }
    }

    fn collection_return(
        &self,
        expression: &Expression,
        data: &RunData,
        instance: &ExecutionInstanceId,
    ) -> Result<Vec<u8>, ExpressionFailure> {
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let value = compiled
            .eval(&environment(&compiled.variables(), data, Some(instance))?)
            .map_err(failure)?;
        serde_json::to_vec(&value).map_err(failure)
    }

    fn collection_member_result(
        &self,
        expression: &Expression,
        data: &RunData,
        instance: &ExecutionInstanceId,
        body_result: &[u8],
    ) -> Result<Vec<u8>, ExpressionFailure> {
        let compiled = Compiled::compile(expression.as_str()).map_err(failure)?;
        let mut env = environment(&compiled.variables(), data, Some(instance))?;
        if compiled.variables().contains("bodyResult") {
            env = env.with("bodyResult", parse(body_result)?);
        }
        let value = compiled.eval(&env).map_err(failure)?;
        serde_json::to_vec(&value).map_err(failure)
    }

    fn loop_entry(
        &self,
        initial: &Expression,
        condition: Option<&Expression>,
        data: &RunData,
    ) -> Result<LoopEntryEvaluation, ExpressionFailure> {
        let initial = Compiled::compile(initial.as_str()).map_err(failure)?;
        let carry = initial
            .eval(&environment(&initial.variables(), data, None)?)
            .map_err(failure)?;
        let evaluated_condition = condition
            .map(|condition| {
                let condition = Compiled::compile(condition.as_str()).map_err(failure)?;
                let mut variables = condition.variables();
                variables.remove("carry");
                let mut env = environment(&variables, data, None)?;
                if condition.variables().contains("carry") {
                    env = env.with("carry", carry.clone());
                }
                condition.eval_bool(&env).map_err(failure)
            })
            .transpose()?;
        Ok(LoopEntryEvaluation {
            carry: serde_json::to_vec(&carry).map_err(failure)?,
            condition: evaluated_condition,
        })
    }

    fn loop_checkpoint(
        &self,
        result: &Expression,
        carry: &Expression,
        condition: &Expression,
        data: &RunData,
        return_instance: &ExecutionInstanceId,
    ) -> Result<LoopCheckpointEvaluation, ExpressionFailure> {
        if !matches!(
            data.scope_input(return_instance.path()),
            Some(ScopeInput::Loop { .. })
        ) {
            return Err(failure("the admitted iteration input is unavailable"));
        }
        let result = Compiled::compile(result.as_str()).map_err(failure)?;
        let result_value = result
            .eval(&environment(
                &result.variables(),
                data,
                Some(return_instance),
            )?)
            .map_err(failure)?;
        let carry = Compiled::compile(carry.as_str()).map_err(failure)?;
        let mut carry_env = environment(&carry.variables(), data, Some(return_instance))?;
        if carry.variables().contains("bodyResult") {
            carry_env = carry_env.with("bodyResult", result_value.clone());
        }
        let carry_value = carry.eval(&carry_env).map_err(failure)?;
        let condition = Compiled::compile(condition.as_str()).map_err(failure)?;
        let mut condition_env = environment(&condition.variables(), data, Some(return_instance))?;
        if condition.variables().contains("carry") {
            condition_env = condition_env.with("carry", carry_value.clone());
        }
        if condition.variables().contains("bodyResult") {
            condition_env = condition_env.with("bodyResult", result_value.clone());
        }
        let condition_value = condition.eval_bool(&condition_env).map_err(failure)?;
        Ok(LoopCheckpointEvaluation {
            result: serde_json::to_vec(&result_value).map_err(failure)?,
            carry: serde_json::to_vec(&carry_value).map_err(failure)?,
            condition: condition_value,
        })
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
