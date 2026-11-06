//! The level of each field an `icebergWrite` step writes, read from the schemas of its source (feature 035: T035-054;
//! FR-035-025, D035-051).
//!
//! The codec is the only place that holds the whole document, so it is the one that classifies: a field inherits the
//! level the schema of its source gives it (016), and anything it cannot prove, it reads as `pii`. An expression that is
//! not a plain reference to the workflow input or to the output of a step carries no schema the platform can follow, and
//! database results and blobs carry none either.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::definitions::IrCodec;
use sagat_domain::{
    classification::Level,
    database::{EgressField, IcebergWrite},
    workflow::NodeId,
};
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

fn rows_schema() -> Value {
    json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"rows": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "total": {"type": "integer", "x-sagat-classification": "internal"},
                "email": {"type": "string", "x-sagat-classification": "pii"},
                "token": {"type": "string", "x-sagat-classification": "secret"},
                "customer": {"type": "object", "properties": {
                    "name": {"type": "string", "x-sagat-classification": "internal"},
                    "ssn": {"type": "string", "x-sagat-classification": "pii"}
                }},
                "a/b": {"type": "string", "x-sagat-classification": "internal"}
            }
        }}}
    })
}

fn step(records: Value, mapping: &[&str]) -> Value {
    let mapping: Vec<Value> = mapping
        .iter()
        .enumerate()
        .map(|(n, field)| json!({"field": field, "column": format!("c{n}")}))
        .collect();
    let mut node = json!({"id": "save", "type": "icebergWrite", "connectionRef": {"name": "lake-main"},
        "table": {"namespace": ["sales"], "name": "orders"}, "mode": "append",
        "records": records, "next": "done"});
    if !mapping.is_empty() {
        node["mapping"] = Value::Array(mapping);
    }
    node
}

fn document(input_schema: Option<Value>, nodes: Vec<Value>) -> Vec<u8> {
    let mut document = json!({"apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "lake-flow"}, "entrypoint": nodes[0]["id"].clone(), "nodes": nodes});
    if let Some(schema) = input_schema {
        document["inputSchema"] = schema;
    }
    document.to_string().into_bytes()
}

fn flow(input_schema: Option<Value>, save: Value) -> Vec<u8> {
    document(
        input_schema,
        vec![save, json!({"id": "done", "type": "end"})],
    )
}

fn levels_of(bytes: &[u8]) -> Vec<(String, Level)> {
    let parsed = JsonIrCodec.parse(bytes).expect("a published document");
    let node = parsed
        .definition
        .node(&NodeId::parse("save").expect("id"))
        .expect("the step");
    let write: &IcebergWrite = node.iceberg_write().expect("an Iceberg write");
    write
        .egress()
        .expect("the codec classifies every step it builds")
        .iter()
        .map(|field: &EgressField| (field.path().to_owned(), field.level()))
        .collect()
}

fn mapped(fields: &[&str], schema: Option<Value>) -> Vec<(String, Level)> {
    levels_of(&flow(schema, step(json!({"inline": "input.rows"}), fields)))
}

fn levels(pairs: &[(&str, Level)]) -> Vec<(String, Level)> {
    pairs
        .iter()
        .map(|(path, level)| ((*path).to_owned(), *level))
        .collect()
}

#[test]
fn each_mapped_field_takes_the_level_the_schema_of_its_source_gives_it() {
    assert_eq!(
        mapped(&["/id", "/total", "/email", "/token"], Some(rows_schema())),
        levels(&[
            ("/id", Level::Public),
            ("/total", Level::Internal),
            ("/email", Level::Pii),
            ("/token", Level::Secret),
        ])
    );
}

#[test]
fn a_field_that_holds_a_structure_is_as_strict_as_the_strictest_thing_inside_it() {
    assert_eq!(
        mapped(&["/customer", "/customer/name"], Some(rows_schema())),
        levels(&[
            ("/customer", Level::Pii),
            ("/customer/name", Level::Internal)
        ])
    );
}

#[test]
fn a_pointer_decodes_its_escapes_before_it_looks_for_the_property() {
    assert_eq!(
        mapped(&["/a~1b"], Some(rows_schema())),
        levels(&[("/a~1b", Level::Internal)])
    );
}

#[test]
fn a_field_the_schema_does_not_name_is_pii() {
    assert_eq!(
        mapped(&["/unknown"], Some(rows_schema())),
        levels(&[("/unknown", Level::Pii)])
    );
}

#[test]
fn a_step_that_lists_no_columns_writes_the_whole_record_at_its_strictest_level() {
    assert_eq!(
        mapped(&[], Some(rows_schema())),
        levels(&[("", Level::Secret)])
    );
}

#[test]
fn a_source_with_no_classification_is_pii_whatever_the_field() {
    let unmarked = json!({"type": "object", "properties": {"rows": {"type": "array", "items": {
        "type": "object", "properties": {"id": {"type": "string"}}}}}});
    assert_eq!(
        mapped(&["/id"], Some(unmarked)),
        levels(&[("/id", Level::Pii)])
    );
}

#[test]
fn a_workflow_with_no_input_schema_gives_its_fields_no_level_but_pii() {
    assert_eq!(mapped(&["/id"], None), levels(&[("/id", Level::Pii)]));
}

#[test]
fn a_list_whose_items_the_schema_does_not_describe_is_pii() {
    let bare = json!({"type": "object", "x-sagat-classification": "public",
        "properties": {"rows": {"type": "array"}}});
    assert_eq!(mapped(&["/id"], Some(bare)), levels(&[("/id", Level::Pii)]));
}

#[test]
fn a_position_the_schema_marks_stricter_than_its_items_raises_the_level() {
    // The first record of the list is described apart from the rest: a field is as strict as the strictest item.
    let tuple = json!({"type": "object", "x-sagat-classification": "public", "properties": {"rows": {
        "type": "array",
        "prefixItems": [{"type": "object", "properties": {"id": {"type": "string", "x-sagat-classification": "pii"}}}],
        "items": {"type": "object", "properties": {"id": {"type": "string"}}}}}});
    assert_eq!(
        mapped(&["/id"], Some(tuple)),
        levels(&[("/id", Level::Pii)])
    );
}

#[test]
fn the_strictest_item_wins_whichever_the_schema_lists_first() {
    // The same list as above with the order of strictness turned round: the first record is public and the rest are pii.
    // A field is as strict as any record it can come from, so listing the strict description second or first changes
    // nothing.
    let tuple = json!({"type": "object", "x-sagat-classification": "public", "properties": {"rows": {
        "type": "array",
        "prefixItems": [{"type": "object", "properties": {"id": {"type": "string"}}}],
        "items": {"type": "object", "properties": {"id": {"type": "string", "x-sagat-classification": "pii"}}}}}});
    assert_eq!(
        mapped(&["/id"], Some(tuple)),
        levels(&[("/id", Level::Pii)])
    );
}

#[test]
fn an_expression_that_is_not_a_plain_reference_is_pii_even_over_a_public_schema() {
    let schema = || {
        json!({"type": "object", "x-sagat-classification": "public",
            "properties": {"rows": {"type": "array", "items": {"type": "object",
                "properties": {"id": {"type": "string"}}}}}})
    };
    // Forms the expression subset accepts as a record source and that are not a plain reference. The ones it refuses
    // (a trailing dot, an empty segment, a numeric segment, a call) can never be published, so they need no level.
    for expression in [
        "input.rows[0]",
        "input.rows + []",
        "input.rows == input.rows",
        "input",
        "steps.fetch",
        "other.rows",
    ] {
        let bytes = flow(
            Some(schema()),
            step(json!({"inline": expression}), &["/id"]),
        );
        assert_eq!(
            levels_of(&bytes),
            levels(&[("/id", Level::Pii)]),
            "`{expression}` is not a plain reference, so its fields are read as pii"
        );
    }
}

#[test]
fn the_output_of_an_earlier_step_is_read_from_the_schema_that_step_declares() {
    let fetch = json!({"id": "fetch", "type": "activity", "activity": "act.fetch", "next": "save",
        "outputSchema": {"type": "object", "x-sagat-classification": "public", "properties": {
            "rows": {"type": "array", "items": {"type": "object", "properties": {
                "id": {"type": "string"},
                "email": {"type": "string", "x-sagat-classification": "pii"}}}}}}});
    let save = step(
        json!({"inline": "steps.fetch.output.rows"}),
        &["/id", "/email"],
    );
    let bytes = document(
        None,
        vec![fetch, save, json!({"id": "done", "type": "end"})],
    );
    assert_eq!(
        levels_of(&bytes),
        levels(&[("/id", Level::Public), ("/email", Level::Pii)])
    );
}

#[test]
fn only_the_output_of_a_step_is_followed_not_whatever_else_hangs_off_it() {
    let fetch = json!({"id": "fetch", "type": "activity", "activity": "act.fetch", "next": "save",
        "outputSchema": {"type": "object", "x-sagat-classification": "public", "properties": {
            "rows": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}}}}}}});
    let save = step(json!({"inline": "steps.fetch.status.rows"}), &["/id"]);
    let bytes = document(
        None,
        vec![fetch, save, json!({"id": "done", "type": "end"})],
    );
    assert_eq!(levels_of(&bytes), levels(&[("/id", Level::Pii)]));
}

#[test]
fn a_name_that_is_not_plain_is_not_the_property_of_that_name() {
    // `input.rows[0]` indexes a list; it does not read a property whose name happens to be spelt `rows[0]`.
    let schema = json!({"type": "object", "x-sagat-classification": "public", "properties": {
        "rows[0]": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}}}}}});
    let bytes = flow(
        Some(schema),
        step(json!({"inline": "input.rows[0]"}), &["/id"]),
    );
    assert_eq!(levels_of(&bytes), levels(&[("/id", Level::Pii)]));
}

#[test]
fn a_stricter_field_in_another_schema_of_the_document_does_not_raise_this_one() {
    // The step reads the workflow input. An earlier step declares a secret at the same relative place in its own output;
    // that is another carrier, and it says nothing about the data this step writes.
    let fetch = json!({"id": "fetch", "type": "activity", "activity": "act.fetch", "next": "save",
        "outputSchema": {"type": "object", "x-sagat-classification": "public", "properties": {
            "rows": {"type": "array", "items": {"type": "object", "properties": {
                "id": {"type": "string", "x-sagat-classification": "secret"}}}}}}});
    let save = step(json!({"inline": "input.rows"}), &["/id"]);
    let bytes = document(
        Some(
            json!({"type": "object", "x-sagat-classification": "public", "properties": {
            "rows": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}}}}}}),
        ),
        vec![fetch, save, json!({"id": "done", "type": "end"})],
    );
    assert_eq!(levels_of(&bytes), levels(&[("/id", Level::Public)]));
}

#[test]
fn the_output_of_a_step_with_no_declared_schema_is_pii() {
    let fetch = json!({"id": "fetch", "type": "activity", "activity": "act.fetch", "next": "save"});
    let save = step(json!({"inline": "steps.fetch.output.rows"}), &["/id"]);
    let bytes = document(
        None,
        vec![fetch, save, json!({"id": "done", "type": "end"})],
    );
    assert_eq!(levels_of(&bytes), levels(&[("/id", Level::Pii)]));
}

#[test]
fn a_reference_to_a_step_that_does_not_exist_is_pii() {
    let save = step(json!({"inline": "steps.ghost.output.rows"}), &["/id"]);
    let bytes = flow(Some(rows_schema()), save);
    assert_eq!(levels_of(&bytes), levels(&[("/id", Level::Pii)]));
}

#[test]
fn blobs_carry_no_schema_so_their_fields_are_pii() {
    let bytes = flow(
        Some(rows_schema()),
        step(json!({"blobs": "input.rows"}), &["/id", "/total"]),
    );
    assert_eq!(
        levels_of(&bytes),
        levels(&[("/id", Level::Pii), ("/total", Level::Pii)])
    );
}

#[test]
fn a_database_result_is_pii_whatever_a_schema_says() {
    let query = json!({"id": "query", "type": "database", "connectionRef": {"name": "orders-db"},
        "operation": "query", "statement": "SELECT 1", "next": "save"});
    let save = step(json!({"databaseResult": "query"}), &["/id", "/total"]);
    let bytes = document(
        Some(rows_schema()),
        vec![query, save, json!({"id": "done", "type": "end"})],
    );
    assert_eq!(
        levels_of(&bytes),
        levels(&[("/id", Level::Pii), ("/total", Level::Pii)])
    );
}

#[test]
fn a_version_read_back_from_the_store_is_classified_the_same_way() {
    // Admission restores the pinned version; it must meet the same levels as publication did.
    let bytes = flow(
        Some(rows_schema()),
        step(json!({"inline": "input.rows"}), &["/id", "/email"]),
    );
    let published = JsonIrCodec.parse(&bytes).expect("published");
    let restored = JsonIrCodec
        .parse_stored(&published.canonical)
        .expect("restored");
    let at = |definition: &sagat_domain::workflow::WorkflowDefinition| {
        definition
            .node(&NodeId::parse("save").expect("id"))
            .and_then(|node| node.iceberg_write())
            .and_then(IcebergWrite::egress)
            .map(<[EgressField]>::to_vec)
    };
    assert_eq!(at(&published.definition), at(&restored.definition));
    assert!(at(&restored.definition).is_some());
}

#[test]
fn the_levels_are_not_part_of_the_canonical_document_or_its_digest() {
    // Two documents that differ only in a schema annotation the step reads have different bytes, as before; and the
    // derived levels never leak into the bytes, so a version published before this change keeps its digest.
    let with_levels = JsonIrCodec
        .parse(&flow(
            Some(rows_schema()),
            step(json!({"inline": "input.rows"}), &["/id"]),
        ))
        .expect("parsed");
    let text = String::from_utf8(with_levels.canonical).expect("utf8");
    assert!(!text.contains("egress"), "{text}");
    assert!(!text.contains("level"), "{text}");
}
