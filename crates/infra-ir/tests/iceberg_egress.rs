//! The level of each field an `icebergWrite` step writes, read from the schemas of its source (feature 035: T035-054;
//! FR-035-025, D035-051).
//!
//! The codec is the only place that holds the whole document, so it is the one that classifies: a field inherits the
//! level the schema of its source gives it (016), and anything it cannot prove, it reads as `pii`. An expression that is
//! not a plain reference to the workflow input or to the output of a step carries no schema the platform can follow, and
//! database results and blobs carry none either.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::{
    classification::ClassificationFindingCode,
    definitions::{IrCodec, IrError},
};
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

/// A SQL step that declares the classification of its output.
fn query_declaring(schema: Value) -> Value {
    json!({"id": "query", "type": "database", "connectionRef": {"name": "orders-db"},
        "operation": "query", "statement": "SELECT 1", "outputSchema": schema, "next": "save"})
}

/// What makes `a_database_result_is_pii_whatever_a_schema_says` true: the output of a SQL step is not a place a schema may classify (016
/// takes an output schema from an activity, a script, a wait and a child, and refuses an annotation anywhere else). Were it one, a
/// `secret` declared there would have to be as strict for the result source as it is for the inline source of the same write, and this
/// module would have to read it (D035-145, the security review of the result source).
#[test]
fn the_output_of_a_sql_step_cannot_declare_a_classification_so_there_is_none_to_lose() {
    let schema = json!({
        "type": "object", "x-sagat-classification": "public",
        "properties": {"password_hash": {"type": "string", "x-sagat-classification": "secret"}}
    });
    let bytes = document(
        Some(rows_schema()),
        vec![
            query_declaring(schema),
            step(json!({"databaseResult": "query"}), &["/id"]),
            json!({"id": "done", "type": "end"}),
        ],
    );
    let Err(IrError::Classification(findings)) = JsonIrCodec.parse(&bytes) else {
        panic!("a classification on the output of a SQL step is refused at publication");
    };
    let refused = |code, path: &str| {
        findings
            .iter()
            .any(|finding| finding.code == code && format!("{:?}", finding.path).contains(path))
    };
    assert!(
        refused(
            ClassificationFindingCode::NotApplicable,
            "/nodes/0/outputSchema/properties/password_hash/x-sagat-classification"
        ),
        "{findings:?}"
    );
    assert!(
        refused(
            ClassificationFindingCode::NotApplicable,
            "/nodes/0/outputSchema/x-sagat-classification"
        ),
        "{findings:?}"
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
    // The levels are derived. The canonical bytes are the document and nothing more, so a version published before the
    // levels existed keeps its digest, and a restore that computes them again changes nothing.
    let bytes = flow(
        Some(rows_schema()),
        step(json!({"inline": "input.rows"}), &["/id", "/email"]),
    );
    let original: Value = serde_json::from_slice(&bytes).expect("json");
    let parsed = JsonIrCodec.parse(&bytes).expect("parsed");
    let canonical: Value = serde_json::from_slice(&parsed.canonical).expect("canonical json");
    assert_eq!(
        canonical, original,
        "the canonical document is the document"
    );
    let restored = JsonIrCodec
        .parse_stored(&parsed.canonical)
        .expect("restored");
    assert_eq!(restored.canonical, parsed.canonical);
    assert_eq!(restored.digest, parsed.digest);
}

#[test]
fn the_input_itself_is_the_list_when_the_schema_says_so() {
    let schema = json!({"type": "array", "x-sagat-classification": "public",
        "items": {"type": "object", "properties": {"id": {"type": "string"}}}});
    let bytes = flow(Some(schema), step(json!({"inline": "input"}), &["/id"]));
    assert_eq!(levels_of(&bytes), levels(&[("/id", Level::Public)]));
}

/// The item schema of the reviews' scenarios, in a workflow that maps the given fields.
fn items_of(item: Value, fields: &[&str]) -> Vec<(String, Level)> {
    let schema = json!({"type": "object", "x-sagat-classification": "public",
        "properties": {"rows": {"type": "array", "items": item}}});
    mapped(fields, Some(schema))
}

#[test]
fn a_numeric_segment_reads_everything_under_the_list_it_indexes() {
    // `/tokens/0` may be the first element of the list or a property named `0`: either way it is as strict as anything
    // that list can hold. Here the elements are secret while the list itself is only pii.
    let item = json!({"type": "object", "properties": {
        "tokens": {"type": "array", "x-sagat-classification": "pii",
            "items": {"type": "string", "x-sagat-classification": "secret"}}}});
    assert_eq!(
        items_of(item, &["/tokens/0"]),
        levels(&[("/tokens/0", Level::Secret)])
    );
}

#[test]
fn a_tuple_record_is_read_at_the_strictest_of_its_positions() {
    let item = json!({"type": "array", "prefixItems": [
        {"type": "string", "x-sagat-classification": "pii"},
        {"type": "string", "x-sagat-classification": "secret"}], "items": {"type": "string"}});
    assert_eq!(
        items_of(item, &["/0", "/1"]),
        levels(&[("/0", Level::Secret), ("/1", Level::Secret)])
    );
}

#[test]
fn a_property_the_schema_leaves_to_additional_properties_is_as_strict_as_what_that_schema_holds() {
    // `customer` is not named: `additionalProperties` governs it, and that schema has a pii child.
    let item = json!({"type": "object", "additionalProperties": {
        "type": "object", "x-sagat-classification": "internal",
        "properties": {"ssn": {"type": "string", "x-sagat-classification": "pii"}}}});
    assert_eq!(
        items_of(item, &["/customer"]),
        levels(&[("/customer", Level::Pii)])
    );
}

#[test]
fn a_whole_record_over_an_open_object_counts_the_keys_the_schema_does_not_name_as_pii() {
    // Nothing closes the object, so a record may carry a key nobody classified; 016 reads such a key as pii.
    let item = json!({"type": "object", "properties": {
        "id": {"type": "string"},
        "total": {"type": "integer", "x-sagat-classification": "internal"}}});
    assert_eq!(items_of(item, &[]), levels(&[("", Level::Pii)]));
}

#[test]
fn a_structure_column_over_an_open_object_counts_the_keys_the_schema_does_not_name_as_pii() {
    let item = json!({"type": "object", "properties": {
        "customer": {"type": "object", "properties": {
            "name": {"type": "string", "x-sagat-classification": "internal"}}}}});
    assert_eq!(
        items_of(item, &["/customer", "/customer/name"]),
        levels(&[
            ("/customer", Level::Pii),
            ("/customer/name", Level::Internal)
        ])
    );
}

#[test]
fn a_schema_with_a_great_many_positions_is_read_in_bounded_time_and_as_pii() {
    // 16000 positions and 1024 mapped columns: a reading that tried every position for every column would not finish.
    let positions: Vec<Value> = (0..16000).map(|_| json!({})).collect();
    let schema = json!({"type": "object", "x-sagat-classification": "public",
        "properties": {"rows": {"type": "array", "prefixItems": positions,
            "items": {"type": "object"}}}});
    let fields: Vec<String> = (0..1024).map(|n| format!("/f{n}")).collect();
    let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
    let bytes = flow(Some(schema), step(json!({"inline": "input.rows"}), &refs));

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(levels_of(&bytes));
    });
    let found = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the levels of a schema with many positions must be computed in bounded time");
    assert_eq!(found.len(), 1024);
    assert!(
        found.iter().all(|(_, level)| *level >= Level::Pii),
        "{found:?}"
    );
}

#[test]
fn a_list_reached_through_an_unnamed_property_is_read_where_the_schema_describes_it() {
    // `rows` is not named: `additionalProperties` governs it, and that list describes its first position apart, as pii.
    let schema = json!({"type": "object", "x-sagat-classification": "public", "additionalProperties": {
        "type": "array",
        "prefixItems": [{"type": "object", "properties": {"id": {"type": "string", "x-sagat-classification": "pii"}}}],
        "items": {"type": "object", "properties": {"id": {"type": "string"}}}}});
    assert_eq!(
        mapped(&["/id"], Some(schema)),
        levels(&[("/id", Level::Pii)])
    );
}

#[test]
fn a_secret_below_an_unnamed_property_is_found_not_hidden_behind_the_pii_of_its_container() {
    let item = json!({"type": "object", "additionalProperties": {
        "type": "object", "x-sagat-classification": "internal",
        "properties": {"token": {"type": "string", "x-sagat-classification": "secret"}}}});
    assert_eq!(
        items_of(item, &["/customer"]),
        levels(&[("/customer", Level::Secret)])
    );
}

#[test]
fn a_schema_that_says_nothing_of_its_type_is_not_a_scalar() {
    let item = json!({"type": "object", "properties": {
        "untyped": {"x-sagat-classification": "internal"},
        "typed": {"type": "string", "x-sagat-classification": "internal"}}});
    assert_eq!(
        items_of(item, &["/untyped", "/typed"]),
        levels(&[("/untyped", Level::Pii), ("/typed", Level::Internal)])
    );
}

#[test]
fn a_type_that_is_a_list_is_a_scalar_only_when_every_type_in_it_is() {
    let item = json!({"type": "object", "properties": {
        "maybe": {"type": ["integer", "null"], "x-sagat-classification": "internal"},
        "either": {"type": ["string", "object"], "x-sagat-classification": "internal"}}});
    assert_eq!(
        items_of(item, &["/maybe", "/either"]),
        levels(&[("/maybe", Level::Internal), ("/either", Level::Pii)])
    );
}
