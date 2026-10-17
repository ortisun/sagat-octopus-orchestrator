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
