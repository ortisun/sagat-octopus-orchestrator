//! Translates shared authoring programs using only the public Rust SDK. No golden access.
use sagat_octopus_orchestrator_sdk::{SagatError, Value, json, workflow::*};
use serde::de::DeserializeOwned;
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, SagatError> {
    serde_json::from_value(value)
        .map_err(|_| SagatError::validation("invalid builder options or unsupported role"))
}
fn options(value: &Value) -> Value {
    value.get("options").cloned().unwrap_or_else(|| json!({}))
}
fn apply(builder: &Builder, program: &Value) -> Result<(), SagatError> {
    for op in program
        .as_array()
        .ok_or_else(|| SagatError::validation("a program needs an array"))?
    {
        let id = text(op, "id");
        match text(op, "op") {
            "step" => {
                builder.step(id, text(op, "activity"), decode(options(op))?);
            }
            "database" => {
                builder.database(
                    id,
                    decode(options(op)).map_err(|_| {
                        SagatError::validation("invalid managed database operation")
                    })?,
                );
            }
            "sleep" => {
                builder.sleep(id, number(op, "durationMs"));
            }
            "end" => {
                builder.end(id, op["options"]["outputExpr"].as_str());
            }
            "condition" => {
                builder.condition(
                    id,
                    text(op, "expression"),
                    text(op, "ifTrue"),
                    text(op, "ifFalse"),
                );
            }
            "parallel" => {
                let mut branches = Vec::new();
                for program in op["branches"]
                    .as_array()
                    .ok_or_else(|| SagatError::validation("parallel needs branches"))?
                {
                    let flow = Builder::flow();
                    apply(&flow, program)?;
                    branches.push(flow);
                }
                builder.parallel(id, decode(op["join"].clone())?, &branches);
            }
            "saga" => {
                let flow = Builder::flow();
                apply(&flow, &op["body"])?;
                builder.saga(id, &flow, decode(options(op))?);
            }
            "waitForEvent" => {
                builder.wait_for_event(id, decode(options(op))?);
            }
            "invokeChild" => {
                builder.invoke_child(id, text(op, "workflow"), decode(options(op))?);
            }
            "switch" => {
                builder.switch(id, decode(op["cases"].clone())?, text(op, "default"));
            }
            "forEach" | "map" => {
                builder.for_each(id, decode(op.clone())?);
            }
            "while" => {
                builder.while_loop(id, decode(op.clone())?);
            }
            "repeatUntil" => {
                builder.repeat_until(id, decode(op.clone())?);
            }
            "return" => {
                builder.return_value(id, text(op, "result"));
            }
            "continueAsNew" => {
                builder.continue_as_new(
                    id,
                    text(op, "input"),
                    op.get("state").and_then(Value::as_str),
                );
            }
            "approval" => {
                builder.approval(
                    id,
                    decode(op["policy"].clone())?,
                    number(op, "timeoutMs"),
                    text(op, "onApprove"),
                    text(op, "onReject"),
                    text(op, "onExpire"),
                );
            }
            "script" => {
                builder.script(id, text(op, "artifactDigest"), decode(options(op))?);
            }
            _ => return Err(SagatError::validation("unsupported builder operation")),
        }
    }
    Ok(())
}
pub fn builder_of(definition: &Value) -> Result<Builder, SagatError> {
    let builder = Builder::new(text(definition, "name"));
    if let Some(value) = definition.get("description").and_then(Value::as_str) {
        builder.description(value);
    }
    if let Some(labels) = definition.get("labels").and_then(Value::as_object) {
        for (key, value) in labels {
            builder.label(key, value.as_str().unwrap_or_default());
        }
    }
    if let Some(triggers) = definition.get("triggers").and_then(Value::as_array) {
        for trigger in triggers {
            builder.trigger(decode(trigger.clone())?);
        }
    }
    for (key, input) in [("inputSchema", true), ("outputSchema", false)] {
        if let Some(schema) = definition.get(key) {
            if input {
                builder.input_schema(schema.clone());
            } else {
                builder.output_schema(schema.clone());
            }
        }
    }
    if let Some(limits) = definition.get("limits") {
        builder.limits(decode(limits.clone())?);
    }
    if let Some(state) = definition.get("state") {
        builder.state(
            decode(state.clone())
                .map_err(|_| SagatError::validation("invalid state disclosure or definition"))?,
        );
    }
    if let Some(policy) = definition.get("chainPolicy") {
        builder.chain_policy(number(policy, "maxSegments"), number(policy, "timeoutMs"));
    }
    if let Some(bodies) = definition.get("bodies").and_then(Value::as_object) {
        for (name, definition) in bodies {
            let body = Builder::flow();
            apply(&body, &definition["program"])?;
            builder.body(name, &body);
        }
    }
    apply(&builder, &definition["program"])?;
    Ok(builder)
}
pub fn build(scenario: &Value) -> Result<Value, SagatError> {
    let mut output = json!({"workflows":{},"canonical":{}});
    if let Some(definitions) = scenario.get("workflows").and_then(Value::as_object) {
        for (name, definition) in definitions {
            let builder = builder_of(definition)?;
            output["workflows"][name] = builder.build()?;
            output["canonical"][name] = builder.json()?.into();
        }
    }
    if let Some(definitions) = scenario.get("invalidWorkflows").and_then(Value::as_object) {
        for (name, definition) in definitions {
            let result = builder_of(&definition["workflow"]).and_then(|builder| builder.json());
            output["errors"][name] = match result {
                Ok(_) => json!({"raised":false}),
                Err(error) => {
                    json!({"raised":true,"code":error.code,"mentions":error.message.contains(text(definition,"contains"))})
                }
            };
        }
    }
    Ok(output)
}
