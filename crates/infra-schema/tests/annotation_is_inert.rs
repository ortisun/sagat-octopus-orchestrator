#![allow(clippy::expect_used)]

use jsonschema::PatternOptions;
use serde_json::{Value, json};

fn compile(schema: &Value) -> jsonschema::Validator {
    jsonschema::options()
        .with_pattern_options(
            PatternOptions::regex()
                .size_limit(1024 * 1024)
                .dfa_size_limit(1024 * 1024),
        )
        .build(schema)
        .expect("schema compiles")
}

#[test]
fn classification_annotation_is_inert_to_jsonschema_validation() {
    let plain = json!({
        "type": "object",
        "required": ["id"],
        "properties": {"id": {"type": "integer"}}
    });
    let annotated = json!({
        "type": "object",
        "x-sagat-classification": "internal",
        "required": ["id"],
        "properties": {"id": {"type": "integer", "x-sagat-classification": "pii"}}
    });

    for instance in [json!({"id": 7}), json!({"id": "wrong"}), json!({})] {
        assert_eq!(
            compile(&plain).is_valid(&instance),
            compile(&annotated).is_valid(&instance)
        );
    }
}
