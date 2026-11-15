//! Publication runs the pre-check of the run-input subset on every `inputSchema`, annotated or not (design 043,
//! T043-13, FR-043-012, D043-051, D043-073): in `enforce` mode a schema outside the subset refuses the document with a
//! typed issue that names the keyword and its pointer; in `report` mode it is a warning with the same fields. The restore
//! of a stored version does not run it, so a version already stored still parses and is refused at start instead.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::{
    InputValidationMode, IrCodec, IrError, problems_of, warning_problem,
};
use sagat_domain::workflow::{DefinitionWarning, InputSchemaIssue};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn document(schema: Option<Value>, triggers: Option<Value>) -> Vec<u8> {
    let mut document = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "orders"}, "entrypoint": "a",
        "nodes": [{"id": "a", "type": "activity", "activity": "orders.take"}]
    });
    if let Some(schema) = schema {
        document["inputSchema"] = schema;
    }
    if let Some(triggers) = triggers {
        document["triggers"] = triggers;
    }
    serde_json::to_vec(&document).expect("json")
}

/// A chain of `links` references: an application path of `links + 1`.
fn chain(links: usize) -> Value {
    let mut defs = serde_json::Map::new();
    defs.insert(format!("d{links}"), json!({}));
    for i in (1..links).rev() {
        defs.insert(
            format!("d{i}"),
            json!({"$ref": format!("#/$defs/d{}", i + 1)}),
        );
    }
    json!({"$defs": defs, "$ref": "#/$defs/d1"})
}

fn outside() -> Value {
    json!({"type": "object", "properties": {"a": {"patternProperties": {"^x": {}}}}})
}

#[test]
fn an_input_schema_outside_the_subset_refuses_the_publication_in_enforce_mode() {
    for parsed in [
        JsonIrCodec.parse(&document(Some(outside()), None)),
        JsonIrCodec.parse_with(
            &document(Some(outside()), None),
            InputValidationMode::Enforce,
        ),
    ] {
        let Err(error) = parsed else {
            panic!("refused in enforce mode")
        };
        assert_eq!(
            error,
            IrError::InputSchema {
                issue: InputSchemaIssue::KeywordUnsupported,
                keyword: "patternProperties".to_owned(),
                path: "/inputSchema/properties/a/patternProperties".to_owned(),
            }
        );
        let problems = problems_of(&error);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].code, "INPUT_SCHEMA_KEYWORD_UNSUPPORTED");
        assert_eq!(
            problems[0].path.as_deref(),
            Some("/inputSchema/properties/a/patternProperties")
        );
        assert!(problems[0].message.contains("patternProperties"));
    }
}

#[test]
fn every_issue_has_its_closed_code_and_an_unannotated_schema_is_checked_too() {
    let cases = [
        (
            json!({"$ref": "https://example.invalid/schema.json"}),
            "INPUT_SCHEMA_REFERENCE_INVALID",
            "/inputSchema/$ref",
        ),
        (
            json!({"properties": {"code": {"pattern": "\\d+"}}}),
            "INPUT_SCHEMA_PATTERN_UNSUPPORTED",
            "/inputSchema/properties/code/pattern",
        ),
        // (a schema past 64 KiB, or nested past 32, is refused first by the classification bound of feature 016,
        // `CLASSIFICATION_TOO_COMPLEX`; the validator's own `INPUT_SCHEMA_TOO_LARGE` is tested in infra-schema)
        (chain(128), "INPUT_SCHEMA_TOO_DEEP", "/inputSchema"),
        (
            json!({"multipleOf": 0.5}),
            "INPUT_SCHEMA_KEYWORD_UNSUPPORTED",
            "/inputSchema/multipleOf",
        ),
    ];
    for (schema, code, path) in cases {
        let error = JsonIrCodec
            .parse(&document(Some(schema.clone()), None))
            .expect_err("refused");
        let problems = problems_of(&error);
        assert_eq!(
            (problems[0].code, problems[0].path.as_deref()),
            (code, Some(path)),
            "{schema}"
        );
    }
    let mut members: Vec<u32> = (0..1_000).collect();
    members.sort_unstable();
    let mut defs = serde_json::Map::new();
    defs.insert("t0".to_owned(), json!({"enum": members}));
    for level in 1..=10 {
        let below = json!({"$ref": format!("#/$defs/t{}", level - 1)});
        defs.insert(
            format!("t{level}"),
            json!({"allOf": [below.clone(), below]}),
        );
    }
    let costly = json!({"$defs": defs, "$ref": "#/$defs/t10"});
    let error = JsonIrCodec
        .parse(&document(Some(costly), None))
        .expect_err("refused");
    assert_eq!(problems_of(&error)[0].code, "INPUT_SCHEMA_TOO_COSTLY");
}

#[test]
fn in_report_mode_the_same_finding_is_a_warning_and_the_document_is_published() {
    let parsed = JsonIrCodec
        .parse_with(
            &document(Some(outside()), None),
            InputValidationMode::Report,
        )
        .expect("published in report mode");
    let warning = DefinitionWarning::InputSchemaOutsideSubset {
        issue: InputSchemaIssue::KeywordUnsupported,
        keyword: "patternProperties".to_owned(),
        at: "/properties/a/patternProperties".to_owned(),
    };
    assert_eq!(parsed.warnings, vec![warning.clone()]);
    let problem = warning_problem(&warning);
    assert_eq!(problem.code, "INPUT_SCHEMA_KEYWORD_UNSUPPORTED");
    assert_eq!(
        problem.path.as_deref(),
        Some("/inputSchema/properties/a/patternProperties")
    );
    assert!(
        parsed.definition.input_schema().is_some(),
        "the schema is still carried"
    );
}

#[test]
fn the_restore_of_a_stored_version_does_not_run_the_pre_check() {
    let parsed = JsonIrCodec
        .parse_stored(&document(Some(outside()), None))
        .expect("a stored version still parses, and is refused at start instead");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
}

#[test]
fn a_schema_inside_the_subset_publishes_and_a_trigger_beside_it_is_warned_about() {
    let inside = json!({"type": "object", "required": ["orderId"], "properties": {"orderId": {"type": "string"}}});
    let parsed = JsonIrCodec
        .parse(&document(Some(inside.clone()), None))
        .expect("inside the subset");
    assert!(parsed.warnings.is_empty());
    let manual = JsonIrCodec
        .parse(&document(
            Some(inside.clone()),
            Some(json!([{"kind": "manual"}])),
        ))
        .expect("manual");
    assert!(
        manual.warnings.is_empty(),
        "a manual trigger is a start like any other"
    );
    let triggered = JsonIrCodec
        .parse(&document(
            Some(inside),
            Some(json!([{"kind": "event", "eventType": "order.created"}])),
        ))
        .expect("event");
    assert_eq!(
        triggered.warnings,
        vec![DefinitionWarning::InputSchemaAppliesToTriggers]
    );
    assert_eq!(
        warning_problem(&triggered.warnings[0]).code,
        "INPUT_SCHEMA_APPLIES_TO_TRIGGERS"
    );
    let unschematic = JsonIrCodec
        .parse(&document(
            None,
            Some(json!([{"kind": "event", "eventType": "order.created"}])),
        ))
        .expect("event without a schema");
    assert!(unschematic.warnings.is_empty());
}
