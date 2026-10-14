//! JSON Schema for the input and the result of a script: checking a schema when it is written,
//! checking a value against the schema the author declared, and inferring a schema from a value
//! that was seen (`FR-005-006`), and the format of the artifact bundle.

pub mod bundle;

use sagat_application::scripts::{SchemaChecker, SchemaProblem};
use sagat_domain::scripts::MAX_SCHEMA_BYTES;
use serde_json::{Map, Value, json};

/// Checks schemas when an author writes them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Schemas;

impl sagat_application::blobs::RunInputChecker for Schemas {
    fn check(&self, workflow: &[u8], input: &[u8]) -> Result<(), &'static str> {
        let workflow: Value = serde_json::from_slice(workflow)
            .map_err(|_| "the stored workflow is not valid JSON")?;
        let input: Value =
            serde_json::from_slice(input).map_err(|_| "the run input is not valid JSON")?;
        let Some(schema) = workflow.get("inputSchema") else {
            return Ok(());
        };
        let validator = compile(schema).map_err(|()| "the workflow input schema is not valid")?;
        if validator.is_valid(&input) {
            Ok(())
        } else {
            Err("the blob descriptor does not satisfy the workflow input schema")
        }
    }
}

#[cfg(test)]
mod run_input_tests {
    use super::*;
    use sagat_application::blobs::RunInputChecker;

    #[test]
    fn claim_check_schema_validation_refuses_external_refs_without_exposing_payloads() {
        let input = br#"{"$blob":{"id":"private-reference"}}"#;
        for reference in ["https://example.invalid/schema.json", "file:///etc/passwd"] {
            let workflow = json!({"inputSchema":{"$ref":reference}}).to_string();
            let error = RunInputChecker::check(&Schemas, workflow.as_bytes(), input)
                .expect_err("external ref");
            assert_eq!(error, "the workflow input schema is not valid");
            assert!(!error.contains("private-reference"));
        }
    }
}

impl SchemaChecker for Schemas {
    fn check(&self, schema: &[u8]) -> Result<(), SchemaProblem> {
        if schema.len() > MAX_SCHEMA_BYTES {
            return Err(SchemaProblem::TooLarge);
        }
        let value: Value = serde_json::from_slice(schema).map_err(|_| SchemaProblem::NotJson)?;
        if !value.is_object() {
            return Err(SchemaProblem::NotAnObject);
        }
        // Compiles it, which also refuses a reference to anything outside the document.
        compile(&value).map_err(|_| SchemaProblem::Invalid)?;
        Ok(())
    }
}

/// Largest compiled size of one `pattern`, in bytes: the author chooses the pattern.
const PATTERN_SIZE_LIMIT: usize = 1024 * 1024;

/// Compiles a schema the author wrote. Its `pattern`s are matched with the linear-time engine: the
/// backtracking one (lookaround, backreferences) can be made to spend a million steps per string on
/// every string of a result, on the host and outside any container's limits, so a schema that needs
/// it is refused when it is written instead of being evaluated.
fn compile(schema: &Value) -> Result<jsonschema::Validator, ()> {
    jsonschema::options()
        .with_pattern_options(
            jsonschema::PatternOptions::regex()
                .size_limit(PATTERN_SIZE_LIMIT)
                .dfa_size_limit(PATTERN_SIZE_LIMIT),
        )
        .build(schema)
        .map_err(|_| ())
}

/// Why a value does not satisfy a schema, in a form that names where and which rule but never the
/// value itself: a result may hold what must not be shown.
///
/// # Errors
/// A description of up to three violations, or of a schema that is not one.
pub fn validate(schema: &Value, instance: &Value) -> Result<(), String> {
    let validator = compile(schema).map_err(|()| "the schema is not valid".to_owned())?;
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
    fn a_pattern_that_needs_backtracking_is_refused_when_the_schema_is_written() {
        let check = |pattern: &str| {
            Schemas.check(
                json!({"type": "string", "pattern": pattern})
                    .to_string()
                    .as_bytes(),
            )
        };
        assert_eq!(check("^[a-z]{2,8}-\\d+$"), Ok(()));
        for risky in ["^(?=a)a$", "(a+)\\1", "^(?!x).*$", "(?<=a)b"] {
            assert_eq!(check(risky), Err(SchemaProblem::Invalid), "{risky}");
        }
        assert!(
            validate(
                &json!({"type": "string", "pattern": "^(?=a)a$"}),
                &json!("a")
            )
            .is_err(),
            "and a value is never checked against one"
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
