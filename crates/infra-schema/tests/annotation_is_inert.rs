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

fn without_annotations(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| key.as_str() != "x-sagat-classification")
                .map(|(key, value)| (key.clone(), without_annotations(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(without_annotations).collect()),
        other => other.clone(),
    }
}

#[test]
fn annotations_at_every_sound_corpus_position_preserve_validation() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/payload-classification/fixtures");
    let mut instances = vec![
        json!(null),
        json!(true),
        json!(7),
        json!("text"),
        json!([]),
        json!({}),
        json!({"id":"id", "staff":"staff", "person":"person", "credential":"credential"}),
        json!({"all":"text", "any":"text", "one":7}),
        json!({"all":7, "any":7, "one":false}),
        json!({"closed":{"id":"id"}, "open":{"extra":"value"}, "tuple":["first","second",3]}),
        json!({"closed":{"id":"id", "extra":true}, "tuple":[7]}),
    ];
    for entry in std::fs::read_dir(fixtures.join("payloads")).expect("payload corpus") {
        let file = entry.expect("payload entry").path();
        if file
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            instances.push(
                serde_json::from_slice(&std::fs::read(file).expect("payload bytes"))
                    .expect("payload JSON"),
            );
        }
    }
    let mut schemas = 0;
    for entry in std::fs::read_dir(fixtures.join("sound")).expect("sound corpus") {
        let file = entry.expect("sound entry").path();
        if !file
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            continue;
        }
        let document: Value =
            serde_json::from_slice(&std::fs::read(&file).expect("document bytes"))
                .expect("document JSON");
        let annotated = document.get("inputSchema").expect("sound input carrier");
        let plain = without_annotations(annotated);
        let annotated = compile(annotated);
        let plain = compile(&plain);
        for instance in &instances {
            assert_eq!(
                annotated.is_valid(instance),
                plain.is_valid(instance),
                "annotation changed validation for corpus schema {}",
                file.file_name().expect("fixture name").to_string_lossy(),
            );
        }
        schemas += 1;
    }
    assert!(schemas >= 3, "the complete sound corpus must be exercised");
}
