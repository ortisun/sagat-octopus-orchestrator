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

    let ClassifiedNode::Container { ref children, .. } = result.tree else {
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
    let ClassifiedNode::Container { ref children, .. } = result.tree else {
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

#[test]
fn root_and_named_body_nodes_with_the_same_local_id_do_not_share_cache_entries() {
    let doc = serde_json::to_vec(&json!({
        "nodes": [{
            "id": "shared", "type": "activity",
            "outputSchema": {"type": "string", "x-sagat-classification": "public"}
        }],
        "bodies": {
            "first": {"nodes": [{
                "id": "shared", "type": "activity",
                "outputSchema": {"type": "string", "x-sagat-classification": "pii"}
            }]},
            "second": {"nodes": [{
                "id": "shared", "type": "activity",
                "outputSchema": {"type": "string", "x-sagat-classification": "secret"}
            }]}
        }
    }))
    .expect("document");
    let version = WorkflowVersionId::generate();
    let node = sagat_domain::workflow::NodeId::parse("shared").expect("node");
    let classifier = Classifier::default();
    let carriers = [
        (CarrierPointer::NodeOutput(node.clone()), Level::Public),
        (
            CarrierPointer::BodyNodeOutput {
                body: sagat_domain::workflow::NodeId::parse("first").expect("body"),
                node: node.clone(),
            },
            Level::Pii,
        ),
        (
            CarrierPointer::BodyNodeOutput {
                body: sagat_domain::workflow::NodeId::parse("second").expect("body"),
                node,
            },
            Level::Secret,
        ),
    ];
    for (carrier, expected) in carriers {
        let mut input = request(&doc, br#""value""#, Audience::new(true, true, true));
        input.cache_key = ClassificationCacheKey { version, carrier };
        let result = classifier.project(input).expect("projection");
        assert!(matches!(
            result.tree,
            ClassifiedNode::Value { level, .. }
                | ClassifiedNode::Withheld { level, .. } if level == expected
        ));
    }
    assert_eq!(classifier.cache_stats().misses, 3);
}

#[test]
fn callers_without_structure_cannot_distinguish_hidden_child_counts() {
    let object_doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {
            "visible": {"type": "string"},
            "hidden": {"type": "string", "x-sagat-classification": "internal"}
        }
    }));
    let classifier = Classifier::default();
    let visible = classifier
        .project(request(
            &object_doc,
            br#"{"visible":"same"}"#,
            Audience::new(false, false, false),
        ))
        .expect("visible object");
    let hidden = classifier
        .project(request(
            &object_doc,
            br#"{"visible":"same","hidden":"private"}"#,
            Audience::new(false, false, false),
        ))
        .expect("hidden object");
    let counts = |payload: &sagat_application::classification::ClassifiedPayload| {
        let ClassifiedNode::Container {
            child_count,
            omitted_children,
            children,
            ..
        } = &payload.tree
        else {
            panic!("container")
        };
        (
            *child_count,
            *omitted_children,
            children
                .iter()
                .map(|child| (child.ordinal, child.name.clone()))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(counts(&visible), counts(&hidden));

    let array_doc = document(json!({
        "type": "array", "x-sagat-classification": "public",
        "prefixItems": [{"type": "string"}],
        "items": {"type": "string", "x-sagat-classification": "internal"}
    }));
    let visible = classifier
        .project(request(
            &array_doc,
            br#"["same"]"#,
            Audience::new(false, false, false),
        ))
        .expect("visible array");
    let hidden = classifier
        .project(request(
            &array_doc,
            br#"["same","private"]"#,
            Audience::new(false, false, false),
        ))
        .expect("hidden array");
    assert_eq!(counts(&visible), counts(&hidden));
}

fn wire_node(node: &ClassifiedNode) -> Value {
    let kind = |kind| match kind {
        sagat_application::classification::ValueKind::Object => "object",
        sagat_application::classification::ValueKind::Array => "array",
        sagat_application::classification::ValueKind::String => "string",
        sagat_application::classification::ValueKind::Number => "number",
        sagat_application::classification::ValueKind::Boolean => "boolean",
        sagat_application::classification::ValueKind::Null => "null",
    };
    match node {
        ClassifiedNode::Value {
            level,
            kind: value_kind,
            value,
            value_truncated,
            original_length,
        } => json!({
            "kind": "value", "level": level.as_str(), "type": kind(*value_kind),
            "value": match value {
                ScalarValue::String(value) => json!(value),
                ScalarValue::Number(value) => json!(value.as_str()),
                ScalarValue::Boolean(value) => json!(value),
                ScalarValue::Null => Value::Null,
            },
            "valueTruncated": value_truncated, "length": original_length,
        }),
        ClassifiedNode::Container {
            level,
            kind: value_kind,
            child_count,
            omitted_children,
            children,
        } => json!({
            "kind": "container", "level": level.as_str(), "type": kind(*value_kind),
            "childCount": child_count, "omittedChildren": omitted_children,
            "children": children.iter().map(|child| json!({
                "ordinal": child.ordinal, "name": child.name,
                "node": wire_node(&child.node),
            })).collect::<Vec<_>>(),
        }),
        ClassifiedNode::Withheld {
            level,
            kind: value_kind,
            reason,
        } => json!({
            "kind": "withheld", "level": level.as_str(), "type": kind(*value_kind),
            "reason": format!("{reason:?}"),
        }),
    }
}

#[test]
fn escaped_data_keys_and_tree_overhead_stay_inside_the_response_budget() {
    let doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "additionalProperties": {"type": "null", "x-sagat-classification": "public"}
    }));
    let payload = Value::Object(
        (0..128)
            .map(|index| {
                (
                    format!("key-{index:03}{}", "\u{0001}".repeat(311)),
                    Value::Null,
                )
            })
            .collect(),
    );
    let payload = serde_json::to_vec(&payload).expect("payload");
    assert!(payload.len() <= sagat_application::classification::MAX_PAYLOAD_BYTES);
    let result = Classifier::default()
        .project(request(&doc, &payload, Audience::new(false, false, false)))
        .expect("projection");
    let encoded = serde_json::to_vec(&wire_node(&result.tree)).expect("wire tree");
    assert!(encoded.len() <= sagat_application::classification::MAX_RESPONSE_BYTES);
    assert!(result.truncated);
}

#[test]
fn numeric_validation_never_uses_a_rounded_value_as_authority() {
    let classifier = Classifier::default();
    let integer = document(json!({
        "type": "integer", "x-sagat-classification": "public"
    }));
    assert!(
        classifier
            .project(request(
                &integer,
                b"1.0000000000000000000000000001",
                Audience::new(false, false, false),
            ))
            .is_err()
    );
    for exact in [b"9007199254740993".as_slice(), b"1e3".as_slice()] {
        let result = classifier
            .project(request(&integer, exact, Audience::new(false, false, false)))
            .expect("exact integer");
        let ClassifiedNode::Value {
            value: ScalarValue::Number(value),
            ..
        } = result.tree
        else {
            panic!("number")
        };
        assert_eq!(value.as_str().as_bytes(), exact);
    }

    for schema in [
        json!({"type": "number", "maximum": 1, "x-sagat-classification": "public"}),
        json!({"const": 1, "x-sagat-classification": "public"}),
        json!({"const": 1.0000000000000000000000000001_f64, "x-sagat-classification": "public"}),
    ] {
        let doc = document(schema);
        assert!(
            classifier
                .project(request(
                    &doc,
                    b"1.0000000000000000000000000001",
                    Audience::new(false, false, false),
                ))
                .is_err()
        );
    }
}

#[test]
fn fractional_lexemes_fail_closed_when_integer_types_are_indirect() {
    let classifier = Classifier::default();
    let fractional = b"1.0000000000000000000000000001";
    for schema in [
        json!({
            "type": "number", "x-sagat-classification": "public",
            "if": true,
            "then": {"type": "integer"}
        }),
        json!({
            "type": "number", "x-sagat-classification": "public",
            "if": false,
            "else": {"type": "integer"}
        }),
        json!({
            "x-sagat-classification": "public",
            "allOf": [{"type": "number"}, {"anyOf": [{"type": "integer"}, {"type": "string"}]}]
        }),
        json!({
            "type": ["number", "integer"], "x-sagat-classification": "public"
        }),
        json!({
            "type": "number", "x-sagat-classification": "public",
            "not": {"type": "integer"}
        }),
    ] {
        let doc = document(schema);
        assert!(
            classifier
                .project(request(
                    &doc,
                    fractional,
                    Audience::new(false, false, false),
                ))
                .is_err(),
            "an indirect integer restriction must never validate a rounded fraction"
        );
    }

    let overlapping = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"x": {"type": "number"}},
        "patternProperties": {"^x$": {"type": "integer"}},
        "additionalProperties": false
    }));
    assert!(
        classifier
            .project(request(
                &overlapping,
                br#"{"x":1.0000000000000000000000000001}"#,
                Audience::new(false, false, false),
            ))
            .is_err()
    );

    let dependent = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {
            "enabled": {"type": "boolean"},
            "x": {"type": "number"}
        },
        "dependentSchemas": {
            "enabled": {"properties": {"x": {"type": "integer"}}}
        }
    }));
    assert!(
        classifier
            .project(request(
                &dependent,
                br#"{"enabled":true,"x":1.0000000000000000000000000001}"#,
                Audience::new(false, false, false),
            ))
            .is_err()
    );
}

#[test]
fn exact_integer_lexemes_survive_indirect_integer_constraints() {
    let doc = document(json!({
        "type": "number", "x-sagat-classification": "public",
        "if": true,
        "then": {"type": ["integer", "string"]}
    }));
    for exact in [b"9007199254740993".as_slice(), b"1e3".as_slice()] {
        let result = Classifier::default()
            .project(request(&doc, exact, Audience::new(false, false, false)))
            .expect("mathematically exact integer");
        let ClassifiedNode::Value {
            value: ScalarValue::Number(number),
            ..
        } = result.tree
        else {
            panic!("number")
        };
        assert_eq!(number.as_str().as_bytes(), exact);
    }
}

#[test]
fn hidden_object_children_do_not_consume_the_visible_child_limit() {
    let doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"z": {"type": "string"}},
        "additionalProperties": true
    }));
    let classifier = Classifier::default();
    let only_visible = classifier
        .project(request(
            &doc,
            br#"{"z":"shown"}"#,
            Audience::new(false, false, false),
        ))
        .expect("visible projection");
    let mut crowded = serde_json::Map::new();
    for index in 0..128 {
        crowded.insert(format!("a{index:03}"), Value::String("hidden".into()));
    }
    crowded.insert("z".into(), Value::String("shown".into()));
    let crowded = serde_json::to_vec(&Value::Object(crowded)).expect("payload");
    let with_hidden = classifier
        .project(request(&doc, &crowded, Audience::new(false, false, false)))
        .expect("hidden fields do not crowd out the visible field");

    assert_eq!(wire_node(&only_visible.tree), wire_node(&with_hidden.tree));
    assert_eq!(only_visible.truncated, with_hidden.truncated);
    assert!(!with_hidden.truncated);
}

#[test]
fn hidden_children_at_the_depth_boundary_do_not_change_truncation() {
    let mut schema = json!({"additionalProperties": true});
    let mut empty_payload = json!({});
    let mut hidden_payload = json!({"hidden": "private"});
    for _ in 0..16 {
        schema = json!({"additionalProperties": schema});
        empty_payload = json!({"next": empty_payload});
        hidden_payload = json!({"next": hidden_payload});
    }
    schema
        .as_object_mut()
        .expect("root schema")
        .insert("x-sagat-classification".into(), json!("public"));
    let doc = document(schema);
    let empty_payload = serde_json::to_vec(&empty_payload).expect("payload");
    let hidden_payload = serde_json::to_vec(&hidden_payload).expect("payload");
    let classifier = Classifier::default();
    let empty = classifier
        .project(request(
            &doc,
            &empty_payload,
            Audience::new(false, false, false),
        ))
        .expect("empty boundary");
    let hidden = classifier
        .project(request(
            &doc,
            &hidden_payload,
            Audience::new(false, false, false),
        ))
        .expect("hidden boundary");

    assert_eq!(wire_node(&empty.tree), wire_node(&hidden.tree));
    assert_eq!(empty.truncated, hidden.truncated);
    assert!(!hidden.truncated);
}

#[test]
fn hidden_array_items_do_not_leak_positions_or_consume_the_child_limit() {
    let mut prefix = vec![
        json!({
            "type": "string", "x-sagat-classification": "pii"
        });
        128
    ];
    prefix.push(json!({
        "type": "string", "x-sagat-classification": "public"
    }));
    let doc = document(json!({
        "type": "array", "x-sagat-classification": "public",
        "prefixItems": prefix,
        "items": false
    }));
    let mut payload = vec![Value::String("hidden".into()); 128];
    payload.push(Value::String("shown".into()));
    let payload = serde_json::to_vec(&payload).expect("payload");
    let result = Classifier::default()
        .project(request(&doc, &payload, Audience::new(false, false, false)))
        .expect("visible array item");
    let ClassifiedNode::Container {
        child_count,
        omitted_children,
        children,
        ..
    } = result.tree
    else {
        panic!("array")
    };
    assert_eq!(child_count, 1);
    assert_eq!(omitted_children, 0);
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].ordinal, 0);
    assert!(matches!(
        children[0].node.as_ref(),
        ClassifiedNode::Value {
            value: ScalarValue::String(value),
            ..
        } if value == "shown"
    ));
    assert!(!result.truncated);
}

#[test]
fn structure_audience_keeps_real_counts_when_the_child_limit_is_reached() {
    let doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"z": {"type": "string"}},
        "additionalProperties": true
    }));
    let mut payload = serde_json::Map::new();
    for index in 0..128 {
        payload.insert(format!("a{index:03}"), Value::String("hidden".into()));
    }
    payload.insert("z".into(), Value::String("shown".into()));
    let payload = serde_json::to_vec(&Value::Object(payload)).expect("payload");
    let result = Classifier::default()
        .project(request(&doc, &payload, Audience::new(false, false, true)))
        .expect("structural projection");
    let ClassifiedNode::Container {
        child_count,
        omitted_children,
        children,
        ..
    } = result.tree
    else {
        panic!("object")
    };
    assert_eq!(child_count, 129);
    assert_eq!(omitted_children, 1);
    assert_eq!(children.len(), 128);
    assert!(result.truncated);
}

#[test]
fn unsupported_pattern_governance_keeps_unknown_properties_at_least_pii() {
    let doc = document(json!({
        "type": "object", "x-sagat-classification": "public",
        "patternProperties": {"^s": {"type": "string"}},
        "additionalProperties": {"type": "string", "x-sagat-classification": "public"}
    }));
    let result = Classifier::default()
        .project(request(
            &doc,
            br#"{"sensitive":"pii-canary"}"#,
            Audience::new(false, false, true),
        ))
        .expect("projection");
    let ClassifiedNode::Container { ref children, .. } = result.tree else {
        panic!("container")
    };
    assert!(matches!(
        children[0].node.as_ref(),
        ClassifiedNode::Withheld {
            level: Level::Pii,
            ..
        }
    ));
    assert_eq!(children[0].name, None);
    assert!(!format!("{result:?}").contains("pii-canary"));
}

#[test]
fn unique_items_under_boolean_applicators_cannot_compare_rounded_numbers() {
    let classifier = Classifier::default();
    for condition in [
        json!({"not":{"uniqueItems":true}}),
        json!({"if":{"uniqueItems":true},"then":false,"else":true}),
    ] {
        let mut schema =
            json!({"type":"array","x-sagat-classification":"public","items":{"type":"number"}});
        schema
            .as_object_mut()
            .expect("schema")
            .extend(condition.as_object().expect("condition").clone());
        let doc = document(schema);
        assert!(
            classifier
                .project(request(
                    &doc,
                    b"[1.0000000000000000000000000001,1.0]",
                    Audience::new(false, false, false)
                ))
                .is_err(),
            "numeric equality in any subschema must not use rounded values"
        );
        assert!(
            classifier
                .project(request(&doc, b"[1,1]", Audience::new(false, false, false)))
                .is_ok(),
            "exact integer comparisons remain supported"
        );
    }
}

#[test]
fn multiple_of_schema_divisors_must_not_be_rounded_during_compilation() {
    let classifier = Classifier::default();
    let doc = document(
        json!({"type":"integer","x-sagat-classification":"public","multipleOf":9_007_199_254_740_993_u64}),
    );
    assert!(
        classifier
            .project(request(
                &doc,
                b"9007199254740992",
                Audience::new(false, false, false)
            ))
            .is_err(),
        "a rounded schema divisor must never validate a different integer"
    );
    let exact =
        document(json!({"type":"integer","x-sagat-classification":"public","multipleOf":3}));
    assert!(
        classifier
            .project(request(
                &exact,
                b"9007199254740993",
                Audience::new(false, false, false)
            ))
            .is_ok(),
        "a supported small divisor retains exact large integer modulo"
    );
}
