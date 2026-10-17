#![allow(clippy::expect_used)]

#[path = "../src/classification_check.rs"]
mod classification_check;

use sagat_application::classification::{ClassificationFindingCode as Code, FindingSeverity};
use serde_json::json;

fn fixture(name: &str) -> serde_json::Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/payload-classification/fixtures")
        .join(name);
    serde_json::from_slice(&std::fs::read(root).expect("fixture bytes")).expect("fixture JSON")
}

#[test]
fn publication_findings_are_closed_bounded_and_point_to_the_defect() {
    let document = json!({
        "inputSchema": {
            "type": "object",
            "properties": {
                "open": {"type": "string", "x-sagat-classification": "public"},
                "bad": {"type": "string", "x-sagat-classification": "restricted"}
            }
        },
        "metadata": {"x-sagat-classification": "secret"}
    });
    let result = classification_check::check(&document);
    let found: Vec<_> = result
        .findings
        .iter()
        .map(|f| (f.code, f.path.as_str()))
        .collect();
    assert!(found.contains(&(Code::Loosened, "/inputSchema/properties/open")));
    assert!(found.contains(&(
        Code::InvalidLevel,
        "/inputSchema/properties/bad/x-sagat-classification"
    )));
    assert!(found.contains(&(Code::NotApplicable, "/metadata/x-sagat-classification")));
    assert!(result.findings.len() <= 20);
    assert!(
        result
            .findings
            .iter()
            .all(|finding| !finding.code.message().contains("restricted"))
    );
}

#[test]
fn report_lists_effective_declared_paths_and_open_inheritance_warning() {
    let document = json!({
        "inputSchema": {
            "type": "object", "x-sagat-classification": "public",
            "properties": {
                "id": {"type": "string"},
                "private": {"type": "string", "x-sagat-classification": "internal"}
            }
        }
    });
    let result = classification_check::check(&document);
    assert!(
        result
            .report
            .iter()
            .any(|entry| entry.path.as_str() == "/id"
                && entry.level == sagat_domain::classification::Level::Public
                && !entry.declared)
    );
    assert!(
        result
            .report
            .iter()
            .any(|entry| entry.path.as_str() == "/private"
                && entry.level == sagat_domain::classification::Level::Internal
                && entry.declared)
    );
    assert!(
        result
            .findings
            .iter()
            .any(|finding| finding.code == Code::InheritedOpen
                && finding.code.severity() == FindingSeverity::Warning)
    );
}

#[test]
fn an_unannotated_document_reports_pii_roots_without_publication_errors() {
    let result = classification_check::check(&json!({
        "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}}
    }));
    assert!(result.is_publishable());
    assert!(
        result
            .report
            .iter()
            .any(|entry| entry.path.as_str().is_empty()
                && entry.level == sagat_domain::classification::Level::Pii)
    );
}

#[test]
fn defect_corpus_covers_every_closed_publication_code() {
    let cases = [
        ("defects/invalid-level.json", Code::InvalidLevel),
        ("defects/loosened.json", Code::Loosened),
        ("defects/loosened-ref.json", Code::Loosened),
        ("defects/unmarked-root-loosened.json", Code::Loosened),
        ("defects/unsupported-keyword.json", Code::UnsupportedKeyword),
        ("defects/unresolved-ref.json", Code::UnresolvedRef),
        ("defects/not-applicable.json", Code::NotApplicable),
        ("defects/too-complex.json", Code::TooComplex),
        ("defects/inherited-open.json", Code::InheritedOpen),
    ];
    for (name, code) in cases {
        let result = classification_check::check(&fixture(name));
        assert!(
            result.findings.iter().any(|finding| finding.code == code),
            "{name} did not produce {}",
            code.as_str()
        );
    }
}

#[test]
fn named_body_output_schemas_are_carriers_only_on_output_producers() {
    use sagat_application::classification::CarrierPointer;
    use sagat_domain::classification::Level;

    let document = json!({
        "nodes": [{"id": "shared", "type": "activity", "outputSchema": {
            "type": "string", "x-sagat-classification": "public"
        }}],
        "bodies": {
            "first": {"nodes": [
                {"id": "shared", "type": "activity", "outputSchema": {
                    "type": "string", "x-sagat-classification": "pii"
                }},
                {"id": "script", "type": "script", "outputSchema": {
                    "type": "string", "x-sagat-classification": "internal"
                }},
                {"id": "wait", "type": "waitEvent", "outputSchema": {
                    "type": "string", "x-sagat-classification": "pii"
                }},
                {"id": "child", "type": "child", "outputSchema": {
                    "type": "string", "x-sagat-classification": "secret"
                }},
                {"id": "timer", "type": "timer", "outputSchema": {
                    "x-sagat-classification": "public"
                }}
            ]},
            "second": {"nodes": [{"id": "shared", "type": "activity", "outputSchema": {
                "type": "string", "x-sagat-classification": "secret"
            }}]}
        },
        "state": {"schema": {"x-sagat-classification": "public"}},
        "database": {"outputSchema": {"x-sagat-classification": "public"}},
        "control": {"outputSchema": {"x-sagat-classification": "public"}},
        "humanApproval": {"formSchema": {"x-sagat-classification": "public"}}
    });
    let result = classification_check::check(&document);
    assert!(result.report.iter().any(|entry| matches!(
        &entry.carrier,
        CarrierPointer::NodeOutput(node)
            if node.as_str() == "shared" && entry.level == Level::Public
    )));
    assert!(result.report.iter().any(|entry| matches!(
        &entry.carrier,
        CarrierPointer::BodyNodeOutput { body, node }
            if body.as_str() == "first" && node.as_str() == "shared" && entry.level == Level::Pii
    )));
    assert!(result.report.iter().any(|entry| matches!(
        &entry.carrier,
        CarrierPointer::BodyNodeOutput { body, node }
            if body.as_str() == "second" && node.as_str() == "shared" && entry.level == Level::Secret
    )));
    for path in [
        "/bodies/first/nodes/4/outputSchema/x-sagat-classification",
        "/state/schema/x-sagat-classification",
        "/database/outputSchema/x-sagat-classification",
        "/control/outputSchema/x-sagat-classification",
        "/humanApproval/formSchema/x-sagat-classification",
    ] {
        assert!(
            result.findings.iter().any(|finding| {
                finding.code == Code::NotApplicable && finding.path.as_str() == path
            }),
            "missing NOT_APPLICABLE at {path}"
        );
    }
}

#[test]
fn named_body_carriers_use_the_same_size_depth_and_reference_bounds() {
    let mut deep = json!({"type": "string", "x-sagat-classification": "pii"});
    for _ in 0..33 {
        deep = json!({"type": "object", "properties": {"next": deep}});
    }
    let cases = [
        json!({
            "x-sagat-classification": "pii",
            "description": "x".repeat(65 * 1024)
        }),
        deep,
        json!({
            "x-sagat-classification": "pii",
            "$ref": "#/$defs/cycle",
            "$defs": {"cycle": {"$ref": "#/$defs/cycle"}}
        }),
    ];
    for schema in cases {
        let result = classification_check::check(&json!({
            "bodies": {"loop": {"nodes": [{
                "id": "same", "type": "activity", "outputSchema": schema
            }]}}
        }));
        assert!(
            result
                .findings
                .iter()
                .any(|finding| finding.code == Code::TooComplex)
        );
        assert!(result.findings.len() <= 20);
    }
}

#[test]
fn items_and_additional_properties_participate_in_monotonic_publication_walks() {
    let result = classification_check::check(&json!({
        "inputSchema": {
            "type": "object", "x-sagat-classification": "internal",
            "additionalProperties": {"x-sagat-classification": "public"}
        },
        "outputSchema": {
            "type": "array", "x-sagat-classification": "pii",
            "items": {"x-sagat-classification": "internal"}
        }
    }));
    for path in ["/inputSchema/additionalProperties", "/outputSchema/items"] {
        assert!(
            result
                .findings
                .iter()
                .any(|finding| { finding.code == Code::Loosened && finding.path.as_str() == path }),
            "missing loosened finding at {path}"
        );
    }
    assert!(result.report.iter().any(|entry| {
        entry.carrier == sagat_application::classification::CarrierPointer::Input
            && entry.path.as_str() == "/*"
            && entry.level == sagat_domain::classification::Level::Internal
    }));
    assert!(result.report.iter().any(|entry| {
        entry.carrier == sagat_application::classification::CarrierPointer::Result
            && entry.path.as_str() == "/*"
            && entry.level == sagat_domain::classification::Level::Pii
    }));
}

#[test]
fn warning_caps_and_unrepresentable_paths_cannot_hide_publication_errors() {
    let mut properties = serde_json::Map::new();
    for index in 0..21 {
        properties.insert(format!("a{index:02}"), json!({"type": "string"}));
    }
    properties.insert(
        "z-invalid".into(),
        json!({"x-sagat-classification": "restricted"}),
    );
    let result = classification_check::check(&json!({
        "inputSchema": {
            "type": "object", "x-sagat-classification": "public",
            "properties": properties,
            "patternProperties": {".*": {"x-sagat-classification": "pii"}}
        }
    }));
    assert!(!result.is_publishable());
    assert!(result.findings.len() <= 20);
    assert!(
        result
            .findings
            .iter()
            .any(|finding| finding.code == Code::InvalidLevel)
    );
    assert!(
        result
            .findings
            .iter()
            .any(|finding| finding.code == Code::UnsupportedKeyword)
    );

    let hostile = format!("{}\u{0001}tail", "x".repeat(2_100));
    let mut hostile_properties = serde_json::Map::new();
    hostile_properties.insert(hostile, json!({"x-sagat-classification": "restricted"}));
    let result = classification_check::check(&json!({
        "inputSchema": {
            "type": "object",
            "properties": hostile_properties
        }
    }));
    assert!(!result.is_publishable());
    assert!(
        result
            .findings
            .iter()
            .any(|finding| finding.code == Code::InvalidLevel)
    );
    assert!(result.findings.iter().all(|finding| {
        finding.path.as_str().len() <= 2_048 && !finding.path.as_str().chars().any(char::is_control)
    }));
}

#[test]
fn all_of_fallbacks_govern_properties_and_prefix_items_from_other_branches() {
    use sagat_domain::classification::Level;

    let result = classification_check::check(&json!({
        "inputSchema": {"allOf": [
            {"x-sagat-classification":"public", "properties":{"named": {"type":"string"}},
             "prefixItems":[{"type":"string"}]},
            {"additionalProperties":{"x-sagat-classification":"pii"},
             "items":{"x-sagat-classification":"secret"}}
        ]}
    }));
    assert!(
        result
            .report
            .iter()
            .any(|entry| { entry.path.as_str() == "/named" && entry.level == Level::Pii })
    );
    assert!(
        result
            .report
            .iter()
            .any(|entry| { entry.path.as_str() == "/0" && entry.level == Level::Secret })
    );
}

#[test]
fn location_visitor_is_not_limited_by_the_human_report_cap() {
    let properties = (0..550)
        .map(|index| (format!("p{index:03}"), json!({"type":"string"})))
        .collect::<serde_json::Map<_, _>>();
    let document = json!({"inputSchema": {
        "x-sagat-classification":"pii", "properties": properties
    }});
    let mut visited = 0;
    let status = classification_check::visit_locations(&document, |_| visited += 1);

    assert_eq!(visited, 551);
    assert!(status.carriers.iter().all(|status| matches!(
        status,
        sagat_application::classification::ClassificationCarrierStatus::Complete(_)
    )));
}

#[test]
fn failed_carrier_emits_no_partial_locations_and_returns_a_safe_status() {
    let document = json!({"inputSchema": {
        "$ref":"https://outside.invalid/schema", "x-sagat-classification":"pii"
    }});
    let mut visited = 0;
    let status = classification_check::visit_locations(&document, |_| visited += 1);

    assert_eq!(visited, 0);
    assert!(matches!(
        status.carriers.as_slice(),
        [sagat_application::classification::ClassificationCarrierStatus::Failed(failure)]
            if failure.code == Code::UnresolvedRef
                && failure.source_path == "/inputSchema/$ref"
    ));
}

#[test]
fn annotations_reached_by_refs_under_unsupported_applicators_are_refused() {
    for (unsupported, expected_path) in [
        (json!({"then":{"$ref":"#/$defs/s"}}), "/inputSchema/then"),
        (
            json!({"patternProperties":{"^private$":{"$ref":"#/$defs/s"}}}),
            "/inputSchema/patternProperties",
        ),
        (
            json!({"contains":{"$ref":"#/$defs/s"}}),
            "/inputSchema/contains",
        ),
    ] {
        let mut schema = json!({
            "x-sagat-classification":"public",
            "$defs":{"s":{"type":"string", "x-sagat-classification":"secret"}},
            "if": true
        });
        schema
            .as_object_mut()
            .expect("schema object")
            .extend(unsupported.as_object().expect("applicator object").clone());
        let result = classification_check::check(&json!({"inputSchema":schema}));
        assert!(result.findings.iter().any(|finding| {
            finding.code == Code::UnsupportedKeyword && finding.path.as_str() == expected_path
        }));
        assert!(!result.is_publishable());
    }
}

#[test]
fn unsupported_applicator_annotation_reachability_follows_two_local_refs() {
    let result = classification_check::check(&json!({"inputSchema":{
        "x-sagat-classification":"public",
        "then":{"$ref":"#/$defs/first"},
        "$defs":{
            "first":{"$ref":"#/$defs/second"},
            "second":{"x-sagat-classification":"secret"}
        }
    }}));
    assert!(result.findings.iter().any(|finding| {
        finding.code == Code::UnsupportedKeyword && finding.path.as_str() == "/inputSchema/then"
    }));
}

#[test]
fn supported_all_of_refs_remain_valid_controls() {
    let result = classification_check::check(&json!({"inputSchema":{
        "x-sagat-classification":"public",
        "allOf":[{"$ref":"#/$defs/s"}],
        "$defs":{"s":{"x-sagat-classification":"secret"}}
    }}));
    assert!(result.is_publishable(), "{:?}", result.findings);
    assert!(
        !result
            .findings
            .iter()
            .any(|finding| finding.code == Code::UnsupportedKeyword)
    );
}
