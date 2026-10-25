//! Ordinary decimal payloads must disclose (finding C1). A number is unsafe to validate only when
//! its lexeme does not survive the validator's `f64` representation.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, ClassifiedNode,
    ClassifiedPayload, PiiRevealFacts, ProjectionFailure, ProjectionRequest, ScalarValue,
};
use sagat_domain::classification::{Audience, ClassificationRules};
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};

fn project(schema: Value, payload: &[u8]) -> Result<ClassifiedPayload, ProjectionFailure> {
    let document = serde_json::to_vec(&json!({"inputSchema": schema})).expect("document");
    Classifier::default().project(ProjectionRequest {
        cache_key: ClassificationCacheKey {
            version: WorkflowVersionId::generate(),
            carrier: CarrierPointer::Input,
        },
        rules: ClassificationRules::CURRENT,
        document: &document,
        payload,
        audience: Audience::new(false, false, false),
        reveal: PiiRevealFacts {
            granted: false,
            requested: false,
        },
    })
}

fn numbers(node: &ClassifiedNode, out: &mut Vec<String>) {
    match node {
        ClassifiedNode::Value {
            value: ScalarValue::Number(number),
            ..
        } => out.push(number.as_str().to_owned()),
        ClassifiedNode::Container { children, .. } => {
            for child in children {
                numbers(&child.node, out);
            }
        }
        _ => {}
    }
}

fn disclosed_numbers(schema: Value, payload: &[u8]) -> Vec<String> {
    let projected = project(schema, payload).expect("an ordinary decimal payload must disclose");
    let mut found = Vec::new();
    numbers(&projected.tree, &mut found);
    found
}

#[test]
fn a_price_under_a_numeric_bound_discloses() {
    let schema = json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"price": {"type": "number", "minimum": 0}}
    });
    assert_eq!(disclosed_numbers(schema, br#"{"price":9.99}"#), ["9.99"]);
}

#[test]
fn a_fractional_amount_next_to_an_integer_quantity_discloses() {
    let schema = json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"amount": {"type": "number"}, "qty": {"type": "integer"}}
    });
    let mut found = disclosed_numbers(schema, br#"{"amount":12.5,"qty":2}"#);
    found.sort();
    assert_eq!(found, ["12.5", "2"]);
}

#[test]
fn decimal_bounds_in_the_schema_and_payload_agree() {
    for (schema, payload) in [
        (json!({"type": "number", "minimum": 0.5}), "0.5"),
        (json!({"type": "number", "maximum": 99.99}), "12.25"),
        (json!({"type": "number", "exclusiveMinimum": 0.1}), "0.2"),
        (json!({"type": "number", "const": 9.99}), "9.99"),
        (json!({"enum": [0.5, 1.5, "x"]}), "1.5"),
        (json!({"type": "number", "multipleOf": 0.25}), "1.75"),
        (json!({"type": "number", "multipleOf": 0.01}), "9.99"),
        (json!({"type": "number", "multipleOf": 0.01}), "0.07"),
        (json!({"type": "number", "minimum": 0}), "1.5e2"),
    ] {
        let mut schema = schema;
        schema["x-sagat-classification"] = json!("public");
        assert_eq!(
            disclosed_numbers(schema.clone(), payload.as_bytes()),
            [payload],
            "{schema} against {payload}"
        );
    }
}

#[test]
fn decimals_still_fail_the_bound_they_violate_as_a_mismatch_not_unsupported() {
    for (schema, payload) in [
        (json!({"type": "number", "minimum": 10}), "9.99"),
        (json!({"type": "number", "const": 9.99}), "9.98"),
        (json!({"type": "number", "multipleOf": 0.25}), "1.3"),
        (json!({"type": "integer"}), "12.5"),
    ] {
        let mut schema = schema;
        schema["x-sagat-classification"] = json!("public");
        assert!(
            matches!(
                project(schema.clone(), payload.as_bytes()),
                Err(ProjectionFailure::SchemaMismatch)
            ),
            "{schema} against {payload} must be a plain mismatch"
        );
    }
}

#[test]
fn unique_items_compares_decimals_by_value() {
    let schema = json!({
        "type": "array", "x-sagat-classification": "public",
        "uniqueItems": true, "items": {"type": "number"}
    });
    assert_eq!(
        disclosed_numbers(schema.clone(), b"[0.5,0.25,1.0,2]"),
        ["0.5", "0.25", "1.0", "2"]
    );
    assert!(matches!(
        project(schema.clone(), b"[0.5,0.25,0.5]"),
        Err(ProjectionFailure::SchemaMismatch)
    ));
    assert!(matches!(
        project(schema, b"[0.5,1,1.0]"),
        Err(ProjectionFailure::SchemaMismatch)
    ));
}

#[test]
fn a_lexeme_the_validator_would_round_stays_unsupported_under_a_numeric_constraint() {
    for payload in [
        "1.0000000000000000000000000001",
        "0.1000000000000000055511151231257827",
        "1e400",
        "123456789012345678.5",
    ] {
        let schema = json!({
            "type": "number", "minimum": 0, "x-sagat-classification": "public"
        });
        assert!(
            matches!(
                project(schema, payload.as_bytes()),
                Err(ProjectionFailure::UnsupportedPayload)
            ),
            "{payload} does not survive f64 and must not be validated as rounded"
        );
    }
    // Without a numeric constraint the lexeme is never compared, so it still discloses verbatim.
    let unconstrained = json!({"type": "number", "x-sagat-classification": "public"});
    assert_eq!(
        disclosed_numbers(unconstrained, b"1.0000000000000000000000000001"),
        ["1.0000000000000000000000000001"]
    );
}
