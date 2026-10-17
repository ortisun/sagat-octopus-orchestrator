#![allow(clippy::expect_used)]

use sagat_application::classification::{
    CarrierPointer, ClassificationCacheKey, ClassificationInterpreter, ClassifiedNode,
    PiiRevealFacts, ProjectionFailure, ProjectionRequest, ScalarValue,
};
use sagat_domain::classification::{Audience, ClassificationRules, Level, WithheldReason};
use sagat_domain::definition::WorkflowVersionId;
use sagat_infra_schema::classification::Classifier;
use serde_json::{Value, json};

fn request<'a>(document: &'a [u8], payload: &'a [u8], audience: Audience) -> ProjectionRequest<'a> {
    ProjectionRequest {
        cache_key: ClassificationCacheKey {
            version: WorkflowVersionId::generate(),
            carrier: CarrierPointer::Input,
        },
        rules: ClassificationRules::CURRENT,
        document,
        payload,
        audience,
        reveal: PiiRevealFacts {
            granted: false,
            requested: false,
        },
    }
}

fn document(schema: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"inputSchema": schema})).expect("document")
}

#[test]
fn effective_levels_and_withheld_reasons_follow_the_matrix() {
    let doc = document(json!({
        "type": "object",
        "x-sagat-classification": "public",
        "properties": {
            "id": {"type": "string"},
            "staff": {"type": "string", "x-sagat-classification": "internal"},
            "person": {"type": "string", "x-sagat-classification": "pii"},
            "credential": {"type": "string", "x-sagat-classification": "secret"}
        },
        "additionalProperties": false
    }));
    let payload = br#"{"id":"1","staff":"ops","person":"Ada","credential":"never"}"#;
    let classifier = Classifier::default();
    let mut input = request(&doc, payload, Audience::new(true, false, true));
    input.reveal = PiiRevealFacts {
        granted: true,
        requested: false,
    };
    let result = classifier.project(input).expect("projection");

    let ClassifiedNode::Container { children, .. } = result.tree else {
        panic!("root container")
    };
    assert_eq!(children.len(), 4);
    let child = |name: &str| {
        children
            .iter()
            .find(|child| child.name.as_deref() == Some(name))
            .expect("declared child")
    };
    assert!(matches!(
        child("id").node.as_ref(),
        ClassifiedNode::Value {
            level: Level::Public,
            ..
        }
    ));
    assert!(matches!(
        child("staff").node.as_ref(),
        ClassifiedNode::Value {
            level: Level::Internal,
            ..
        }
    ));
    assert!(matches!(
        child("person").node.as_ref(),
        ClassifiedNode::Withheld {
            level: Level::Pii,
            reason: WithheldReason::RevealNotRequested,
            ..
        }
    ));
    assert!(matches!(
        child("credential").node.as_ref(),
        ClassifiedNode::Withheld {
            level: Level::Secret,
            reason: WithheldReason::Secret,
            ..
        }
    ));
}

#[test]
fn undeclared_keys_are_pii_and_are_not_named_when_withheld() {
    let doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "additionalProperties": true
    }));
    let classifier = Classifier::default();
    let result = classifier
        .project(request(
            &doc,
            br#"{"private-key":"canary"}"#,
            Audience::new(true, false, true),
        ))
        .expect("projection");
    let ClassifiedNode::Container { ref children, .. } = result.tree else {
        panic!("container")
    };
    assert_eq!(children[0].name, None);
    assert!(matches!(
        children[0].node.as_ref(),
        ClassifiedNode::Withheld {
            level: Level::Pii,
            ..
        }
    ));
    assert!(!format!("{result:?}").contains("canary"));
    assert!(!format!("{result:?}").contains("private-key"));
}

#[test]
fn strictest_combiner_and_local_ref_annotation_wins() {
    let doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "$defs": {"protected": {"type": "string", "x-sagat-classification": "pii"}},
        "properties": {"value": {
            "allOf": [
                {"type": "string", "x-sagat-classification": "internal"},
                {"$ref": "#/$defs/protected"}
            ]
        }}
    }));
    let result = Classifier::default()
        .project(request(
            &doc,
            br#"{"value":"canary"}"#,
            Audience::new(true, false, true),
        ))
        .expect("projection");
    let ClassifiedNode::Container { children, .. } = result.tree else {
        panic!("container")
    };
    assert!(matches!(
        children[0].node.as_ref(),
        ClassifiedNode::Withheld {
            level: Level::Pii,
            ..
        }
    ));
}

#[test]
fn validity_failures_and_reference_cycles_are_closed_and_value_free() {
    let mismatch = document(json!({"type": "integer", "x-sagat-classification": "public"}));
    let error = Classifier::default()
        .project(request(
            &mismatch,
            br#""private-canary""#,
            Audience::new(true, false, true),
        ))
        .expect_err("mismatch");
    assert_eq!(error, ProjectionFailure::SchemaMismatch);
    assert!(!format!("{error:?}").contains("private-canary"));

    let cycle = document(json!({
        "$ref": "#/$defs/cycle", "x-sagat-classification": "public",
        "$defs": {"cycle": {"$ref": "#/$defs/cycle"}}
    }));
    assert_eq!(
        Classifier::default().project(request(&cycle, b"null", Audience::new(true, false, true))),
        Err(ProjectionFailure::SchemaUnusable)
    );
}

#[test]
fn cache_is_keyed_by_version_and_closed_carrier() {
    let doc = document(json!({"type": "null", "x-sagat-classification": "public"}));
    let version = WorkflowVersionId::generate();
    let classifier = Classifier::default();
    for _ in 0..2 {
        let mut input = request(&doc, b"null", Audience::new(false, false, false));
        input.cache_key.version = version;
        classifier.project(input).expect("projection");
    }
    let stats = classifier.cache_stats();
    assert_eq!((stats.misses, stats.hits), (1, 1));
}

#[test]
fn exact_number_text_reaches_the_typed_projection() {
    let doc = document(json!({"type": "number", "x-sagat-classification": "public"}));
    let result = Classifier::default()
        .project(request(
            &doc,
            b"-12345678901234567890.001e+42",
            Audience::new(false, false, false),
        ))
        .expect("projection");
    let ClassifiedNode::Value {
        value: ScalarValue::Number(number),
        ..
    } = result.tree
    else {
        panic!("number")
    };
    assert_eq!(number.as_str(), "-12345678901234567890.001e+42");
}

#[test]
fn an_omitted_root_is_not_disclosable_without_synthetic_structure() {
    let doc = document(json!({"type": "string", "x-sagat-classification": "secret"}));
    assert_eq!(
        Classifier::default().project(request(
            &doc,
            br#""canary""#,
            Audience::new(false, false, false),
        )),
        Err(ProjectionFailure::NotDisclosable)
    );
}
