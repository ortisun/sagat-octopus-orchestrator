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
