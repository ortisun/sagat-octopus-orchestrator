//! The walker runs inside the disclosure read transaction, so its memory and CPU must be bounded by
//! the payload and the carrier together, never by their product (security review S1, S7;
//! correctness review C2). The tests read the walk's own cost counters, never a timer.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, ClassifiedNode,
    ClassifiedPayload, MAX_PAYLOAD_BYTES, PiiRevealFacts, ProjectionFailure, ProjectionRequest,
};
use sagat_domain::classification::{Audience, ClassificationRules};
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};

fn request<'a>(document: &'a [u8], payload: &'a [u8], inspect: bool) -> ProjectionRequest<'a> {
    ProjectionRequest {
        cache_key: ClassificationCacheKey {
            version: WorkflowVersionId::generate(),
            carrier: CarrierPointer::Input,
        },
        rules: ClassificationRules::CURRENT,
        document,
        payload,
        audience: Audience::new(inspect, false, inspect),
        reveal: PiiRevealFacts {
            granted: false,
            requested: false,
        },
    }
}

fn document(schema: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"inputSchema": schema})).expect("document")
}

fn count_nodes(node: &ClassifiedNode) -> usize {
    match node {
        ClassifiedNode::Container { children, .. } => {
            1 + children.iter().map(|c| count_nodes(&c.node)).sum::<usize>()
        }
        _ => 1,
    }
}

/// 120 nested keys of 1 KiB, then an array of 60,000 integers: the review's first payload. The
/// path of every integer used to be a 120 KiB string kept for the whole walk (about 7 GB).
fn deep_key_payload() -> Vec<u8> {
    let mut leaf = String::from("[");
    for index in 0..60_000 {
        if index > 0 {
            leaf.push(',');
        }
        leaf.push('1');
    }
    leaf.push(']');
    let mut text = leaf;
    for level in (0..120).rev() {
        let key = format!("{level:03}{}", "k".repeat(1021));
        text = format!("{{\"{key}\":{text}}}");
    }
    assert!(text.len() <= MAX_PAYLOAD_BYTES, "{} bytes", text.len());
    text.into_bytes()
}

#[test]
fn nested_long_keys_do_not_multiply_into_per_number_paths() {
    let document = document(&json!({
        "x-sagat-classification": "public",
        "additionalProperties": true
    }));
    let payload = deep_key_payload();
    let classifier = Classifier::default();
    let (answer, counters) = classifier.project_counted(request(&document, &payload, true));
    // The bookkeeping that carries number spellings is linear in the payload (a tree node and a
    // spelling per number), never one path per number: 60,000 numbers under a 120 KiB prefix
    // used to hold about 7 GB.
    assert!(
        counters.retained_bytes <= 32 * payload.len(),
        "the walk kept {} bytes for a {} byte payload",
        counters.retained_bytes,
        payload.len()
    );
    assert!(counters.steps <= counters.limit);
    // Whatever the answer, it is a closed outcome and carries no payload text.
    match answer {
        Ok(projected) => assert!(count_nodes(&projected.tree) <= 1_024),
        Err(failure) => assert!(!matches!(failure, ProjectionFailure::LegacyVersion)),
    }
}

/// 20,000 empty `allOf` branches fit in a 64 KiB carrier; every array element used to expand all
/// of them again, and the validator repeated the work for each element.
#[test]
fn a_wide_all_of_under_items_cannot_be_repeated_for_every_element() {
    let document = document(&json!({
        "type": "array", "x-sagat-classification": "public",
        "items": {"allOf": vec![json!({}); 20_000]}
    }));
    assert!(document.len() < 64 * 1024);
    let payload = format!("[{}1]", "1,".repeat((MAX_PAYLOAD_BYTES - 4) / 2)).into_bytes();
    assert!(payload.len() <= MAX_PAYLOAD_BYTES);
    let classifier = Classifier::default();
    let (answer, counters) = classifier.project_counted(request(&document, &payload, true));
    assert_eq!(answer.map(|_| ()), Err(ProjectionFailure::SchemaUnusable));
    assert!(
        counters.steps <= counters.limit,
        "{} steps against a limit of {}",
        counters.steps,
        counters.limit
    );
}

/// The validator evaluates every expanded branch for every element; the walk charges the same
/// cost before it ever calls the validator.
#[test]
fn branches_that_the_validator_would_repeat_per_element_are_charged_up_front() {
    let document = document(&json!({
        "type": "array", "x-sagat-classification": "public",
        "items": {"allOf": vec![json!({"type": "integer"}); 3_000]}
    }));
    assert!(document.len() < 64 * 1024);
    let payload = format!("[{}1]", "1,".repeat(100_000)).into_bytes();
    let classifier = Classifier::default();
    let (answer, counters) = classifier.project_counted(request(&document, &payload, true));
    assert_eq!(answer.map(|_| ()), Err(ProjectionFailure::SchemaUnusable));
    assert!(counters.steps <= counters.limit);
}

/// A schema the author wrote honestly still discloses: one `items` schema with a handful of
/// branches over a long array is linear in the array and nowhere near the limit.
#[test]
fn a_long_array_under_a_small_schema_stays_far_from_the_limit() {
    let document = document(&json!({
        "type": "array", "x-sagat-classification": "public",
        "items": {"allOf": [{"type": "integer"}, {"minimum": 0}]}
    }));
    let payload = format!("[{}1]", "1,".repeat((MAX_PAYLOAD_BYTES - 4) / 2)).into_bytes();
    let classifier = Classifier::default();
    let (answer, counters): (Result<ClassifiedPayload, _>, _) =
        classifier.project_counted(request(&document, &payload, false));
    let projected = answer.expect("a long honest array discloses");
    assert!(projected.truncated, "only 128 children are returned");
    assert!(
        counters.steps * 4 < counters.limit,
        "{} of {} steps",
        counters.steps,
        counters.limit
    );
}

/// C2/S7: the 1,000 reference limit applies to one schema location, not to the whole instance.
#[test]
fn an_array_of_more_than_a_thousand_referenced_items_still_discloses() {
    let document = document(&json!({
        "type": "array", "x-sagat-classification": "public",
        "$defs": {"line": {
            "type": "object",
            "properties": {
                "sku": {"$ref": "#/$defs/text"},
                "note": {"$ref": "#/$defs/text"},
                "qty": {"$ref": "#/$defs/count"},
                "unit": {"$ref": "#/$defs/text"},
                "tag": {"$ref": "#/$defs/text"}
            }
        }, "text": {"type": "string"}, "count": {"type": "integer"}},
        "items": {"$ref": "#/$defs/line"}
    }));
    let line = r#"{"sku":"a","note":"b","qty":1,"unit":"c","tag":"d"},"#;
    let payload = format!("[{}{}]", line.repeat(1_500), line.trim_end_matches(',')).into_bytes();
    assert!(payload.len() <= MAX_PAYLOAD_BYTES, "{}", payload.len());
    let classifier = Classifier::default();
    let (answer, _) = classifier.project_counted(request(&document, &payload, false));
    let projected = answer.expect("1,501 items with five references each disclose");
    let ClassifiedNode::Container { children, .. } = projected.tree else {
        panic!("array root")
    };
    assert_eq!(children.len(), 128);
    assert!(projected.truncated);
}

/// Cycles still end as an unusable carrier, now that the limit is per location.
#[test]
fn a_reference_cycle_is_still_unusable() {
    let document = document(&json!({
        "x-sagat-classification": "public",
        "$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"$ref": "#/$defs/a"}},
        "$ref": "#/$defs/a"
    }));
    let answer = Classifier::default().project(request(&document, b"1", false));
    assert_eq!(answer.map(|_| ()), Err(ProjectionFailure::SchemaUnusable));
}

/// The validator also evaluates keywords the walk does not visit (`not`, `if`, `contains`...).
/// When such a keyword descends into children it repeats for every descendant, so a branch that
/// hides a wide `allOf` behind `not` is charged per descendant before the validator runs.
#[test]
fn a_descending_keyword_outside_the_walked_set_is_charged_per_descendant() {
    let document = document(&json!({
        "type": "array", "x-sagat-classification": "public",
        "not": {"items": {"allOf": vec![json!({"type": "string"}); 3_000]}}
    }));
    assert!(document.len() < 64 * 1024);
    let payload = format!("[{}1]", "1,".repeat(100_000)).into_bytes();
    let (answer, counters) =
        Classifier::default().project_counted(request(&document, &payload, true));
    assert_eq!(answer.map(|_| ()), Err(ProjectionFailure::SchemaUnusable));
    assert!(counters.steps <= counters.limit);
}

/// A key no schema names costs the same as any other unnamed key: the context is computed once
/// per object, so tens of thousands of hidden keys do not multiply a wide `allOf`.
#[test]
fn many_undeclared_keys_do_not_repeat_the_additional_properties_expansion() {
    let document = document(&json!({
        "type": "object", "x-sagat-classification": "public",
        "additionalProperties": {"allOf": vec![json!({}); 20_000]}
    }));
    assert!(document.len() < 64 * 1024);
    let mut payload = String::from("{");
    let mut index = 0usize;
    while payload.len() < MAX_PAYLOAD_BYTES - 16 {
        if index > 0 {
            payload.push(',');
        }
        payload.push_str(&format!("\"{index}\":1"));
        index += 1;
    }
    payload.push('}');
    let (answer, counters) =
        Classifier::default().project_counted(request(&document, payload.as_bytes(), true));
    assert_eq!(answer.map(|_| ()), Err(ProjectionFailure::SchemaUnusable));
    assert!(counters.steps <= counters.limit);
}
