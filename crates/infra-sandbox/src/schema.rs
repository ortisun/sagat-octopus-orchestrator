//! JSON Schema for the input and the result of a script: checking a value against the schema the
//! author declared, and inferring a schema from a value that was seen (`FR-005-006`).

use serde_json::{Map, Value, json};

/// Why a value does not satisfy a schema, in a form that names where and which rule but never the
/// value itself: a result may hold what must not be shown.
///
/// # Errors
/// A description of up to three violations, or of a schema that is not one.
pub fn validate(schema: &Value, instance: &Value) -> Result<(), String> {
    let validator =
        jsonschema::validator_for(schema).map_err(|_| "the schema is not valid".to_owned())?;
    let mut problems: Vec<String> = Vec::new();
    for error in validator.iter_errors(instance).take(3) {
        let at = error.instance_path().to_string();
        let kind = format!("{:?}", error.kind());
        let rule = kind
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or("violation");
        problems.push(format!(
            "{} breaks `{rule}`",
            if at.is_empty() { "/" } else { &at }
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// A JSON Schema that `value` satisfies and that says as much as one sample can: the type of every
/// property, which are present, and the type of the items of an array (merged across items).
#[must_use]
pub fn infer(value: &Value) -> Value {
    match value {
        Value::Null => json!({"type": "null"}),
        Value::Bool(_) => json!({"type": "boolean"}),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({"type": "integer"}),
        Value::Number(_) => json!({"type": "number"}),
        Value::String(_) => json!({"type": "string"}),
        Value::Array(items) => {
            let mut schema = json!({"type": "array"});
            if let Some(merged) = items.iter().map(infer).reduce(merge) {
                schema["items"] = merged;
            }
            schema
        }
        Value::Object(map) => {
            let mut properties = Map::new();
            for (key, item) in map {
                properties.insert(key.clone(), infer(item));
            }
            let required: Vec<&String> = map.keys().collect();
            json!({"type": "object", "properties": properties, "required": required})
        }
    }
}

/// The schema both of two inferred schemas satisfy: equal schemas stay, two of different types
/// become a union of the types, and objects keep the properties of both with only the common ones
/// required.
fn merge(a: Value, b: Value) -> Value {
    if a == b {
        return a;
    }
    let (ta, tb) = (a["type"].as_str(), b["type"].as_str());
    if ta == Some("object") && tb == Some("object") {
        let mut properties = a["properties"].as_object().cloned().unwrap_or_default();
        for (key, schema) in b["properties"].as_object().cloned().unwrap_or_default() {
            let merged = match properties.remove(&key) {
                Some(existing) => merge(existing, schema),
                None => schema,
            };
            properties.insert(key, merged);
        }
        let required: Vec<Value> = a["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|key| b["required"].as_array().is_some_and(|r| r.contains(key)))
            .cloned()
            .collect();
        return json!({"type": "object", "properties": properties, "required": required});
    }
    if ta == Some("array") && tb == Some("array") {
        let mut schema = json!({"type": "array"});
        match (a.get("items").cloned(), b.get("items").cloned()) {
            (Some(x), Some(y)) => schema["items"] = merge(x, y),
            (Some(x), None) | (None, Some(x)) => schema["items"] = x,
            (None, None) => {}
        }
        return schema;
    }
    let mut types: Vec<Value> = Vec::new();
    for schema in [&a, &b] {
        match &schema["type"] {
            Value::Array(list) => types.extend(list.iter().cloned()),
            other => types.push(other.clone()),
        }
    }
    types.sort_by_key(ToString::to_string);
    types.dedup();
    // An integer is a number.
    if types.contains(&json!("number")) {
        types.retain(|t| t != &json!("integer"));
    }
    if types.len() == 1 {
        return json!({"type": types[0]});
    }
    json!({"type": types})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_checked_against_a_schema_and_a_violation_never_shows_the_value() {
        let schema = json!({
            "type": "object",
            "properties": {"amount": {"type": "integer", "minimum": 1}, "card": {"type": "string"}},
            "required": ["amount"]
        });
        assert_eq!(
            validate(&schema, &json!({"amount": 5, "card": "4111"})),
            Ok(())
        );
        let problem =
            validate(&schema, &json!({"amount": "sk_live_secretvalue"})).expect_err("a violation");
        assert!(problem.contains("/amount"), "{problem}");
        assert!(
            !problem.contains("sk_live"),
            "the value stays out of the message: {problem}"
        );
        let missing = validate(&schema, &json!({})).expect_err("a violation");
        assert!(
            missing.contains("required") || missing.contains("Required"),
            "{missing}"
        );
        assert!(
            validate(&json!({"type": "nonsense"}), &json!(1)).is_err(),
            "a schema that is not one"
        );
    }

    #[test]
    fn a_schema_inferred_from_a_value_is_satisfied_by_it() {
        let sample = json!({
            "id": 7, "price": 9.5, "name": "n", "ok": true, "none": null,
            "tags": ["a", "b"], "nested": {"deep": [1, 2, 3]}
        });
        let schema = infer(&sample);
        assert_eq!(validate(&schema, &sample), Ok(()));
        assert_eq!(schema["properties"]["id"], json!({"type": "integer"}));
        assert_eq!(schema["properties"]["price"], json!({"type": "number"}));
        assert_eq!(
            schema["properties"]["tags"],
            json!({"type": "array", "items": {"type": "string"}})
        );
        assert_eq!(schema["required"].as_array().map(Vec::len), Some(7));
    }

    #[test]
    fn the_items_of_an_array_are_merged_into_one_schema_they_all_satisfy() {
        let sample = json!([{"a": 1, "b": "x"}, {"a": 2}, {"a": 2.5, "c": null}]);
        let schema = infer(&sample);
        assert_eq!(validate(&schema, &sample), Ok(()));
        let item = &schema["items"];
        assert_eq!(
            item["properties"]["a"],
            json!({"type": "number"}),
            "an integer and a number are a number"
        );
        assert_eq!(
            item["required"],
            json!(["a"]),
            "only what every item has is required"
        );
        assert_eq!(
            infer(&json!([1, "a"]))["items"],
            json!({"type": ["integer", "string"]})
        );
        assert_eq!(infer(&json!([])), json!({"type": "array"}));
    }
}
